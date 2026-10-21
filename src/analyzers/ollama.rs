use crate::analyzer::{Analyzer, DataSource};
use crate::config::OllamaConfig;
use crate::price_feed::{self, ModelPrice};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

pub struct OllamaAnalyzer {
    usage_file: Option<std::path::PathBuf>,
    /// Electricity pricing from `[ollama]` in the config.
    power: OllamaConfig,
    /// Parsed usage log against its (size, mtime) (BZL-74). `usage.jsonl` is append-only and
    /// grows forever, so re-reading it on every reload gets steadily worse.
    ///
    /// Busy samples are cached beside the messages rather than recomputed: `record_busy` replaces
    /// the whole set and `capacity_in` sums across all of it, so handing it only the newly parsed
    /// tail would quietly understate throughput.
    cache: std::sync::Mutex<HashMap<std::path::PathBuf, CachedUsage>>,
}

struct CachedUsage {
    len: u64,
    modified: Option<std::time::SystemTime>,
    messages: std::sync::Arc<Vec<ConversationMessage>>,
    busy: std::sync::Arc<Vec<crate::plan_limits::BusySample>>,
}

impl OllamaAnalyzer {
    pub fn new() -> Self {
        Self {
            cache: Default::default(),
            power: crate::config::Config::load()
                .ok()
                .flatten()
                .unwrap_or_default()
                .ollama,
            usage_file: std::env::home_dir().map(|home| {
                home.join(".config")
                    .join("splitrail")
                    .join("ollama")
                    .join("usage.jsonl")
            }),
        }
    }
}

impl Default for OllamaAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct UsageEvent {
    ts: f64,
    model: String,
    #[allow(dead_code)] // Recorded by the proxy; not shown yet.
    endpoint: String,
    prompt_tokens: u64,
    cached_tokens: u64,
    completion_tokens: u64,
    /// Ollama's own `total_duration`; 0 for OpenAI-style calls.
    duration_ms: u64,
    /// Wall time through the proxy (absent from lines written before it was recorded).
    #[serde(default)]
    wall_ms: u64,
    /// SoC energy the proxy measured for this request (macmon); absent without a meter.
    #[serde(default)]
    energy_j: Option<f64>,
    client: String,
    /// The caller's last message and the start of the reply, cut short by the proxy.
    #[serde(default)]
    prompt_preview: String,
    #[serde(default)]
    reply_preview: String,
    /// Ollama's parameter count for the model (`20.9B`), for names whose tag has no size.
    #[serde(default)]
    param_size: Option<String>,
    /// Ollama's base family (`qwen3_5`), for custom or fine-tuned models priced by family.
    #[serde(default)]
    family: Option<String>,
    /// The calling process's working directory, and its name, found by the proxy from the
    /// connection (BZL-238); absent from older lines.
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    caller: Option<String>,
    /// The reply's own `id` (OpenAI-style `chatcmpl-…`), when it has one.
    #[serde(default)]
    response_id: Option<String>,
}

/// The project a request came from: the caller's folder (or process), else, on lines from
/// before the proxy found callers, its address.
fn project_of(event: &UsageEvent) -> String {
    crate::utils::caller_project(event.cwd.as_deref(), event.caller.as_deref())
        .unwrap_or_else(|| hash_text(&format!("ollama:{}", event.client)))
}

/// Normalise model name from the proxy event to the price feed slug.
/// If the model contains a namespace (e.g. `ollama/qwen3-coder:30b`), keep it.
fn normalize_model(name: &str) -> String {
    name.trim().to_string()
}

/// Price-feed names to try for an Ollama model, most specific first. Hosted providers name
/// open models `<family>-<size>`: `qwen3.8:27b-mlx` -> `qwen3.8-27b`; `gpt-oss:latest` with
/// 20.9B parameters -> `gpt-oss-20b`. The bare family is the last resort.
pub fn street_names(model: &str, param_size: Option<&str>) -> Vec<String> {
    let lower = model.trim().to_lowercase();
    let lower = lower.strip_prefix("library/").unwrap_or(&lower);
    let (base, tag) = lower.split_once(':').unwrap_or((lower, ""));
    let size_from_tag = tag.split(['-', '_']).next().filter(|t| {
        t.len() > 1
            && t.ends_with(['b', 'm'])
            && t[..t.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.')
    });
    let size = size_from_tag.map(str::to_string).or_else(|| {
        // `20.9B` -> `20b`: providers round parameter counts down.
        let count = param_size?.trim().to_lowercase();
        let (number, unit) = count.split_at(count.len().checked_sub(1)?);
        let number: f64 = number.parse().ok()?;
        Some(format!("{}{unit}", number.floor() as u64))
    });
    let mut names = Vec::new();
    if let Some(size) = size {
        names.push(format!("{base}-{size}"));
    }
    names.push(base.to_string());
    names
}

/// Street price for an Ollama model: a `[pricing.overrides]` entry under its Ollama name, else
/// the median across providers serving its hosted equivalent. Warns once when there is neither.
/// Models with no per-token hosted equivalent (by their base street name).
const LOCAL_ONLY_MODELS: &[&str] = &["moondream"];

/// Ollama's family as hosted providers spell it: `qwen3_5` and `qwen35` -> `qwen3.5`.
pub fn family_name(family: &str) -> String {
    let lower = family.trim().to_lowercase().replace('_', ".");
    let digits: Vec<char> = lower.chars().rev().take_while(char::is_ascii_digit).collect();
    if !lower.contains('.') && digits.len() == 2 {
        let (stem, number) = lower.split_at(lower.len() - 2);
        return format!("{stem}{}.{}", &number[..1], &number[1..]);
    }
    lower
}

/// Billions of parameters from Ollama's count: `752.39M` -> 0.75, `20.9B` -> 20.9.
fn billions(param_size: &str) -> Option<f64> {
    let count = param_size.trim().to_lowercase();
    let (number, unit) = count.split_at(count.len().checked_sub(1)?);
    let number: f64 = number.parse().ok()?;
    match unit {
        "b" => Some(number),
        "m" => Some(number / 1000.0),
        _ => None,
    }
}

/// For a model whose own name has no hosted price (a custom or fine-tuned one): the cheapest
/// way to buy its family from a host, the smallest hosted size at or above its own (the largest
/// below it if none is bigger). `sizes` is the family's hosted sizes in billions with their names.
pub fn family_fallback(sizes: &[(f64, String)], own: Option<f64>) -> Option<String> {
    let own = own.unwrap_or(0.0);
    let above = sizes.iter().filter(|(b, _)| *b >= own).min_by(|a, b| a.0.total_cmp(&b.0));
    let below = sizes.iter().filter(|(b, _)| *b < own).max_by(|a, b| a.0.total_cmp(&b.0));
    above.or(below).map(|(_, name)| name.clone())
}

fn street_price(model: &str, param_size: Option<&str>, family: Option<&str>) -> Option<ModelPrice> {
    if let Some((price, _)) = price_feed::lookup(model) {
        return Some(price);
    }
    let names = street_names(model, param_size);
    let found = price_feed::median_price(&names).map(|(_, price)| price).or_else(|| {
        let family = family_name(family?);
        let name = family_fallback(&price_feed::family_sizes(&family), param_size.and_then(billions))?;
        price_feed::median_price(&[name]).map(|(_, price)| price)
    });
    // No hosted service sells these per token, so there is no street price to find; their cost
    // is the electricity, which is tracked separately.
    let local_only = names
        .first()
        .is_some_and(|name| LOCAL_ONLY_MODELS.contains(&name.as_str()));
    if found.is_none() && !local_only {
        crate::utils::warn_once(format!(
            "No street price for Ollama model {model}; costed at $0 (set [pricing.overrides.\"{model}\"])."
        ));
    }
    found
}

/// Parse the JSONL written by `ollama_usage_proxy.py` into messages.
/// Each line is one request: the caller's message (when the proxy recorded it) and one assistant
/// message carrying the tokens. `cost` is the street price, what the same tokens cost from a
/// hosted provider, as for every other tool (their subscriptions are not per-token either); the
/// electricity actually used goes in `energy_wh` / `energy_cost`. Malformed lines are skipped.
pub fn parse_usage_log(
    content: &str,
    source: &Path,
    power: &OllamaConfig,
) -> Vec<ConversationMessage> {
    let source_str = source.to_string_lossy();
    let mut prices: HashMap<String, Option<ModelPrice>> = HashMap::new();
    // Lines written before the proxy recorded families borrow one from a later line.
    let families: HashMap<String, String> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageEvent>(line.trim()).ok())
        .filter_map(|event| Some((event.model, event.family?)))
        .collect();
    content
        .lines()
        .enumerate()
        .flat_map(|(line_no, line)| {
            let Ok(event) = serde_json::from_str::<UsageEvent>(line.trim()) else {
                return Vec::new();
            };
            let date = DateTime::<Utc>::from_timestamp_millis((event.ts * 1000.0) as i64)
                .unwrap_or_else(Utc::now);
            let model = normalize_model(&event.model);
            // Local inference costs its electricity. Ollama's own duration covers model load,
            // prompt processing and generation; /v1 calls lack it, so use the proxy's wall time.
            let millis = if event.duration_ms > 0 {
                event.duration_ms
            } else {
                event.wall_ms
            };
            let joules = event
                .energy_j
                .unwrap_or(power.power_watts * millis as f64 / 1000.0);
            // Input tokens = prompt - cached (cached are read, not billed as fresh input).
            let input_tokens = event.prompt_tokens.saturating_sub(event.cached_tokens);
            let output_tokens = event.completion_tokens;
            let cost = prices
                .entry(event.model.clone())
                .or_insert_with(|| {
                    let family = event.family.as_deref().or(families.get(&event.model).map(String::as_str));
                    street_price(&event.model, event.param_size.as_deref(), family)
                })
                .as_ref()
                .map_or(0.0, |price| {
                    price.input_cost(input_tokens)
                        + price.output_cost(output_tokens)
                        + price.cache_cost(0, event.cached_tokens)
                });
            let conversation_hash = hash_text(&format!("{}:{}:{}", source_str, line_no, event.ts));
            let project_hash = project_of(&event);
            let mut messages = Vec::with_capacity(2);
            if !event.prompt_preview.is_empty() {
                // Sent when the request started, which is wall time before it was logged.
                let sent = date - chrono::Duration::milliseconds(event.wall_ms as i64);
                messages.push(ConversationMessage {
                    application: Application::Ollama,
                    date: sent,
                    project_hash: project_hash.clone(),
                    conversation_hash: conversation_hash.clone(),
                    local_hash: None,
                    global_hash: hash_text(&format!("ollama_{}_{}_user", source_str, event.ts)),
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::User,
                    content: Some(event.prompt_preview.clone()),
                });
            }
            messages.push(ConversationMessage {
                application: Application::Ollama,
                date,
                project_hash,
                conversation_hash,
                // The reply's id, for matching it to an agent's record of the same request.
                local_hash: event.response_id.clone(),
                global_hash: hash_text(&format!("ollama_{}_{}", source_str, event.ts)),
                model: Some(model.clone()),
                stats: Stats {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: event.cached_tokens,
                    cached_tokens: event.cached_tokens,
                    cost,
                    energy_wh: joules / 3600.0,
                    energy_cost: power.joules_cost(joules),
                    ..Default::default()
                },
                role: MessageRole::Assistant,
                content: (!event.reply_preview.is_empty()).then(|| event.reply_preview.clone()),
            });
            messages
        })
        .collect()
}

/// How long each logged request kept the model busy, for the capacity rows (5h / Week).
pub fn busy_samples(content: &str) -> Vec<crate::plan_limits::BusySample> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageEvent>(line.trim()).ok())
        .map(|event| {
            let millis = if event.duration_ms > 0 {
                event.duration_ms
            } else {
                event.wall_ms
            };
            crate::plan_limits::BusySample {
                ended_at: event.ts,
                busy_secs: millis as f64 / 1000.0,
                output_tokens: event.completion_tokens,
            }
        })
        .collect()
}

#[async_trait]
impl Analyzer for OllamaAnalyzer {
    fn display_name(&self) -> &'static str {
        "Ollama"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        self.usage_file
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        Ok(self
            .usage_file
            .iter()
            .filter(|path| path.is_file())
            .map(|path| DataSource { path: path.clone() })
            .collect())
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let mut messages = Vec::new();
        let mut busy = Vec::new();
        for source in sources {
            let (len, modified) = std::fs::metadata(&source.path)
                .map(|m| (m.len(), m.modified().ok()))
                .unwrap_or((0, None));

            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let entry = match cache.get(&source.path) {
                Some(hit) if hit.len == len && hit.modified == modified => hit,
                _ => {
                    let content = std::fs::read_to_string(&source.path)?;
                    cache.insert(
                        source.path.clone(),
                        CachedUsage {
                            len,
                            modified,
                            messages: std::sync::Arc::new(parse_usage_log(
                                &content,
                                &source.path,
                                &self.power,
                            )),
                            busy: std::sync::Arc::new(busy_samples(&content)),
                        },
                    );
                    &cache[&source.path]
                }
            };
            messages.extend(entry.messages.iter().cloned());
            busy.extend(entry.busy.iter().cloned());
        }
        crate::plan_limits::record_busy(self.display_name(), busy);
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations = daily_stats.values().map(|s| s.conversations as u64).sum();
        Ok(AgenticCodingToolStats {
            analyzer_name: self.display_name().to_string(),
            daily_stats,
            messages,
            num_conversations,
        })
    }

    fn is_available(&self) -> bool {
        self.discover_data_sources()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_is_one_assistant_message() {
        let log = concat!(
            r#"{"ts": 1790880000.5, "model": "qwen3-coder:30b", "endpoint": "/api/generate", "prompt_tokens": 100, "cached_tokens": 20, "completion_tokens": 30, "duration_ms": 500, "client": "127.0.0.1:11434"}"#,
            "\n",
            "bad line\n",
            r#"{"ts": 1790880100, "model": "hermes3:8b", "endpoint": "/api/chat", "prompt_tokens": 50, "cached_tokens": 0, "completion_tokens": 10, "duration_ms": 200, "client": "127.0.0.1:11434"}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &OllamaConfig::default());
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.application, Application::Ollama);
        assert_eq!(first.role, MessageRole::Assistant);
        assert_eq!(first.model.as_deref(), Some("qwen3-coder:30b"));
        assert_eq!(first.stats.input_tokens, 80); // 100 - 20 cached
        assert_eq!(first.stats.output_tokens, 30);
        assert_eq!(first.stats.cache_read_tokens, 20);
        assert_eq!(first.stats.cost, 0.0);
        assert_ne!(first.conversation_hash, messages[1].conversation_hash);
    }

    #[test]
    fn an_embedding_request_counts_its_input_and_no_output() {
        let log = r#"{"ts": 1790880000, "model": "qwen3-embedding:0.6b", "endpoint": "/api/embed", "prompt_tokens": 37, "cached_tokens": 0, "completion_tokens": 0, "duration_ms": 41, "wall_ms": 45, "client": "127.0.0.1", "prompt_preview": "a beach (+1 more)", "reply_preview": ""}"#;
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &OllamaConfig::default());
        // The prompt preview becomes the user's side; the reply carries the counts.
        let reply = messages.iter().find(|m| m.role == MessageRole::Assistant).unwrap();
        assert_eq!(reply.model.as_deref(), Some("qwen3-embedding:0.6b"));
        assert_eq!((reply.stats.input_tokens, reply.stats.output_tokens), (37, 0));
    }

    #[test]
    fn a_request_is_filed_under_its_callers_folder() {
        let log = concat!(
            r#"{"ts": 1790880000, "model": "m", "endpoint": "/api/chat", "prompt_tokens": 1, "cached_tokens": 0, "completion_tokens": 1, "duration_ms": 1, "client": "127.0.0.1", "cwd": "/Users/me/Projects/mediasrc", "caller": "python3"}"#, "\n",
            r#"{"ts": 1790880001, "model": "m", "endpoint": "/v1/chat/completions", "prompt_tokens": 1, "cached_tokens": 0, "completion_tokens": 1, "duration_ms": 1, "client": "127.0.0.1", "cwd": "/", "caller": "opencode", "response_id": "chatcmpl-9"}"#, "\n",
        );
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &OllamaConfig::default());
        assert_eq!(messages[0].project_hash, "mediasrc");
        assert_eq!(messages[1].project_hash, "opencode");
        assert_eq!(messages[1].local_hash.as_deref(), Some("chatcmpl-9"));
    }

    #[test]
    fn energy_is_zero_until_power_is_configured() {
        let log = r#"{"ts": 1790880000, "model": "local-llama", "endpoint": "/api/generate", "prompt_tokens": 1, "cached_tokens": 0, "completion_tokens": 1, "duration_ms": 100, "client": "127.0.0.1"}"#;
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &OllamaConfig::default());
        assert_eq!(messages[0].stats.energy_cost, 0.0);
    }

    #[test]
    fn energy_cost_is_the_electricity_used() {
        let power = OllamaConfig {
            power_watts: 60.0,
            electricity_per_kwh: 0.24,
        };
        let log = concat!(
            // One hour of generation: 60 W -> 0.06 kWh -> $0.0144.
            r#"{"ts": 1790880000, "model": "qwen3.8:27b-mlx", "endpoint": "/api/chat", "prompt_tokens": 10, "cached_tokens": 0, "completion_tokens": 72000, "duration_ms": 3600000, "wall_ms": 3600100, "client": "127.0.0.1"}"#,
            "\n",
            // An OpenAI-style call has no Ollama duration; wall time is used instead.
            r#"{"ts": 1790880100, "model": "qwen3.8:27b-mlx", "endpoint": "/v1/chat/completions", "prompt_tokens": 10, "cached_tokens": 0, "completion_tokens": 5, "duration_ms": 0, "wall_ms": 1800000, "client": "127.0.0.1"}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &power);
        assert!((messages[0].stats.energy_cost - 0.0144).abs() < 1e-12);
        assert!((messages[0].stats.energy_wh - 60.0).abs() < 1e-9);
        assert!((messages[1].stats.energy_cost - 0.0072).abs() < 1e-12);
    }

    #[test]
    fn previews_become_the_transcript() {
        let log = r#"{"ts": 1790880010, "model": "qwen3.8:27b-mlx", "endpoint": "/api/chat", "prompt_tokens": 9, "cached_tokens": 0, "completion_tokens": 4, "duration_ms": 2000, "wall_ms": 2000, "client": "127.0.0.1", "prompt_preview": "why is the sky blue", "reply_preview": "Rayleigh scattering"}"#;
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &OllamaConfig::default());
        assert_eq!(messages.len(), 2);
        let (user, reply) = (&messages[0], &messages[1]);
        assert_eq!(user.role, MessageRole::User);
        assert_eq!(user.content.as_deref(), Some("why is the sky blue"));
        assert_eq!(user.model, None);
        assert_eq!(user.date.timestamp(), 1_790_880_008);
        assert_eq!(reply.content.as_deref(), Some("Rayleigh scattering"));
        assert_eq!(reply.stats.output_tokens, 4);
        assert_eq!(user.conversation_hash, reply.conversation_hash);
        assert_ne!(user.global_hash, reply.global_hash);
    }

    #[test]
    fn measured_energy_beats_the_configured_wattage() {
        let power = OllamaConfig {
            power_watts: 60.0,
            electricity_per_kwh: 0.24,
        };
        // 36 kJ measured = 0.01 kWh = $0.0024, whatever 60 W x 1 h would say.
        let log = r#"{"ts": 1790880000, "model": "m", "endpoint": "/api/chat", "prompt_tokens": 1, "cached_tokens": 0, "completion_tokens": 1, "duration_ms": 3600000, "client": "127.0.0.1", "energy_j": 36000.0}"#;
        let messages = parse_usage_log(log, Path::new("/test.jsonl"), &power);
        assert!((messages[0].stats.energy_cost - 0.0024).abs() < 1e-12);
        assert!((messages[0].stats.energy_wh - 10.0).abs() < 1e-9);
    }

    #[test]
    fn ollama_names_map_to_hosted_names() {
        assert_eq!(
            street_names("qwen3.8:27b-mlx", None),
            ["qwen3.8-27b", "qwen3.8"]
        );
        assert_eq!(
            street_names("gpt-oss:latest", Some("20.9B")),
            ["gpt-oss-20b", "gpt-oss"]
        );
        assert_eq!(
            street_names("qwen3-coder:latest", Some("30.5B")),
            ["qwen3-coder-30b", "qwen3-coder"]
        );
        assert_eq!(
            street_names("gemma3:27b", Some("27.4B")),
            ["gemma3-27b", "gemma3"]
        );
        assert_eq!(street_names("moondream:latest", None), ["moondream"]);
        assert_eq!(street_names("smollm:135m", None), ["smollm-135m", "smollm"]);
    }

    #[test]
    fn custom_models_are_priced_by_their_family() {
        assert_eq!(family_name("qwen3_5"), "qwen3.5");
        assert_eq!(family_name("qwen35"), "qwen3.5");
        assert_eq!(family_name("llama"), "llama");
        assert_eq!(family_name("gemma3"), "gemma3");
        assert_eq!(billions("752.39M"), Some(0.75239));
        let sizes = [(9.0, "qwen3.5-9b".to_string()), (27.0, "qwen3.5-27b".to_string()), (397.0, "qwen3.5-397b".to_string())];
        // A 0.8B fine-tune: the smallest hosted size above it.
        assert_eq!(family_fallback(&sizes, Some(0.75)).as_deref(), Some("qwen3.5-9b"));
        assert_eq!(family_fallback(&sizes, Some(20.0)).as_deref(), Some("qwen3.5-27b"));
        // Bigger than anything hosted: the largest.
        assert_eq!(family_fallback(&sizes, Some(500.0)).as_deref(), Some("qwen3.5-397b"));
        assert_eq!(family_fallback(&[], Some(1.0)), None);
    }

    /// Prices a decision model against the real feed: `cargo test ollama_family_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ollama_family_live_prices_a_fine_tune() {
        crate::price_feed::init(Default::default(), true);
        let price = street_price("tev1:0.8b-mxfp8", Some("752.39M"), Some("qwen3_5")).unwrap();
        println!("tev1 at ${:.3} in / ${:.3} out per 1M", price.input_per_1m, price.output_per_1m);
    }
}

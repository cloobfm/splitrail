//! MTPLX, a local MLX model server (`mtplx serve` / `mtplx start pi`), OpenAI-compatible, one
//! port per model. It appends one line per request to
//! `~/.mtplx/logs/request-log-<port>.jsonl` (rotated to `….jsonl.1` and so on):
//!
//! ```json
//! {"logged_at_s": 1792474425.4, "request_id": "chatcmpl-e82e…", "session_id": "anon-1b8f…",
//!  "served_model_id": "qwen-fast", "prompt_tokens": 2272, "cached_tokens": 2202,
//!  "completion_tokens": 82, "request_elapsed_s": 4.77, "decode_tok_s": 21.3, …}
//! ```
//!
//! The server writes it, so every client is covered (Pi, OpenCode, the web UI, scripts).
//! Warm-up requests carry no model and are credited to the file's. A benchmark run inside mtplx
//! rather than through the server leaves no line here.
//!
//! The log has no text and no caller. When splitrail's usage proxy (`scripts/mtplx_proxy.sh`)
//! sits in front of a port, its lines in `~/.config/splitrail/mtplx/usage-<port>.jsonl` carry the
//! prompt and reply previews, the calling project and measured energy, and the reply's `id`,
//! which is mtplx's `request_id`; they are merged in by that id (BZL-238).
//!
//! Costed like Ollama: `cost` is the street price, what the same tokens cost from a hosted
//! provider of the same model, and the electricity goes in `energy_wh` / `energy_cost` (measured
//! by the proxy, else the request's wall time at `[ollama] power_watts`). The log names only the
//! server's alias (`qwen-fast`), so the model is found through Pi's `~/.pi/agent/models.json`,
//! which names each alias ("Qwen 3.6 35B-A3B (local, fast)" → `qwen3.6-35b-a3b`). A
//! `[pricing.overrides."qwen-fast"]` entry wins over both.
//!
//! Each request is counted in one place. Pi's own sessions name the request (`responseId` is
//! mtplx's `request_id`), and Pi has the transcript and the project, so Pi's requests belong to
//! the Pi Agent column (priced here, through [`LocalPricing`]); this column keeps the rest.

use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::analyzer::{Analyzer, DataSource};
use crate::config::OllamaConfig;
use crate::price_feed::ModelPrice;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub struct MtplxAnalyzer {
    /// Logs are appended to and small (a few KB a request); a whole-file cache is enough.
    cache: crate::stamp_cache::StampCache,
    /// Wattage and electricity price for local inference, shared with Ollama.
    power: OllamaConfig,
    /// Alias -> the model Pi says it is, for pricing.
    names: HashMap<String, String>,
}

impl MtplxAnalyzer {
    pub fn new() -> Self {
        Self {
            cache: crate::stamp_cache::StampCache::new(),
            power: crate::config::Config::load().ok().flatten().unwrap_or_default().ollama,
            names: read_pi_models().map(|text| pi_model_names(&text)).unwrap_or_default(),
        }
    }

    fn logs_dir() -> Option<PathBuf> {
        std::env::home_dir().map(|home| home.join(".mtplx/logs"))
    }
}

/// Where the usage proxy in front of an mtplx port writes (`usage-<port>.jsonl`).
fn proxy_dir() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".config/splitrail/mtplx"))
}

/// The port in `request-log-8000.jsonl` or `request-log-8000.jsonl.1`.
fn port_of(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name.trim_start_matches("request-log-").split('.').next().unwrap_or_default().to_string()
}

fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn read_pi_models() -> Option<String> {
    std::fs::read_to_string(std::env::home_dir()?.join(".pi/agent/models.json")).ok()
}

/// Alias -> model name, from Pi's `models.json` (`"models": [{"id": "qwen-fast", "name":
/// "Qwen 3.6 35B-A3B (local, fast)"}]`). Only those two fields are read.
fn pi_model_names(models_json: &str) -> HashMap<String, String> {
    let Ok(config) = serde_json::from_str::<Value>(models_json) else { return HashMap::new() };
    let mut names = HashMap::new();
    let providers = config.get("providers").and_then(Value::as_object).into_iter().flat_map(|p| p.values());
    for model in providers.flat_map(|p| p.get("models").and_then(Value::as_array).into_iter().flatten()) {
        if let (Some(id), Some(name)) = (text(model, "id"), text(model, "name")) {
            names.insert(id, name);
        }
    }
    names
}

/// A price-list slug from a display name: "Qwen 3.6 35B-A3B (local, fast)" → `qwen3.6-35b-a3b`,
/// "Qwen 3.8 27B Q8" → `qwen3.8-27b`. Quantisation and format words are dropped: a hosted
/// provider serves the model, not this build of it.
fn slug(name: &str) -> String {
    let name = name.split('(').next().unwrap_or(name).to_lowercase();
    let quant = |w: &str| {
        matches!(w, "fp16" | "bf16" | "fp8" | "int4" | "int8" | "mlx" | "gguf" | "mtplx")
            || (w.starts_with('q') && w[1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
            || w.ends_with("-bit")
    };
    let words: Vec<&str> = name.split_whitespace().filter(|w| !quant(w)).collect();
    match words.as_slice() {
        // "qwen 3.6 …": the version joins the family.
        [family, version, rest @ ..]
            if version.chars().next().is_some_and(|c| c.is_ascii_digit()) && version.contains('.') =>
        {
            std::iter::once(format!("{family}{version}"))
                .chain(rest.iter().map(|w| w.to_string()))
                .collect::<Vec<_>>()
                .join("-")
        }
        _ => words.join("-"),
    }
}

/// The street price for a served model: an override or feed entry under its alias, else the
/// median hosted price of the model Pi names for it. None when neither is sold anywhere.
fn street_price(alias: &str, names: &HashMap<String, String>) -> Option<ModelPrice> {
    if let Some((price, _)) = crate::price_feed::lookup(alias) {
        return Some(price);
    }
    // The median across the providers that host it, as for Ollama's models.
    let name = slug(names.get(alias)?);
    crate::price_feed::median_price(&[name]).map(|(_, price)| price)
}

/// What the usage proxy saw of one request.
#[derive(Debug, Clone, Default, PartialEq)]
struct ProxyRecord {
    prompt: String,
    reply: String,
    project: Option<String>,
    energy_j: Option<f64>,
    wall_ms: u64,
}

/// The proxy's records, by the reply's id.
fn proxy_records(dir: &Path) -> HashMap<String, ProxyRecord> {
    let mut records = HashMap::new();
    for path in glob::glob(&format!("{}/usage-*.jsonl", dir.display())).into_iter().flatten().flatten() {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for entry in content.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            let Some(id) = text(&entry, "response_id") else { continue };
            records.insert(
                id,
                ProxyRecord {
                    prompt: text(&entry, "prompt_preview").unwrap_or_default(),
                    reply: text(&entry, "reply_preview").unwrap_or_default(),
                    project: crate::utils::caller_project(
                        entry.get("cwd").and_then(Value::as_str),
                        entry.get("caller").and_then(Value::as_str),
                    ),
                    energy_j: entry.get("energy_j").and_then(Value::as_f64),
                    wall_ms: entry.get("wall_ms").and_then(Value::as_u64).unwrap_or(0),
                },
            );
        }
    }
    records
}

/// Fill in what the proxy saw: the project, the reply's text, measured energy, and the prompt
/// as the user's side, sent when the request started.
fn merge_proxy(messages: Vec<ConversationMessage>, records: &HashMap<String, ProxyRecord>, power: &OllamaConfig) -> Vec<ConversationMessage> {
    let mut out = Vec::with_capacity(messages.len());
    for mut reply in messages {
        let Some(record) = reply.local_hash.as_ref().and_then(|id| records.get(id)) else {
            out.push(reply);
            continue;
        };
        if let Some(project) = &record.project {
            reply.project_hash = project.clone();
        }
        if !record.reply.is_empty() {
            reply.content = Some(record.reply.clone());
        }
        if let Some(joules) = record.energy_j {
            reply.stats.energy_wh = joules / 3600.0;
            reply.stats.energy_cost = power.joules_cost(joules);
        }
        if !record.prompt.is_empty() {
            out.push(ConversationMessage {
                date: reply.date - chrono::Duration::milliseconds(record.wall_ms as i64),
                local_hash: None,
                global_hash: hash_text(&format!("{}-prompt", reply.global_hash)),
                model: None,
                stats: Stats::default(),
                role: MessageRole::User,
                content: Some(record.prompt.clone()),
                ..reply.clone()
            });
        }
        out.push(reply);
    }
    out
}

/// Every request's wall time, by `request_id`, across the request logs: Pi's electricity.
fn request_seconds() -> HashMap<String, f64> {
    let Some(dir) = MtplxAnalyzer::logs_dir() else { return HashMap::new() };
    let mut seconds = HashMap::new();
    for path in glob::glob(&format!("{}/request-log-*.jsonl*", dir.display())).into_iter().flatten().flatten() {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for entry in content.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            if let (Some(id), Some(s)) = (text(&entry, "request_id"), entry.get("request_elapsed_s").and_then(Value::as_f64)) {
                seconds.insert(id, s);
            }
        }
    }
    seconds
}

/// The requests Pi's sessions already count: `responseId`s of replies from `mtplx*` providers.
fn pi_request_ids() -> HashSet<String> {
    let Some(home) = std::env::home_dir() else { return Default::default() };
    let pattern = format!("{}/.pi/agent/sessions/*/*.jsonl", home.display());
    let mut ids = HashSet::new();
    for path in glob::glob(&pattern).into_iter().flatten().flatten() {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for entry in content.lines().filter(|l| l.contains("\"responseId\"")).filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            let message = entry.get("message");
            let local = message
                .and_then(|m| m.get("provider"))
                .and_then(Value::as_str)
                .is_some_and(LocalPricing::is_local);
            if let (true, Some(id)) = (local, message.and_then(|m| m.get("responseId")).and_then(Value::as_str)) {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

/// How Pi costs a reply it got from an MTPLX model: the street price of the model behind the
/// alias, and the electricity of the request's wall time.
pub(crate) struct LocalPricing {
    power: OllamaConfig,
    names: HashMap<String, String>,
    seconds: HashMap<String, f64>,
    prices: std::sync::Mutex<HashMap<String, Option<ModelPrice>>>,
}

impl LocalPricing {
    pub(crate) fn load() -> Self {
        Self {
            power: crate::config::Config::load().ok().flatten().unwrap_or_default().ollama,
            names: read_pi_models().map(|text| pi_model_names(&text)).unwrap_or_default(),
            seconds: request_seconds(),
            prices: Default::default(),
        }
    }

    /// No prices, names or timings: for tests.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self { power: OllamaConfig::default(), names: HashMap::new(), seconds: HashMap::new(), prices: Default::default() }
    }

    pub(crate) fn is_local(provider: &str) -> bool {
        provider.starts_with("mtplx")
    }

    fn street_price(&self, model: &str) -> Option<ModelPrice> {
        self.prices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(model.to_string())
            .or_insert_with(|| street_price(model, &self.names))
            .clone()
    }

    /// (cost, energy Wh, energy cost) for a reply.
    pub(crate) fn price(&self, model: &str, response_id: Option<&str>, stats: &Stats) -> (f64, f64, f64) {
        let cost = self.street_price(model).map_or(0.0, |p| {
            p.input_cost(stats.input_tokens) + p.output_cost(stats.output_tokens) + p.cache_cost(0, stats.cache_read_tokens)
        });
        let joules = self.power.power_watts * response_id.and_then(|id| self.seconds.get(id)).copied().unwrap_or(0.0);
        (cost, joules / 3600.0, self.power.joules_cost(joules))
    }
}

/// One assistant message per request in one log file, costed at `price` (by model) and
/// `power`.
pub(crate) fn parse_log(
    path: &Path,
    power: &OllamaConfig,
    price: &dyn Fn(&str) -> Option<ModelPrice>,
) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(path)?;
    let port = port_of(path);
    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    // Warm-ups name no model: they ran on the file's, which later lines name.
    let file_model = entries
        .iter()
        .find_map(|e| text(e, "served_model_id").or_else(|| text(e, "request_model")))
        .unwrap_or_else(|| format!("mtplx-{port}"));
    let mut messages = Vec::new();
    for (line, entry) in entries.iter().enumerate() {
        let Some(at) = entry.get("logged_at_s").and_then(Value::as_f64) else { continue };
        let Some(date) = DateTime::from_timestamp_millis((at * 1000.0) as i64) else { continue };
        let field = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
        let (prompt, cached, completion) = (field("prompt_tokens"), field("cached_tokens"), field("completion_tokens"));
        if prompt + completion == 0 {
            continue;
        }
        let model = text(entry, "served_model_id")
            .or_else(|| text(entry, "request_model"))
            .unwrap_or_else(|| file_model.clone());
        let input = prompt.saturating_sub(cached);
        let cost = price(&model).map_or(0.0, |p| p.input_cost(input) + p.output_cost(completion) + p.cache_cost(0, cached));
        // The request's wall time at the configured draw, unless the proxy measured it.
        let joules = power.power_watts * entry.get("request_elapsed_s").and_then(Value::as_f64).unwrap_or(0.0);
        let id = text(entry, "request_id").unwrap_or_else(|| format!("{port}-{line}-{at}"));
        let session = text(entry, "session_id").unwrap_or_else(|| id.clone());
        messages.push(ConversationMessage {
            application: Application::Mtplx,
            date,
            project_hash: String::new(),
            conversation_hash: hash_text(&format!("mtplx-{port}-{session}")),
            local_hash: Some(id.clone()),
            global_hash: hash_text(&format!("mtplx-{port}-{id}-{at}")),
            model: Some(model),
            stats: Stats {
                input_tokens: input,
                cache_read_tokens: cached,
                cached_tokens: cached,
                output_tokens: completion,
                cost,
                energy_wh: joules / 3600.0,
                energy_cost: power.joules_cost(joules),
                ..Default::default()
            },
            role: MessageRole::Assistant,
            content: None,
        });
    }
    Ok(messages)
}

#[async_trait]
impl Analyzer for MtplxAnalyzer {
    fn display_name(&self) -> &'static str {
        "MTPLX"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        Self::logs_dir()
            .map(|dir| vec![format!("{}/request-log-*.jsonl*", dir.display())])
            .unwrap_or_default()
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let mut sources = Vec::new();
        for pattern in self.get_data_glob_patterns() {
            for path in glob::glob(&pattern)?.flatten().filter(|p| p.is_file()) {
                sources.push(DataSource { path });
            }
        }
        Ok(sources)
    }

    async fn parse_conversations(&self, sources: Vec<DataSource>) -> Result<Vec<ConversationMessage>> {
        // One lookup per model, shared by the files parsed in parallel.
        let prices: std::sync::Mutex<HashMap<String, Option<ModelPrice>>> = Default::default();
        let price = |model: &str| {
            let mut prices = prices.lock().unwrap_or_else(|e| e.into_inner());
            prices.entry(model.to_string()).or_insert_with(|| street_price(model, &self.names)).clone()
        };
        let (mut messages, _report) = self.cache.parse_all(&sources, |path| parse_log(path, &self.power, &price));
        // Pi's requests are counted, with their transcript, in the Pi Agent column.
        let pi = pi_request_ids();
        messages.retain(|m| m.local_hash.as_ref().is_none_or(|id| !pi.contains(id)));
        let records = proxy_dir().map(|dir| proxy_records(&dir)).unwrap_or_default();
        Ok(merge_proxy(messages, &records, &self.power))
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let messages = self.parse_conversations(self.discover_data_sources()?).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations =
            messages.iter().map(|m| &m.conversation_hash).collect::<HashSet<_>>().len() as u64;
        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        Self::logs_dir().is_some_and(|d| d.is_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<ModelPrice> {
        None
    }

    #[test]
    fn each_request_is_one_reply_and_warmups_take_the_files_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("request-log-8000.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"logged_at_s": 1792474346.07, "prompt_tokens": 6, "cached_tokens": 0, "completion_tokens": 16}"#, "\n",
                r#"{"logged_at_s": 1792474416.62, "request_id": "chatcmpl-a", "session_id": "anon-1", "served_model_id": "qwen-fast", "prompt_tokens": 2085, "cached_tokens": 2003, "completion_tokens": 91}"#, "\n",
                r#"{"logged_at_s": 1792474420.57, "request_id": "chatcmpl-b", "session_id": "anon-1", "served_model_id": "qwen-fast", "prompt_tokens": 2202, "cached_tokens": 2085, "completion_tokens": 54}"#, "\n",
                "not json\n",
            ),
        )
        .unwrap();
        let messages = parse_log(&path, &OllamaConfig::default(), &none).unwrap();
        assert_eq!(messages.len(), 3);
        assert!(messages.iter().all(|m| m.model.as_deref() == Some("qwen-fast") && m.application == Application::Mtplx));
        let b = &messages[2];
        assert_eq!((b.stats.input_tokens, b.stats.cache_read_tokens, b.stats.output_tokens, b.stats.cost), (117, 2085, 54, 0.0));
        // One conversation per session; the warm-up is its own.
        assert_eq!(messages[1].conversation_hash, messages[2].conversation_hash);
        assert_ne!(messages[0].conversation_hash, messages[1].conversation_hash);
        assert_eq!(port_of(Path::new("/x/request-log-8001.jsonl.2")), "8001");
    }

    #[test]
    fn requests_cost_the_street_price_and_their_electricity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("request-log-8000.jsonl");
        std::fs::write(
            &path,
            r#"{"logged_at_s": 1792474420.57, "request_id": "r", "session_id": "s", "served_model_id": "qwen-fast", "prompt_tokens": 1000000, "cached_tokens": 0, "completion_tokens": 1000000, "request_elapsed_s": 3600}"#,
        )
        .unwrap();
        let price = |model: &str| {
            assert_eq!(model, "qwen-fast");
            Some(ModelPrice { input_per_1m: 0.25, output_per_1m: 1.5, cache_write_per_1m: None, cache_read_per_1m: None, tiers: Vec::new() })
        };
        let power = OllamaConfig { power_watts: 20.0, electricity_per_kwh: 0.30 };
        let reply = &parse_log(&path, &power, &price).unwrap()[0];
        assert!((reply.stats.cost - 1.75).abs() < 1e-9);
        // 20 W for an hour: 20 Wh, $0.006 at $0.30/kWh.
        assert!((reply.stats.energy_wh - 20.0).abs() < 1e-9);
        assert!((reply.stats.energy_cost - 0.006).abs() < 1e-9);
    }

    #[test]
    fn pi_names_the_model_behind_each_alias_for_pricing() {
        let names = pi_model_names(
            r#"{"providers": {"mtplx-fast": {"baseUrl": "http://127.0.0.1:8000/v1", "apiKey": "x", "models": [{"id": "qwen-fast", "name": "Qwen 3.6 35B-A3B (local, fast)"}]},
                              "mtplx-code": {"baseUrl": "http://127.0.0.1:8002/v1", "models": [{"id": "qwen-code", "name": "Qwen 3.8 27B 4-bit (local, everyday)"}]}}}"#,
        );
        assert_eq!(slug(&names["qwen-fast"]), "qwen3.6-35b-a3b");
        assert_eq!(slug(&names["qwen-code"]), "qwen3.8-27b");
        assert_eq!(slug("Qwen 3.8 27B Q8 (local, deep)"), "qwen3.8-27b");
    }

    #[test]
    fn the_proxys_text_project_and_energy_are_merged_by_request_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("usage-8000.jsonl"),
            r#"{"ts": 1792474420.5, "model": "qwen-fast", "endpoint": "/v1/chat/completions", "prompt_tokens": 2202, "cached_tokens": 2085, "completion_tokens": 54, "wall_ms": 3900, "energy_j": 360.0, "response_id": "chatcmpl-b", "cwd": "/Users/me/Projects/mediasrc", "caller": "python3", "prompt_preview": "caption this", "reply_preview": "A beach at dusk"}"#,
        )
        .unwrap();
        let log = dir.path().join("request-log-8000.jsonl");
        std::fs::write(
            &log,
            r#"{"logged_at_s": 1792474420.57, "request_id": "chatcmpl-b", "session_id": "s", "served_model_id": "qwen-fast", "prompt_tokens": 2202, "cached_tokens": 2085, "completion_tokens": 54}"#,
        )
        .unwrap();
        let power = OllamaConfig { power_watts: 15.0, electricity_per_kwh: 0.30 };
        let merged = merge_proxy(parse_log(&log, &power, &none).unwrap(), &proxy_records(dir.path()), &power);
        assert_eq!(merged.len(), 2);
        let (prompt, reply) = (&merged[0], &merged[1]);
        assert_eq!((prompt.role.clone(), prompt.content.as_deref()), (MessageRole::User, Some("caption this")));
        assert_eq!(reply.content.as_deref(), Some("A beach at dusk"));
        assert_eq!((prompt.project_hash.as_str(), reply.project_hash.as_str()), ("mediasrc", "mediasrc"));
        // Measured: 360 J is 0.1 Wh.
        assert!((reply.stats.energy_wh - 0.1).abs() < 1e-9);
        assert_eq!(reply.stats.output_tokens, 54);
    }
}

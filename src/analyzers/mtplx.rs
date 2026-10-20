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
//! The server writes it, so every client is covered (Pi, OpenCode, the web UI, the terminal);
//! Pi itself saves no sessions in this setup. Warm-up requests carry no model and are credited
//! to the port's. A benchmark run inside mtplx rather than through the server leaves no line
//! here.
//!
//! Costed like Ollama: `cost` is the street price, what the same tokens cost from a hosted
//! provider of the same model, and the electricity goes in `energy_wh` / `energy_cost` (the
//! request's wall time at `[ollama] power_watts`, priced at `electricity_per_kwh`). The log
//! names only the server's alias (`qwen-fast`), so the model is found through Pi's
//! `~/.pi/agent/models.json`, whose provider for the port names it ("Qwen 3.6 35B-A3B (local,
//! fast)" → `qwen3.6-35b-a3b`). A `[pricing.overrides."qwen-fast"]` entry wins over both.

use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde_json::Value;
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::config::OllamaConfig;
use crate::price_feed::ModelPrice;
use std::collections::HashMap;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub struct MtplxAnalyzer {
    /// Logs are appended to and small (a few KB a request); a whole-file cache is enough.
    cache: crate::stamp_cache::StampCache,
    /// Wattage and electricity price for local inference, shared with Ollama.
    power: OllamaConfig,
    /// Port -> the model Pi says it serves, for pricing.
    names: HashMap<String, String>,
}

impl MtplxAnalyzer {
    pub fn new() -> Self {
        Self {
            cache: crate::stamp_cache::StampCache::new(),
            power: crate::config::Config::load().ok().flatten().unwrap_or_default().ollama,
            names: std::env::home_dir()
                .and_then(|home| std::fs::read_to_string(home.join(".pi/agent/models.json")).ok())
                .map(|text| pi_model_names(&text))
                .unwrap_or_default(),
        }
    }

    fn logs_dir() -> Option<std::path::PathBuf> {
        std::env::home_dir().map(|home| home.join(".mtplx/logs"))
    }
}

/// The port in `request-log-8000.jsonl` or `request-log-8000.jsonl.1`.
fn port_of(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name.trim_start_matches("request-log-").split('.').next().unwrap_or_default().to_string()
}

/// Port -> model name, from Pi's `models.json` providers (`"baseUrl":
/// "http://127.0.0.1:8000/v1"`, `"models": [{"name": "Qwen 3.6 35B-A3B (local, fast)"}]`).
/// Only those two fields are read.
fn pi_model_names(text: &str) -> HashMap<String, String> {
    let Ok(config) = serde_json::from_str::<Value>(text) else { return HashMap::new() };
    let mut names = HashMap::new();
    for provider in config.get("providers").and_then(Value::as_object).into_iter().flat_map(|p| p.values()) {
        let Some(port) = provider
            .get("baseUrl")
            .and_then(Value::as_str)
            .and_then(|url| url.rsplit_once(':'))
            .map(|(_, rest)| rest.split('/').next().unwrap_or_default().to_string())
        else {
            continue;
        };
        let name = provider.pointer("/models/0/name").and_then(Value::as_str);
        if let Some(name) = name {
            names.insert(port, name.to_string());
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
        [family, version, rest @ ..] if version.chars().next().is_some_and(|c| c.is_ascii_digit()) && version.contains('.') => {
            std::iter::once(format!("{family}{version}")).chain(rest.iter().map(|w| w.to_string())).collect::<Vec<_>>().join("-")
        }
        _ => words.join("-"),
    }
}

/// The street price for a served model: an override or feed entry under its alias, else the
/// median hosted price of the model Pi names for its port. None when neither is sold anywhere.
fn street_price(alias: &str, port: &str, names: &HashMap<String, String>) -> Option<ModelPrice> {
    if let Some((price, _)) = crate::price_feed::lookup(alias) {
        return Some(price);
    }
    // The median across the providers that host it, as for Ollama's models.
    let name = slug(names.get(port)?);
    crate::price_feed::median_price(&[name]).map(|(_, price)| price)
}

fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// One assistant message per request in one log file, costed at `price` (by model) and
/// `power`.
pub(crate) fn parse_log(
    path: &Path,
    power: &OllamaConfig,
    price: &dyn Fn(&str, &str) -> Option<ModelPrice>,
) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(path)?;
    let port = port_of(path);
    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    // Warm-ups name no model: they ran on the port's, which later lines name.
    let port_model = entries
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
        let model = text(entry, "served_model_id").or_else(|| text(entry, "request_model")).unwrap_or_else(|| port_model.clone());
        let input = prompt.saturating_sub(cached);
        let cost = price(&model, &port).map_or(0.0, |p| {
            p.input_cost(input) + p.output_cost(completion) + p.cache_cost(0, cached)
        });
        // No meter here: the request's wall time at the configured draw.
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
        let price = |model: &str, port: &str| {
            let mut prices = prices.lock().unwrap_or_else(|e| e.into_inner());
            prices.entry(model.to_string()).or_insert_with(|| street_price(model, port, &self.names)).clone()
        };
        let (messages, _report) = self.cache.parse_all(&sources, |path| parse_log(path, &self.power, &price));
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let messages = self.parse_conversations(self.discover_data_sources()?).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations =
            messages.iter().map(|m| &m.conversation_hash).collect::<std::collections::HashSet<_>>().len() as u64;
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

    #[test]
    fn each_request_is_one_reply_and_warmups_take_the_ports_model() {
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
        let none = |_: &str, _: &str| None;
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
        let price = |model: &str, port: &str| {
            assert_eq!((model, port), ("qwen-fast", "8000"));
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
    fn pi_names_the_model_behind_each_port_for_pricing() {
        let names = pi_model_names(
            r#"{"providers": {"mtplx-fast": {"baseUrl": "http://127.0.0.1:8000/v1", "apiKey": "x", "models": [{"id": "qwen-fast", "name": "Qwen 3.6 35B-A3B (local, fast)"}]},
                              "mtplx-code": {"baseUrl": "http://127.0.0.1:8002/v1", "models": [{"id": "qwen-code", "name": "Qwen 3.8 27B 4-bit (local, everyday)"}]}}}"#,
        );
        assert_eq!(slug(&names["8000"]), "qwen3.6-35b-a3b");
        assert_eq!(slug(&names["8002"]), "qwen3.8-27b");
        assert_eq!(slug("Qwen 3.8 27B Q8 (local, deep)"), "qwen3.8-27b");
    }
}

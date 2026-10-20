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
//! to the port's. Local, so the cost is $0. A benchmark run inside mtplx rather than through
//! the server leaves no line here.

use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde_json::Value;
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub struct MtplxAnalyzer {
    /// Logs are appended to and small (a few KB a request); a whole-file cache is enough.
    cache: crate::stamp_cache::StampCache,
}

impl MtplxAnalyzer {
    pub fn new() -> Self {
        Self { cache: crate::stamp_cache::StampCache::new() }
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

fn text(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// One assistant message per request in one log file.
pub(crate) fn parse_log(path: &Path) -> Result<Vec<ConversationMessage>> {
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
                input_tokens: prompt.saturating_sub(cached),
                cache_read_tokens: cached,
                cached_tokens: cached,
                output_tokens: completion,
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
        let (messages, _report) = self.cache.parse_all(&sources, parse_log);
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
        let messages = parse_log(&path).unwrap();
        assert_eq!(messages.len(), 3);
        assert!(messages.iter().all(|m| m.model.as_deref() == Some("qwen-fast") && m.application == Application::Mtplx));
        let b = &messages[2];
        assert_eq!((b.stats.input_tokens, b.stats.cache_read_tokens, b.stats.output_tokens, b.stats.cost), (117, 2085, 54, 0.0));
        // One conversation per session; the warm-up is its own.
        assert_eq!(messages[1].conversation_hash, messages[2].conversation_hash);
        assert_ne!(messages[0].conversation_hash, messages[1].conversation_hash);
        assert_eq!(port_of(Path::new("/x/request-log-8001.jsonl.2")), "8001");
    }
}

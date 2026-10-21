//! Pi Agent (`pi`, upstream #65): one JSONL per session under `~/.pi/agent/sessions/<cwd>/`.
//!
//! ```json
//! {"type":"session","version":3,"id":"019d…","timestamp":"…","cwd":"/Users/me/proj"}
//! {"type":"message","id":"c39e2a6a","timestamp":"…","message":{"role":"user","content":[…]}}
//! {"type":"message","id":"…","message":{"role":"assistant","model":"gpt-5.5","provider":"openai-codex",
//!   "content":[{"type":"thinking"},{"type":"toolCall"}],"stopReason":"toolUse",
//!   "usage":{"input":1069,"output":77,"cacheRead":0,"cacheWrite":0,"cost":{"total":0.007655}}}}
//! ```
//!
//! Pi's own `usage.cost.total` is used when it is set; otherwise the cost comes from our prices.
//! A reply that failed before any tokens (`stopReason: error`, all zero) is not counted.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub struct PiAgentAnalyzer {
    /// Sessions are appended to, but small and few; a whole-file cache is enough (BZL-74).
    cache: crate::stamp_cache::StampCache,
}

impl PiAgentAnalyzer {
    pub fn new() -> Self {
        Self { cache: crate::stamp_cache::StampCache::new() }
    }

    fn sessions_dir() -> Option<std::path::PathBuf> {
        std::env::home_dir().map(|home| home.join(".pi/agent/sessions"))
    }
}

fn timestamp(entry: &Value) -> Option<DateTime<Utc>> {
    let text = entry.get("timestamp")?.as_str()?;
    DateTime::parse_from_rfc3339(text).ok().map(|t| t.with_timezone(&Utc))
}

/// The text of a message: a string, or its `text` parts (not thinking or tool calls), cut short.
fn message_text(message: &Value) -> Option<String> {
    let text = match message.get("content")? {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(TEXT_CHARS).collect())
}

/// Text kept per message, for the verbose view's transcript lines.
const TEXT_CHARS: usize = 2000;

/// Messages in one session file. Replies from local MTPLX models are costed by `local`.
pub(crate) fn parse_session(path: &Path, local: &crate::analyzers::mtplx::LocalPricing) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(path)?;
    let mut messages = Vec::new();
    let mut session_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let mut project = "unknown".to_string();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        match entry.get("type").and_then(Value::as_str) {
            Some("session") => {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    session_id = id.to_string();
                }
                if let Some(cwd) = entry.get("cwd").and_then(Value::as_str) {
                    project = Path::new(cwd).file_name().and_then(|n| n.to_str()).unwrap_or(cwd).to_string();
                }
            }
            Some("message") => {
                let (Some(message), Some(date)) = (entry.get("message"), timestamp(&entry)) else { continue };
                let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
                let mut stats = Stats::default();
                let (role, model) = match message.get("role").and_then(Value::as_str) {
                    Some("user") => (MessageRole::User, None),
                    Some("assistant") => {
                        let usage = message.get("usage");
                        let field = |key: &str| usage.and_then(|u| u.get(key)).and_then(Value::as_u64).unwrap_or(0);
                        stats.input_tokens = field("input");
                        stats.output_tokens = field("output");
                        stats.cache_read_tokens = field("cacheRead");
                        stats.cache_creation_tokens = field("cacheWrite");
                        stats.cached_tokens = stats.cache_read_tokens;
                        if stats.input_tokens + stats.output_tokens + stats.cache_read_tokens + stats.cache_creation_tokens == 0 {
                            continue; // Failed before any tokens.
                        }
                        stats.tool_calls = message
                            .get("content")
                            .and_then(Value::as_array)
                            .map(|parts| parts.iter().filter(|p| p.get("type").and_then(Value::as_str) == Some("toolCall")).count() as u32)
                            .unwrap_or(0);
                        let model = message.get("model").and_then(Value::as_str).map(str::to_string);
                        let reported = usage
                            .and_then(|u| u.pointer("/cost/total"))
                            .and_then(Value::as_f64)
                            .filter(|c| *c > 0.0);
                        let provider = message.get("provider").and_then(Value::as_str).unwrap_or_default();
                        if crate::analyzers::mtplx::LocalPricing::is_local(provider) {
                            // A local model: Pi records $0; price it as the MTPLX column would.
                            let response = message.get("responseId").and_then(Value::as_str);
                            (stats.cost, stats.energy_wh, stats.energy_cost) =
                                local.price(model.as_deref().unwrap_or_default(), response, &stats);
                        } else {
                            stats.cost = reported.unwrap_or_else(|| {
                                model.as_deref().map_or(0.0, |m| {
                                    crate::models::calculate_total_cost(
                                        m,
                                        stats.input_tokens,
                                        stats.output_tokens,
                                        stats.cache_creation_tokens,
                                        stats.cache_read_tokens,
                                    )
                                })
                            });
                        }
                        (MessageRole::Assistant, model)
                    }
                    _ => continue,
                };
                messages.push(ConversationMessage {
                    application: Application::PiAgent,
                    date,
                    project_hash: project.clone(),
                    conversation_hash: hash_text(&session_id),
                    local_hash: Some(id.to_string()),
                    global_hash: hash_text(&format!("pi-{session_id}-{id}")),
                    model,
                    stats,
                    role,
                    content: message_text(message),
                });
            }
            _ => {}
        }
    }
    Ok(messages)
}

#[async_trait]
impl Analyzer for PiAgentAnalyzer {
    fn display_name(&self) -> &'static str {
        "Pi Agent"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        Self::sessions_dir()
            .map(|dir| vec![format!("{}/*/*.jsonl", dir.display())])
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
        let local = crate::analyzers::mtplx::LocalPricing::load();
        let (messages, _report) = self.cache.parse_all(&sources, |path| parse_session(path, &local));
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
        Self::sessions_dir().is_some_and(|d| d.is_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_mtplx_replies_are_counted_here_with_their_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","id":"sess","timestamp":"2026-10-20T14:39:48Z","cwd":"/Users/me/shop"}
{"type":"message","id":"u1","timestamp":"2026-10-20T14:39:57Z","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}
{"type":"message","id":"a1","timestamp":"2026-10-20T14:40:00Z","message":{"role":"assistant","provider":"mtplx-fast","model":"qwen-fast","responseId":"chatcmpl-1","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"Hi there"}],"usage":{"input":1917,"output":41,"cacheRead":0,"cacheWrite":0,"cost":{"total":0}}}}
"#,
        )
        .unwrap();
        let messages = parse_session(&path, &crate::analyzers::mtplx::LocalPricing::empty()).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content.as_deref(), Some("hello"));
        let reply = &messages[1];
        assert_eq!((reply.model.as_deref(), reply.stats.output_tokens), (Some("qwen-fast"), 41));
        // The text, not the thinking.
        assert_eq!(reply.content.as_deref(), Some("Hi there"));
    }

    #[test]
    fn reads_usage_and_skips_replies_that_failed_before_any_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","version":3,"id":"sess","timestamp":"2026-04-26T10:02:57.739Z","cwd":"/Users/me/shop"}
{"type":"message","id":"u1","timestamp":"2026-04-26T10:04:34.970Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
{"type":"message","id":"a0","timestamp":"2026-04-26T10:04:35.000Z","message":{"role":"assistant","model":"claude-opus-4-7","stopReason":"error","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}}
{"type":"message","id":"a1","timestamp":"2026-04-26T10:05:01.137Z","message":{"role":"assistant","model":"gpt-5.5","content":[{"type":"thinking"},{"type":"toolCall"}],"usage":{"input":1069,"output":77,"cacheRead":5,"cacheWrite":0,"cost":{"total":0.007655}}}}
"#,
        )
        .unwrap();
        let messages = parse_session(&path, &crate::analyzers::mtplx::LocalPricing::empty()).unwrap();
        assert_eq!(messages.len(), 2);
        let reply = &messages[1];
        assert_eq!((reply.role.clone(), reply.model.as_deref()), (MessageRole::Assistant, Some("gpt-5.5")));
        assert_eq!((reply.stats.input_tokens, reply.stats.output_tokens, reply.stats.cache_read_tokens), (1069, 77, 5));
        assert_eq!(reply.stats.cost, 0.007655);
        assert_eq!(reply.stats.tool_calls, 1);
        assert_eq!(reply.project_hash, "shop");
        assert_eq!(reply.application, Application::PiAgent);
    }
}

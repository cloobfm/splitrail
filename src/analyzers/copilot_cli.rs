//! GitHub Copilot CLI (`copilot`), as 0.0.3xx saves it: one JSON per session in
//! `~/.copilot/history-session-state/session_<id>_<ms>.json`.
//!
//! ```json
//! {"sessionId":"1848…","startTime":"2025-09-28T01:52:43.467Z","chatMessages":[…],
//!  "timeline":[{"type":"user","timestamp":"…","text":"…"},
//!              {"type":"copilot","timestamp":"…","text":"…"},
//!              {"type":"tool_call_completed","timestamp":"…","name":"str_replace_editor","arguments":{…},"result":…},
//!              {"type":"error","text":"Model call failed: …quota_exceeded…"}]}
//! ```
//!
//! These files record **no token counts and no model**, so tokens are estimated from text at
//! about 4 characters a token: a reply's output is its text plus the tool calls made after it,
//! and its input is everything said so far, which an Anthropic-backed agent would read from
//! cache except for what is new since the previous reply. The model is taken as Claude Sonnet
//! 4.5, the CLI's default then (its tool call ids are Anthropic's `toolu_`). Copilot is a flat
//! subscription, so the cost is only what the same tokens would cost at API prices.
//!
//! Newer CLIs write `~/.copilot/session-state/<id>/events.jsonl` with real usage on shutdown
//! (upstream #141 reads those); none exist here, so that format is not read yet.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

/// What the old CLI ran by default; the session files do not say.
const DEFAULT_MODEL: &str = "claude-sonnet-4-5";
const CHARS_PER_TOKEN: u64 = 4;

pub struct CopilotCliAnalyzer {
    /// Session files are rewritten whole; parse only the ones that changed (BZL-74).
    cache: crate::stamp_cache::StampCache,
}

impl CopilotCliAnalyzer {
    pub fn new() -> Self {
        Self { cache: crate::stamp_cache::StampCache::new() }
    }

    fn history_dir() -> Option<std::path::PathBuf> {
        std::env::home_dir().map(|home| home.join(".copilot/history-session-state"))
    }
}

fn tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(CHARS_PER_TOKEN)
}

/// Characters an entry adds to the conversation.
fn size(entry: &Value) -> usize {
    match entry.get("type").and_then(Value::as_str) {
        Some("tool_call_completed") | Some("tool_call_requested") => {
            entry.get("arguments").map_or(0, |a| a.to_string().len())
                + entry.get("result").map_or(0, |r| r.as_str().map_or_else(|| r.to_string().len(), str::len))
        }
        _ => entry.get("text").and_then(Value::as_str).map_or(0, str::len),
    }
}

/// Messages in one session file, with estimated tokens.
pub(crate) fn parse_session(path: &Path) -> Result<Vec<ConversationMessage>> {
    let session: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let session_id = session.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_string();
    let conversation_hash = hash_text(&session_id);
    let timeline = session.get("timeline").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut messages: Vec<ConversationMessage> = Vec::new();
    // Characters said before the latest reply, and since it.
    let (mut context, mut fresh) = (0usize, 0usize);
    let mut last_reply: Option<usize> = None;
    for entry in &timeline {
        let Some(date) = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            continue;
        };
        let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let chars = size(entry);
        let message = |role, model: Option<&str>, stats| ConversationMessage {
            application: Application::CopilotCli,
            date,
            project_hash: "unknown".into(),
            conversation_hash: conversation_hash.clone(),
            local_hash: Some(id.to_string()),
            global_hash: hash_text(&format!("copilot_cli-{session_id}-{id}")),
            model: model.map(str::to_string),
            stats,
            role,
            content: None,
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("user") => {
                messages.push(message(MessageRole::User, None, Stats::default()));
                fresh += chars;
            }
            Some("copilot") => {
                let stats = Stats {
                    input_tokens: tokens(fresh),
                    cache_read_tokens: tokens(context),
                    cached_tokens: tokens(context),
                    output_tokens: tokens(chars),
                    ..Default::default()
                };
                context += fresh + chars;
                fresh = 0;
                last_reply = Some(messages.len());
                messages.push(message(MessageRole::Assistant, Some(DEFAULT_MODEL), stats));
            }
            Some("tool_call_completed") => {
                // The call is the reply's output; its result is read on the next turn.
                if let Some(i) = last_reply {
                    let reply = &mut messages[i];
                    reply.stats.tool_calls += 1;
                    reply.stats.output_tokens += tokens(entry.get("arguments").map_or(0, |a| a.to_string().len()));
                }
                fresh += chars;
            }
            _ => {}
        }
    }
    for reply in messages.iter_mut().filter(|m| m.role == MessageRole::Assistant) {
        let s = &reply.stats;
        reply.stats.cost =
            crate::models::calculate_total_cost(DEFAULT_MODEL, s.input_tokens, s.output_tokens, 0, s.cache_read_tokens);
    }
    Ok(messages)
}

#[async_trait]
impl Analyzer for CopilotCliAnalyzer {
    fn display_name(&self) -> &'static str {
        "Copilot CLI"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        Self::history_dir()
            .map(|dir| vec![format!("{}/session_*.json", dir.display())])
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
        let (messages, _report) = self.cache.parse_all(&sources, parse_session);
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
        Self::history_dir().is_some_and(|d| d.is_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_tokens_from_the_timeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session_x_1.json");
        std::fs::write(
            &path,
            r#"{"sessionId":"x","startTime":"2025-09-28T01:52:43.467Z","chatMessages":[],"timeline":[
              {"id":"i","type":"info","timestamp":"2025-09-28T01:52:47Z","text":"Connected"},
              {"id":"u","type":"user","timestamp":"2025-09-28T02:00:59Z","text":"12345678"},
              {"id":"c1","type":"copilot","timestamp":"2025-09-28T02:01:11Z","text":"1234"},
              {"id":"t","type":"tool_call_completed","timestamp":"2025-09-28T02:01:20Z","name":"bash","arguments":{"c":"ls"},"result":"aaaaaaaa"},
              {"id":"c2","type":"copilot","timestamp":"2025-09-28T02:01:30Z","text":"12345678"},
              {"id":"e","type":"error","timestamp":"2025-09-28T02:02:00Z","text":"Model call failed: quota_exceeded"}]}"#,
        )
        .unwrap();
        let messages = parse_session(&path).unwrap();
        let replies: Vec<&Stats> =
            messages.iter().filter(|m| m.role == MessageRole::Assistant).map(|m| &m.stats).collect();
        assert_eq!(messages.len(), 3);
        // c1 reads the 8-char prompt fresh (2 tokens) and writes 1 token, then calls a tool
        // (`{"c":"ls"}`, 10 chars: 3 tokens).
        assert_eq!((replies[0].input_tokens, replies[0].cache_read_tokens, replies[0].output_tokens), (2, 0, 4));
        assert_eq!(replies[0].tool_calls, 1);
        // c2 reads the tool call and result fresh (18 chars) and the first turn (12) from cache.
        assert_eq!((replies[1].input_tokens, replies[1].cache_read_tokens, replies[1].output_tokens), (5, 3, 2));
        assert!(replies[1].cost > 0.0);
        assert_eq!(messages[1].model.as_deref(), Some(DEFAULT_MODEL));
    }
}


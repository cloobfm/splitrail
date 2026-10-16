//! Kiro CLI's new TUI (2.x, `tui.js`), which keeps each session in `~/.kiro/sessions/cli/`
//! rather than the `data.sqlite3` the classic chat used:
//!
//! * `<id>.json`: `cwd`, `title`, and `session_state.conversation_metadata.user_turn_metadatas`,
//!   one per turn: `end_timestamp`, `metering_usage` (`[{"value": 0.086, "unit": "credit"}]`),
//!   `input_token_count` / `output_token_count` (0 for the `auto` model), `builtin_tool_uses`,
//!   and the reply (`result.Ok.content[].data`). The model is
//!   `session_state.rts_model_state.model_info.model_id`.
//! * `<id>.jsonl`: one event per line; `Prompt` lines carry the user's text and
//!   `meta.timestamp` (epoch seconds).
//!
//! Kiro bills in credits, so cost is the credits used at `DOLLARS_PER_CREDIT`.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::types::{Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

/// Pro's $20 for 1,000 credits; overage and add-on credits cost $0.04.
pub const DOLLARS_PER_CREDIT: f64 = 0.02;

pub fn sessions_dir() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".kiro").join("sessions").join("cli"))
}

/// Every session's metadata file.
pub fn session_files() -> Vec<PathBuf> {
    let Some(dir) = sessions_dir() else { return Vec::new() };
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect()
        })
        .unwrap_or_default()
}

fn text_of(content: Option<&Value>) -> Option<String> {
    let text: Vec<&str> = content?
        .as_array()?
        .iter()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("data").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

/// Messages for one session: its metadata (`<id>.json`) and event log (`<id>.jsonl`, may be
/// empty).
pub fn parse_session(meta: &str, events: &str) -> Vec<ConversationMessage> {
    let Ok(meta) = serde_json::from_str::<Value>(meta) else {
        return Vec::new();
    };
    let session_id = meta.get("session_id").and_then(Value::as_str).unwrap_or_default();
    if session_id.is_empty() {
        return Vec::new();
    }
    let conversation_hash = hash_text(session_id);
    let project = meta
        .get("cwd")
        .and_then(Value::as_str)
        .and_then(|cwd| Path::new(cwd).file_name()?.to_str().map(str::to_string))
        .unwrap_or_default();
    let state = meta.get("session_state");
    let model = state
        .and_then(|s| s.pointer("/rts_model_state/model_info/model_id"))
        .and_then(Value::as_str)
        .map(|m| format!("kiro-{m}"));
    let mut messages = Vec::new();

    for line in events.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
        if event.get("kind").and_then(Value::as_str) != Some("Prompt") {
            continue;
        }
        let data = event.get("data");
        let Some(at) = data
            .and_then(|d| d.pointer("/meta/timestamp"))
            .and_then(Value::as_i64)
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        else {
            continue;
        };
        let id = data.and_then(|d| d.get("message_id")).and_then(Value::as_str).unwrap_or_default();
        messages.push(ConversationMessage {
            application: Application::KiroCli,
            date: at,
            project_hash: project.clone(),
            conversation_hash: conversation_hash.clone(),
            local_hash: None,
            global_hash: hash_text(&format!("kiro-session:{session_id}:prompt:{id}:{}", at.timestamp())),
            model: None,
            stats: Stats::default(),
            role: MessageRole::User,
            content: text_of(data.and_then(|d| d.get("content"))),
        });
    }

    let turns = state
        .and_then(|s| s.pointer("/conversation_metadata/user_turn_metadatas"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, turn) in turns.iter().enumerate() {
        let Some(at) = turn
            .get("end_timestamp")
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc))
        else {
            continue;
        };
        let credits: f64 = turn
            .get("metering_usage")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|m| m.get("unit").and_then(Value::as_str) == Some("credit"))
            .filter_map(|m| m.get("value").and_then(Value::as_f64))
            .sum();
        let count = |key: &str| turn.get(key).and_then(Value::as_u64).unwrap_or(0);
        let result = turn.pointer("/result/Ok");
        messages.push(ConversationMessage {
            application: Application::KiroCli,
            date: at,
            project_hash: project.clone(),
            conversation_hash: conversation_hash.clone(),
            local_hash: None,
            global_hash: hash_text(&format!("kiro-session:{session_id}:turn:{i}:{}", at.timestamp_millis())),
            model: model.clone().or_else(|| Some("kiro".to_string())),
            stats: Stats {
                input_tokens: count("input_token_count"),
                output_tokens: count("output_token_count"),
                tool_calls: u32::try_from(count("builtin_tool_uses")).unwrap_or(u32::MAX),
                cost: credits * DOLLARS_PER_CREDIT,
                ..Default::default()
            },
            role: MessageRole::Assistant,
            content: text_of(result.and_then(|r| r.get("content"))),
        });
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = r#"{"session_id": "s1", "cwd": "/Users/x/nas-devbox", "title": "t",
        "session_state": {"rts_model_state": {"model_info": {"model_id": "auto"}},
        "conversation_metadata": {"user_turn_metadatas": [
            {"end_timestamp": "2026-10-16T06:37:12.415534Z", "input_token_count": 0, "output_token_count": 0,
             "builtin_tool_uses": 2, "metering_usage": [{"value": 0.5, "unit": "credit", "unitPlural": "credits"}],
             "result": {"Ok": {"content": [{"kind": "text", "data": "Here is what I can do"}]}}}]}}}"#;
    const EVENTS: &str = r#"{"version": "v1", "kind": "Prompt", "data": {"message_id": "m1", "content": [{"kind": "text", "data": "what can we do?"}], "meta": {"timestamp": 1792132624}}}
{"version": "v1", "kind": "AssistantMessage", "data": {"message_id": "m2", "content": []}}"#;

    #[test]
    fn a_session_becomes_prompts_and_credit_priced_turns() {
        let messages = parse_session(META, EVENTS);
        assert_eq!(messages.len(), 2);
        let (prompt, turn) = (&messages[0], &messages[1]);
        assert_eq!(prompt.role, MessageRole::User);
        assert_eq!(prompt.content.as_deref(), Some("what can we do?"));
        assert_eq!(prompt.project_hash, "nas-devbox");
        assert_eq!(turn.role, MessageRole::Assistant);
        assert_eq!(turn.model.as_deref(), Some("kiro-auto"));
        assert!((turn.stats.cost - 0.01).abs() < 1e-12);
        assert_eq!(turn.stats.tool_calls, 2);
        assert_eq!(turn.content.as_deref(), Some("Here is what I can do"));
        assert_eq!(prompt.conversation_hash, turn.conversation_hash);
    }

    #[test]
    fn unreadable_sessions_give_nothing() {
        assert!(parse_session("{", "").is_empty());
        assert!(parse_session(r#"{"cwd": "/x"}"#, "").is_empty());
    }
}

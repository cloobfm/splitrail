//! Antigravity CLI (`agy`). It keeps no readable usage logs (its conversation store is opaque
//! protobuf), so usage comes from `scripts/agy_statusline_capture.py`, which runs as `agy`'s
//! status line and appends each growth in a conversation's token totals to
//! `~/.config/splitrail/antigravity/usage.jsonl`:
//!
//! ```json
//! {"ts": 1790880000.5, "conversation_id": "c1", "session_id": "s1", "model": "gemini-3-pro",
//!  "cwd": "/Users/x/proj", "delta_input": 500, "delta_output": 80}
//! ```
//!
//! Each line becomes one assistant message. The totals do not separate cache reads, so cost is
//! input priced at the full input rate: an upper bound when the model caches.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub const DISPLAY_NAME: &str = "Antigravity";
/// Model name for events whose payload had none. Messages need a model to count as assistant
/// messages; this one has no price, so it is costed at $0 and listed in the warnings.
const UNKNOWN_MODEL: &str = "antigravity-unknown";

#[derive(Debug, Deserialize)]
struct UsageEvent {
    ts: f64,
    conversation_id: String,
    model: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    delta_input: u64,
    #[serde(default)]
    delta_output: u64,
}

pub struct AntigravityAnalyzer {
    usage_file: Option<PathBuf>,
}

impl AntigravityAnalyzer {
    pub fn new() -> Self {
        Self {
            usage_file: std::env::home_dir().map(|home| {
                home.join(".config")
                    .join("splitrail")
                    .join("antigravity")
                    .join("usage.jsonl")
            }),
        }
    }
}

impl Default for AntigravityAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// `agy` often omits `model.id`, leaving the display name (`Gemini 3.8 Flash (High)`). Reduce
/// that to the slug the price feed uses (`gemini-3.8-flash`): drop the parenthesised effort,
/// lowercase, hyphenate. Ids that are already slugs pass through unchanged.
pub fn normalize_model(name: &str) -> String {
    let base = name.split(" (").next().unwrap_or(name).trim();
    base.split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

/// Parse a usage log into messages. Malformed lines (a partial write, say) are skipped.
pub fn parse_usage_log(content: &str, source: &Path) -> Vec<ConversationMessage> {
    let source = source.to_string_lossy();
    content
        .lines()
        .enumerate()
        .filter_map(|(line_no, line)| {
            let event: UsageEvent = serde_json::from_str(line).ok()?;
            let date = DateTime::<Utc>::from_timestamp_millis((event.ts * 1000.0) as i64)?;
            let model = event
                .model
                .map(|m| normalize_model(&m))
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
            let cost = calculate_total_cost(&model, event.delta_input, event.delta_output, 0, 0);
            Some(ConversationMessage {
                application: Application::Antigravity,
                date,
                project_hash: event.cwd.as_deref().map(hash_text).unwrap_or_default(),
                conversation_hash: hash_text(&event.conversation_id),
                local_hash: None,
                global_hash: hash_text(&format!("{source}:{line_no}:{line}")),
                model: Some(model),
                stats: Stats {
                    input_tokens: event.delta_input,
                    output_tokens: event.delta_output,
                    cost,
                    ..Default::default()
                },
                role: MessageRole::Assistant,
                content: None,
            })
        })
        .collect()
}

#[async_trait]
impl Analyzer for AntigravityAnalyzer {
    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
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
        for source in sources {
            let content = std::fs::read_to_string(&source.path)?;
            messages.extend(parse_usage_log(&content, &source.path));
        }
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations = daily_stats
            .values()
            .map(|stats| stats.conversations as u64)
            .sum();

        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        self.discover_data_sources()
            .is_ok_and(|sources| !sources.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_is_one_assistant_message() {
        let log = concat!(
            r#"{"ts": 1790880000.5, "conversation_id": "c1", "session_id": "s1", "model": "gemini-2.5-pro", "cwd": "/p", "delta_input": 1000, "delta_output": 100}"#,
            "\n",
            "{\"ts\": 1790880\n", // A torn write.
            r#"{"ts": 1790880100, "conversation_id": "c2", "model": null, "cwd": null, "delta_input": 5, "delta_output": 1}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/u.jsonl"));
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.application, Application::Antigravity);
        assert_eq!(first.role, MessageRole::Assistant);
        assert_eq!(first.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(first.stats.input_tokens, 1000);
        assert_eq!(first.stats.output_tokens, 100);
        assert_eq!(first.date.timestamp_millis(), 1_790_880_000_500);
        assert_eq!(first.project_hash, hash_text("/p"));
        assert!(first.stats.cost > 0.0);

        // No model: still an assistant message, costed at zero.
        assert_eq!(messages[1].model.as_deref(), Some(UNKNOWN_MODEL));
        assert_eq!(messages[1].stats.cost, 0.0);
        assert_ne!(first.conversation_hash, messages[1].conversation_hash);
        assert_ne!(first.global_hash, messages[1].global_hash);
    }

    #[test]
    fn display_names_become_feed_slugs() {
        assert_eq!(
            normalize_model("Gemini 3.8 Flash (High)"),
            "gemini-3.8-flash"
        );
        assert_eq!(normalize_model("Claude Sonnet 4.5"), "claude-sonnet-4.5");
        assert_eq!(normalize_model("gemini-2.5-pro"), "gemini-2.5-pro");
        assert_eq!(normalize_model("  "), "");
    }

    #[test]
    fn conversations_are_counted_per_conversation_id() {
        let log = concat!(
            r#"{"ts": 1790880000, "conversation_id": "c1", "delta_input": 10, "delta_output": 1}"#,
            "\n",
            r#"{"ts": 1790880060, "conversation_id": "c1", "delta_input": 20, "delta_output": 2}"#,
            "\n",
            r#"{"ts": 1790880120, "conversation_id": "c2", "delta_input": 30, "delta_output": 3}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/u.jsonl"));
        let days = crate::utils::aggregate_by_date(&messages);
        assert_eq!(days.values().map(|d| d.conversations).sum::<u32>(), 2);
        assert_eq!(days.values().map(|d| d.stats.input_tokens).sum::<u64>(), 60);
    }
}

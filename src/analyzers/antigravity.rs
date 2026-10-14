//! Antigravity CLI (`agy`). Conversations are read from agy's own store,
//! `~/.gemini/antigravity-cli/conversations/<id>.db` (see `antigravity_store`): prompts, replies,
//! tool calls and each model call's exact usage, for interactive and headless runs alike.
//!
//! Conversations with no database (deleted, or from before it existed) fall back to what
//! `scripts/agy_statusline_capture.py`, running as `agy`'s status line, appended to
//! `~/.config/splitrail/antigravity/usage.jsonl`: each growth in a conversation's token totals.
//! The status line also supplies the plan quota (`plan_limits`).
//!
//! ```json
//! {"ts": 1790880000.5, "conversation_id": "c1", "session_id": "s1", "model": "gemini-3-pro",
//!  "cwd": "/Users/x/proj", "delta_input": 500, "delta_output": 80}
//! ```
//!
//! Each line becomes one assistant message. The status line totals do not separate cache reads,
//! so cost is input priced at the full input rate: an upper bound when the model caches.
//!
//! Headless runs through `agy-run` (`scripts/headless_run.py`) add a `"source": "print"` line
//! with the run's exact usage, cache reads included. The status line fires during those runs
//! too, so the status line lines of a conversation up to its print line (and after the one
//! before it, for a resumed conversation) describe the same run and are dropped.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::antigravity_store::{self, Step};

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
    #[serde(default)]
    cache_read_tokens: u64,
    source: Option<String>,
    reply_preview: Option<String>,
}

/// A conversation database's messages, kept until the database or its WAL changes.
struct CachedConversation {
    stamp: Stamp,
    messages: Arc<Vec<ConversationMessage>>,
}

type Stamp = [(u64, Option<SystemTime>); 2];

fn stamp(db: &Path) -> Stamp {
    let of = |path: &Path| {
        std::fs::metadata(path).map_or((0, None), |m| (m.len(), m.modified().ok()))
    };
    [of(db), of(&db.with_extension("db-wal"))]
}

pub struct AntigravityAnalyzer {
    usage_file: Option<PathBuf>,
    conversations_dir: Option<PathBuf>,
    cache: Mutex<HashMap<PathBuf, CachedConversation>>,
}

impl AntigravityAnalyzer {
    pub fn new() -> Self {
        let home = std::env::home_dir();
        Self {
            usage_file: home.as_ref().map(|home| {
                home.join(".config")
                    .join("splitrail")
                    .join("antigravity")
                    .join("usage.jsonl")
            }),
            conversations_dir: home
                .map(|home| home.join(".gemini").join("antigravity-cli").join("conversations")),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// A conversation database's messages, re-read only when it changed. A database that cannot
    /// be read right now (mid-write) keeps its previous messages.
    fn conversation_messages(&self, db: &Path) -> Option<Arc<Vec<ConversationMessage>>> {
        let current = stamp(db);
        let mut cache = self.cache.lock().ok()?;
        if let Some(cached) = cache.get(db).filter(|c| c.stamp == current) {
            return Some(cached.messages.clone());
        }
        let id = db.file_stem()?.to_str()?.to_string();
        match antigravity_store::read_conversation(db) {
            Ok(steps) => {
                let messages = Arc::new(messages_from_steps(&id, &steps));
                cache.insert(
                    db.to_path_buf(),
                    CachedConversation { stamp: current, messages: messages.clone() },
                );
                Some(messages)
            }
            Err(_) => cache.get(db).map(|c| c.messages.clone()),
        }
    }
}

/// The project's folder name, as Claude's columns show it.
fn project_name(cwd: Option<&str>) -> String {
    cwd.map(|cwd| {
        Path::new(cwd)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(cwd)
            .to_string()
    })
    .unwrap_or_default()
}

/// Messages for one conversation's steps: a user message per prompt, an assistant message per
/// model response (priced from its own usage and model).
pub fn messages_from_steps(conversation_id: &str, steps: &[Step]) -> Vec<ConversationMessage> {
    let conversation_hash = hash_text(conversation_id);
    let mut cwd: Option<&str> = None;
    let mut last_model: Option<String> = None;
    let mut messages = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let global_hash = hash_text(&format!("antigravity:{conversation_id}:{i}"));
        match step {
            Step::User { at, text, cwd: step_cwd } => {
                cwd = step_cwd.as_deref().or(cwd);
                messages.push(ConversationMessage {
                    application: Application::Antigravity,
                    date: *at,
                    project_hash: project_name(cwd),
                    conversation_hash: conversation_hash.clone(),
                    local_hash: None,
                    global_hash,
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::User,
                    content: text.clone(),
                });
            }
            Step::Response { at, text, tool_calls, model, usage } => {
                if let Some(model) = model {
                    last_model = Some(normalize_model(model));
                }
                let model = last_model.clone().unwrap_or_else(|| UNKNOWN_MODEL.to_string());
                let usage = usage.clone().unwrap_or_default();
                let cost = calculate_total_cost(&model, usage.input, usage.output, 0, usage.cache_read);
                messages.push(ConversationMessage {
                    application: Application::Antigravity,
                    date: *at,
                    project_hash: project_name(cwd),
                    conversation_hash: conversation_hash.clone(),
                    local_hash: None,
                    global_hash,
                    model: Some(model),
                    stats: Stats {
                        input_tokens: usage.input,
                        // Visible output and thinking, apart, as the other columns show them.
                        output_tokens: usage.output.saturating_sub(usage.thinking),
                        reasoning_tokens: usage.thinking,
                        cache_read_tokens: usage.cache_read,
                        cached_tokens: usage.cache_read,
                        tool_calls: *tool_calls,
                        cost,
                        ..Default::default()
                    },
                    role: MessageRole::Assistant,
                    content: text.clone(),
                });
            }
        }
    }
    messages
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
    let slug = base
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    // The conversation store appends the effort to the slug: `gemini-3.8-flash-high`,
    // `claude-sonnet-5-5-medium`, `gemini-3.8-flash-n`.
    for effort in ["-minimal", "-low", "-medium", "-xhigh", "-high", "-n"] {
        if let Some(stripped) = slug.strip_suffix(effort) {
            return stripped.to_string();
        }
    }
    slug
}

/// Parse a usage log into messages. Malformed lines (a partial write, say) are skipped.
pub fn parse_usage_log(
    content: &str,
    source: &Path,
    skip_conversations: &HashSet<String>,
) -> Vec<ConversationMessage> {
    let source = source.to_string_lossy();
    let events: Vec<(usize, &str, UsageEvent)> = content
        .lines()
        .enumerate()
        .filter_map(|(line_no, line)| Some((line_no, line, serde_json::from_str(line).ok()?)))
        .collect();
    // Print-run end times per conversation, oldest first.
    let mut runs: HashMap<&str, Vec<f64>> = HashMap::new();
    for (_, _, event) in &events {
        if event.source.as_deref() == Some("print") {
            runs.entry(&event.conversation_id).or_default().push(event.ts);
        }
    }
    for ends in runs.values_mut() {
        ends.sort_by(f64::total_cmp);
    }
    // A status line line covered by a print run: at or before a run's end, after the previous.
    let in_print_run = |event: &UsageEvent| {
        runs.get(event.conversation_id.as_str()).is_some_and(|ends| {
            ends.iter()
                .any(|&end| event.ts <= end && !ends.iter().any(|&e| e < end && e >= event.ts))
        })
    };
    events
        .iter()
        .filter(|(_, _, event)| !skip_conversations.contains(&event.conversation_id))
        .filter(|(_, _, event)| event.source.as_deref() == Some("print") || !in_print_run(event))
        .filter_map(|(line_no, line, event)| {
            let date = DateTime::<Utc>::from_timestamp_millis((event.ts * 1000.0) as i64)?;
            let model = event
                .model
                .as_deref()
                .map(normalize_model)
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
            let cost = calculate_total_cost(
                &model,
                event.delta_input,
                event.delta_output,
                0,
                event.cache_read_tokens,
            );
            Some(ConversationMessage {
                application: Application::Antigravity,
                date,
                project_hash: project_name(event.cwd.as_deref()),
                conversation_hash: hash_text(&event.conversation_id),
                local_hash: None,
                global_hash: hash_text(&format!("{source}:{line_no}:{line}")),
                model: Some(model),
                stats: Stats {
                    input_tokens: event.delta_input,
                    output_tokens: event.delta_output,
                    cache_read_tokens: event.cache_read_tokens,
                    cached_tokens: event.cache_read_tokens,
                    cost,
                    ..Default::default()
                },
                role: MessageRole::Assistant,
                content: event.reply_preview.clone(),
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
        let mut patterns: Vec<String> = self
            .usage_file
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        if let Some(dir) = &self.conversations_dir {
            patterns.push(dir.join("*.db").to_string_lossy().into_owned());
            patterns.push(dir.join("*.db-wal").to_string_lossy().into_owned());
        }
        patterns
    }

    /// The usage log and every conversation database with its WAL (listed so a write that only
    /// reaches the WAL still counts as a change).
    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let mut sources: Vec<DataSource> = self
            .usage_file
            .iter()
            .filter(|path| path.is_file())
            .map(|path| DataSource { path: path.clone() })
            .collect();
        if let Some(dir) = &self.conversations_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name.ends_with(".db") || name.ends_with(".db-wal") {
                    sources.push(DataSource { path });
                }
            }
        }
        Ok(sources)
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let databases: Vec<&PathBuf> = sources
            .iter()
            .map(|s| &s.path)
            .filter(|p| p.extension().is_some_and(|e| e == "db"))
            .collect();
        let mut messages = Vec::new();
        let mut from_store = HashSet::new();
        for db in databases {
            if let Some(found) = self.conversation_messages(db) {
                if let Some(id) = db.file_stem().and_then(|s| s.to_str()) {
                    from_store.insert(id.to_string());
                }
                messages.extend(found.iter().cloned());
            }
        }
        for source in sources.iter().filter(|s| Some(&s.path) == self.usage_file.as_ref()) {
            let content = std::fs::read_to_string(&source.path)?;
            messages.extend(parse_usage_log(&content, &source.path, &from_store));
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
mod print_run_tests {
    use super::*;

    #[test]
    fn status_line_lines_of_a_print_run_are_not_counted_twice() {
        let log = [
            r#"{"ts": 100, "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 50, "delta_output": 5}"#,
            r#"{"ts": 110, "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 60, "delta_output": 6}"#,
            r#"{"ts": 111, "source": "print", "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 400, "delta_output": 40, "cache_read_tokens": 9000, "reply_preview": "done"}"#,
            // Resumed with --conversation: its own status line lines and run.
            r#"{"ts": 200, "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 70, "delta_output": 7}"#,
            r#"{"ts": 201, "source": "print", "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 80, "delta_output": 8}"#,
            // Interactive afterwards, and another conversation: kept.
            r#"{"ts": 300, "conversation_id": "c1", "model": "gemini-3-pro", "delta_input": 1, "delta_output": 1}"#,
            r#"{"ts": 105, "conversation_id": "c2", "model": "gemini-3-pro", "delta_input": 2, "delta_output": 2}"#,
        ]
        .join("\n");
        let messages = parse_usage_log(&log, Path::new("/u.jsonl"), &HashSet::new());
        let inputs: Vec<u64> = messages.iter().map(|m| m.stats.input_tokens).collect();
        assert_eq!(inputs, vec![400, 80, 1, 2]);
        assert_eq!(messages[0].stats.cache_read_tokens, 9000);
        assert_eq!(messages[0].content.as_deref(), Some("done"));
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
        let messages = parse_usage_log(log, Path::new("/u.jsonl"), &HashSet::new());
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.application, Application::Antigravity);
        assert_eq!(first.role, MessageRole::Assistant);
        assert_eq!(first.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(first.stats.input_tokens, 1000);
        assert_eq!(first.stats.output_tokens, 100);
        assert_eq!(first.date.timestamp_millis(), 1_790_880_000_500);
        assert_eq!(first.project_hash, "p"); // Folder name, as Claude's columns show it.
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
        // Store slugs carry the effort.
        assert_eq!(normalize_model("gemini-3.8-flash-high"), "gemini-3.8-flash");
        assert_eq!(normalize_model("gemini-3.8-flash-n"), "gemini-3.8-flash");
        assert_eq!(normalize_model("claude-sonnet-5-5-medium"), "claude-sonnet-5-5");
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
        let messages = parse_usage_log(log, Path::new("/u.jsonl"), &HashSet::new());
        let days = crate::utils::aggregate_by_date(&messages);
        assert_eq!(days.values().map(|d| d.conversations).sum::<u32>(), 2);
        assert_eq!(days.values().map(|d| d.stats.input_tokens).sum::<u64>(), 60);
    }
}

#[cfg(test)]
mod live {
    /// Reads the real conversation store: `cargo test antigravity_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn antigravity_live_store_has_data() {
        use crate::analyzer::Analyzer;
        let stats = super::AntigravityAnalyzer::new().get_stats().await.unwrap();
        for (day, s) in &stats.daily_stats {
            println!("{day} in {} out {} think {} cache {} cost {:.2} tools {} convs {}", s.stats.input_tokens, s.stats.output_tokens, s.stats.reasoning_tokens, s.stats.cache_read_tokens, s.stats.cost, s.stats.tool_calls, s.conversations);
        }
        let users = stats.messages.iter().filter(|m| m.role == crate::types::MessageRole::User).count();
        let models: std::collections::BTreeSet<_> = stats.messages.iter().filter_map(|m| m.model.clone()).collect();
        println!("messages {} user {users} models {models:?}", stats.messages.len());
        assert!(stats.messages.iter().any(|m| m.content.is_some()));
    }
}

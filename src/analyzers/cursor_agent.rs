//! Cursor Agent CLI (`cursor-agent`). It keeps no readable usage locally (transcripts hold text
//! only, the chat store is encrypted), so usage comes from `scripts/cursor_hook_capture.py`,
//! registered for the `afterAgentResponse` and `stop` hooks in `~/.cursor/hooks.json`. Each call
//! appends a line to `~/.config/splitrail/cursor/usage.jsonl`:
//!
//! ```json
//! {"ts":1791946000.2,"hook_event_name":"stop","conversation_id":"c1","generation_id":"g1",
//!  "model":"claude-4.5-sonnet","input_tokens":1200,"output_tokens":300,
//!  "cache_read_tokens":50000,"cache_write_tokens":0,"cwd":"/Users/x/proj","user_email":"me@x"}
//! ```
//!
//! Both hooks report the same totals for a generation (one user turn), so each generation
//! becomes one assistant message; an aborted turn only fires `stop` and still counts. Print-mode
//! runs (no hooks fire there) are logged to the same file by `scripts/cursor_agent_wrapper.py`
//! with `hook_event_name: "print"`, prompt and reply previews on the same line. The
//! `beforeSubmitPrompt` line of a generation (`prompt_preview`) becomes its user message, and
//! `afterAgentResponse`'s `reply_preview` the assistant's text, for the activity feed.
//!
//! With a Cursor API key, `cursor_limits` also saves Cursor's billed usage events to
//! `cursor/charges.jsonl`. Those are authoritative: hooks and print mode report only part of a
//! turn's requests (summarization and subagent calls are billed but not reported), and Auto turns
//! carry no price. A turn takes the events of its conversation billed after the previous turn
//! ended and by the time it ended: their summed tokens and charged cost replace the local counts.
//! Events of conversations with no captured turn (IDE, bots) are left out.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::analyzer::{Analyzer, DataSource};
use crate::cursor_limits::Charge;
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub const DISPLAY_NAME: &str = "Cursor Agent";
/// Model for turns where Cursor chose the model itself (`default`/`auto`); it has no fixed
/// price, so those turns cost $0 here rather than a guess.
const AUTO_MODEL: &str = "cursor-auto";
/// Slack for a turn's last event being stamped just after its hook fired.
const CHARGE_GRACE_MS: i64 = 2_000;

/// Charges by conversation, oldest first.
pub type ChargeIndex = HashMap<String, Vec<Charge>>;

pub fn index_charges(content: &str) -> ChargeIndex {
    let mut index: ChargeIndex = HashMap::new();
    for charge in content
        .lines()
        .filter_map(|line| serde_json::from_str::<Charge>(line).ok())
    {
        index
            .entry(charge.conversation_id.clone())
            .or_default()
            .push(charge);
    }
    for charges in index.values_mut() {
        charges.sort_by_key(|c| c.ts);
    }
    index
}

#[derive(Debug, Deserialize)]
struct HookLine {
    ts: f64,
    conversation_id: Option<String>,
    generation_id: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    user_email: Option<String>,
    prompt_preview: Option<String>,
    reply_preview: Option<String>,
    hook_event_name: Option<String>,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
}

fn model_name(model: Option<&str>) -> String {
    match model.map(str::trim) {
        // Hooks say `default`, print mode's stream says `Auto`.
        Some(m)
            if !m.is_empty()
                && !m.eq_ignore_ascii_case("default")
                && !m.eq_ignore_ascii_case("auto") =>
        {
            m.to_string()
        }
        _ => AUTO_MODEL.to_string(),
    }
}

/// Parse a hook log into one assistant message per generation. Malformed lines (a torn write)
/// are skipped. The newest login seen is recorded for the dashboard's Login row.
pub fn parse_usage_log(
    content: &str,
    source: &Path,
    charges: &ChargeIndex,
) -> Vec<ConversationMessage> {
    // generation key -> merged line; the two hooks of one turn carry the same totals, so the
    // larger of each counter is kept in case one of them fired before usage was final.
    let mut turns: HashMap<String, HookLine> = HashMap::new();
    // generation key -> the user's prompt line (its own cwd/conversation, for turns still running).
    let mut prompts: HashMap<String, HookLine> = HashMap::new();
    let mut order = Vec::new();
    let mut login: Option<(f64, String)> = None;
    for (line_no, line) in content.lines().enumerate() {
        let Ok(event) = serde_json::from_str::<HookLine>(line) else {
            continue;
        };
        if let Some(email) = event.user_email.as_deref().filter(|e| !e.is_empty())
            && login.as_ref().is_none_or(|(ts, _)| event.ts >= *ts)
        {
            login = Some((
                event.ts,
                email.split('@').next().unwrap_or(email).to_string(),
            ));
        }
        let key = event
            .generation_id
            .clone()
            .unwrap_or_else(|| format!("{}:{line_no}", source.display()));
        if event.hook_event_name.as_deref() == Some("beforeSubmitPrompt") {
            if event.prompt_preview.is_some() {
                if !order.contains(&key) {
                    order.push(key.clone());
                }
                prompts.insert(key, event);
            }
            continue;
        }
        match turns.get_mut(&key) {
            Some(turn) => {
                turn.ts = turn.ts.max(event.ts);
                turn.input_tokens = turn.input_tokens.max(event.input_tokens);
                turn.output_tokens = turn.output_tokens.max(event.output_tokens);
                turn.cache_read_tokens = turn.cache_read_tokens.max(event.cache_read_tokens);
                turn.cache_write_tokens = turn.cache_write_tokens.max(event.cache_write_tokens);
                if turn.model.is_none() {
                    turn.model = event.model;
                }
                if turn.cwd.is_none() {
                    turn.cwd = event.cwd;
                }
                if turn.reply_preview.is_none() {
                    turn.reply_preview = event.reply_preview;
                }
            }
            None => {
                if !order.contains(&key) {
                    order.push(key.clone());
                }
                turns.insert(key, event);
            }
        }
    }
    if let Some((ts, name)) = login
        && let Some(at) = DateTime::<Utc>::from_timestamp_millis((ts * 1000.0) as i64)
    {
        crate::plan_limits::observe_login(DISPLAY_NAME, at, name);
    }

    // Turn end times per conversation, to split its billed events between turns.
    let mut ends: HashMap<&str, Vec<i64>> = HashMap::new();
    for turn in turns.values() {
        if let Some(conversation) = turn.conversation_id.as_deref() {
            ends.entry(conversation)
                .or_default()
                .push((turn.ts * 1000.0) as i64);
        }
    }
    for list in ends.values_mut() {
        list.sort_unstable();
    }
    let billed = |turn: &HookLine| -> Option<Vec<&Charge>> {
        let conversation = turn.conversation_id.as_deref()?;
        let end = (turn.ts * 1000.0) as i64;
        let start = ends
            .get(conversation)?
            .iter()
            .copied()
            .filter(|&e| e < end)
            .max()
            .map_or(i64::MIN, |e| e + CHARGE_GRACE_MS);
        let mine: Vec<&Charge> = charges
            .get(conversation)?
            .iter()
            .filter(|c| c.ts > start && c.ts <= end + CHARGE_GRACE_MS)
            .collect();
        (!mine.is_empty()).then_some(mine)
    };
    let billed: HashMap<String, Vec<&Charge>> = turns
        .iter()
        .filter_map(|(key, turn)| Some((key.clone(), billed(turn)?)))
        .collect();

    let mut messages = Vec::new();
    for key in order {
        if let Some(prompt) = prompts.remove(&key)
            && let Some(date) = DateTime::<Utc>::from_timestamp_millis((prompt.ts * 1000.0) as i64)
        {
            let turn = turns.get(&key);
            let conversation = prompt
                .conversation_id
                .clone()
                .or_else(|| turn.and_then(|t| t.conversation_id.clone()))
                .unwrap_or_else(|| key.clone());
            let cwd = prompt
                .cwd
                .as_deref()
                .or(turn.and_then(|t| t.cwd.as_deref()));
            messages.push(ConversationMessage {
                application: Application::CursorAgent,
                date,
                project_hash: project_name(cwd),
                conversation_hash: hash_text(&conversation),
                local_hash: None,
                global_hash: hash_text(&format!("cursor-agent:{key}:user")),
                model: None,
                stats: Stats::default(),
                role: MessageRole::User,
                content: prompt.prompt_preview,
            });
        }
        if let Some(msg) = turns.remove(&key).and_then(|mut turn| {
            let date = DateTime::<Utc>::from_timestamp_millis((turn.ts * 1000.0) as i64)?;
            let mut model = model_name(turn.model.as_deref());
            if let Some(events) = billed.get(&key) {
                turn.input_tokens = events.iter().map(|c| c.input_tokens).sum();
                turn.output_tokens = events.iter().map(|c| c.output_tokens).sum();
                turn.cache_read_tokens = events.iter().map(|c| c.cache_read_tokens).sum();
                turn.cache_write_tokens = events.iter().map(|c| c.cache_write_tokens).sum();
                // Cursor names the model it billed unless it chose one itself (`default`).
                let billed_model = model_name(Some(&events[0].model));
                if billed_model != AUTO_MODEL && events.iter().all(|c| c.model == events[0].model) {
                    model = billed_model;
                }
            }
            let cost = if let Some(events) = billed.get(&key) {
                events.iter().map(|c| c.cents).sum::<f64>() / 100.0
            } else if model == AUTO_MODEL {
                0.0
            } else {
                calculate_total_cost(
                    &model,
                    turn.input_tokens,
                    turn.output_tokens,
                    turn.cache_write_tokens,
                    turn.cache_read_tokens,
                )
            };
            let project = project_name(turn.cwd.as_deref());
            Some(ConversationMessage {
                application: Application::CursorAgent,
                date,
                project_hash: project,
                conversation_hash: hash_text(turn.conversation_id.as_deref().unwrap_or(&key)),
                local_hash: None,
                global_hash: hash_text(&format!("cursor-agent:{key}")),
                model: Some(model),
                stats: Stats {
                    input_tokens: turn.input_tokens,
                    output_tokens: turn.output_tokens,
                    cache_read_tokens: turn.cache_read_tokens,
                    cache_creation_tokens: turn.cache_write_tokens,
                    cached_tokens: turn.cache_read_tokens,
                    cost,
                    ..Default::default()
                },
                role: MessageRole::Assistant,
                content: turn.reply_preview,
            })
        }) {
            messages.push(msg);
        }
    }
    messages
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

pub struct CursorAgentAnalyzer {
    usage_file: Option<PathBuf>,
    charges_file: Option<PathBuf>,
    /// Per-file parse cache (BZL-74). Each usage log is stamped together with the charges file,
    /// because a message's cost comes from the charges index: when that moves, every entry has to
    /// be priced again.
    cache: crate::stamp_cache::StampCache,
}

impl CursorAgentAnalyzer {
    pub fn new() -> Self {
        Self {
            usage_file: std::env::home_dir().map(|home| {
                home.join(".config")
                    .join("splitrail")
                    .join("cursor")
                    .join("usage.jsonl")
            }),
            charges_file: crate::cursor_limits::charges_file(),
            cache: crate::stamp_cache::StampCache::new(),
        }
    }
}

impl Default for CursorAgentAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Analyzer for CursorAgentAnalyzer {
    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        // The charges file is watched too, so fetched costs show up without a new turn.
        self.usage_file
            .iter()
            .chain(self.charges_file.iter())
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
        let charges = self
            .charges_file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|content| index_charges(&content))
            .unwrap_or_default();

        // The charges file is a source of prices, not of messages.
        let logs: Vec<DataSource> = sources
            .into_iter()
            .filter(|s| Some(&s.path) != self.charges_file.as_ref())
            .collect();

        // Only logs whose own file, or the charges file they are priced against, moved are
        // re-parsed (BZL-74).
        let charges_path = self.charges_file.clone();
        let (messages, _report) = self.cache.parse_all_stamping(
            &logs,
            |_| charges_path.iter().cloned().collect(),
            |path| {
                let content = std::fs::read_to_string(path)?;
                Ok(parse_usage_log(&content, path, &charges))
            },
        );
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
    fn one_message_per_generation_from_both_hooks() {
        let log = concat!(
            r#"{"ts":1791945999.0,"hook_event_name":"beforeSubmitPrompt","conversation_id":"c1","generation_id":"g1","prompt_preview":"fix the bug"}"#,
            "\n",
            r#"{"ts":1791946000.0,"hook_event_name":"afterAgentResponse","reply_preview":"Fixed it.","conversation_id":"c1","generation_id":"g1","model":"claude-4.5-sonnet","input_tokens":1200,"output_tokens":300,"cache_read_tokens":50000,"cache_write_tokens":10,"cwd":"/Users/x/proj","user_email":"me@x.com"}"#,
            "\n",
            r#"{"ts":1791946000.4,"hook_event_name":"stop","conversation_id":"c1","generation_id":"g1","model":"claude-4.5-sonnet","status":"completed","input_tokens":1200,"output_tokens":300,"cache_read_tokens":50000,"cache_write_tokens":10}"#,
            "\n",
            "{\"ts\":17919\n", // A torn write.
            r#"{"ts":1791946100.0,"hook_event_name":"stop","conversation_id":"c1","generation_id":"g2","model":"default","status":"aborted","input_tokens":5,"output_tokens":1}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/u.jsonl"), &ChargeIndex::new());
        assert_eq!(messages.len(), 3);

        let prompt = &messages[0];
        assert_eq!(prompt.role, MessageRole::User);
        assert_eq!(prompt.content.as_deref(), Some("fix the bug"));
        assert_eq!(prompt.project_hash, "proj");
        let messages = &messages[1..];

        let first = &messages[0];
        assert_eq!(first.content.as_deref(), Some("Fixed it."));
        assert_eq!(first.application, Application::CursorAgent);
        assert_eq!(first.role, MessageRole::Assistant);
        assert_eq!(first.stats.input_tokens, 1200); // Not doubled by the stop hook.
        assert_eq!(first.stats.output_tokens, 300);
        assert_eq!(first.stats.cache_read_tokens, 50000);
        assert_eq!(first.stats.cache_creation_tokens, 10);
        assert_eq!(first.project_hash, "proj");
        assert_eq!(first.date.timestamp_millis(), 1_791_946_000_400);

        // Cursor picked the model: counted, but not priced.
        assert_eq!(messages[1].model.as_deref(), Some(AUTO_MODEL));
        assert_eq!(messages[1].stats.cost, 0.0);
        assert_eq!(first.conversation_hash, messages[1].conversation_hash);
        assert_ne!(first.global_hash, messages[1].global_hash);

        assert_eq!(
            crate::plan_limits::login_for_analyzer(DISPLAY_NAME).as_deref(),
            Some("me")
        );
        assert_eq!(model_name(Some("Auto")), AUTO_MODEL);
        assert_eq!(model_name(None), AUTO_MODEL);
        assert_eq!(model_name(Some("gpt-5")), "gpt-5");
    }
}

#[cfg(test)]
mod charge_tests {
    use super::*;

    #[test]
    fn billed_events_replace_a_turns_tokens_and_cost() {
        let log = concat!(
            r#"{"ts":1000.0,"hook_event_name":"print","conversation_id":"c1","generation_id":"g1","model":"Auto","input_tokens":100,"output_tokens":10}"#,
            "\n",
            r#"{"ts":2000.0,"hook_event_name":"stop","conversation_id":"c1","generation_id":"g2","model":"default","input_tokens":50,"output_tokens":5}"#,
            "\n",
            r#"{"ts":3000.0,"hook_event_name":"stop","conversation_id":"c2","generation_id":"g3","model":"gpt-5","input_tokens":7,"output_tokens":1}"#,
            "\n",
        );
        let charge = |conversation: &str, ts: i64, input: u64, cents: f64, model: &str| {
            format!(
                r#"{{"conversation_id":"{conversation}","ts":{ts},"model":"{model}","input_tokens":{input},"output_tokens":1,"cache_read_tokens":0,"cache_write_tokens":0,"cents":{cents}}}"#
            )
        };
        let charges = index_charges(
            &[
                charge("c1", 400_000, 60, 1.5, "default"), // g1: part the hook saw
                charge("c1", 999_000, 90, 2.5, "default"), // g1: a request it did not report
                charge("c1", 1_500_000, 50, 1.0, "composer-2"), // g2
                charge("c9", 1_500_000, 999, 50.0, "default"), // Another client: ignored
            ]
            .join("\n"),
        );
        let messages = parse_usage_log(log, Path::new("/u.jsonl"), &charges);
        let [g1, g2, g3] = &messages[..] else {
            panic!("{messages:?}")
        };
        assert_eq!((g1.stats.input_tokens, g1.stats.output_tokens), (150, 2));
        assert!((g1.stats.cost - 0.04).abs() < 1e-9);
        assert_eq!(g1.model.as_deref(), Some(AUTO_MODEL));
        assert!((g2.stats.cost - 0.01).abs() < 1e-9);
        assert_eq!(g2.model.as_deref(), Some("composer-2")); // Billed under a named model.
        // No events for this turn: priced from the table as before.
        assert_eq!(g3.stats.input_tokens, 7);
        assert!(g3.stats.cost > 0.0);
    }
}

#[cfg(test)]
mod live {
    /// Reads the real hook log: `cargo test cursor_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn cursor_live_log_has_data() {
        use crate::analyzer::Analyzer;
        let stats = super::CursorAgentAnalyzer::new().get_stats().await.unwrap();
        for m in &stats.messages {
            println!(
                "{} {:?} in {} out {} cache {} cost {}",
                m.date,
                m.model,
                m.stats.input_tokens,
                m.stats.output_tokens,
                m.stats.cache_read_tokens,
                m.stats.cost
            );
        }
        println!(
            "conversations {} days {:?}",
            stats.num_conversations,
            stats.daily_stats.keys().collect::<Vec<_>>()
        );
        assert!(!stats.messages.is_empty());
    }
}

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

use super::cursor_transcripts;
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
        index.entry(charge.conversation_id.clone()).or_default().push(charge);
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
    duration_ms: Option<f64>,
}

fn model_name(model: Option<&str>) -> String {
    match model.map(str::trim) {
        // Hooks say `default`, print mode's stream says `Auto`.
        Some(m) if !m.is_empty() && !m.eq_ignore_ascii_case("default") && !m.eq_ignore_ascii_case("auto") => {
            m.to_string()
        }
        _ => AUTO_MODEL.to_string(),
    }
}

/// A turn as the hooks saw it: its conversation, when it started and, once over, when it ended.
#[derive(Debug, Clone)]
pub struct TurnWindow {
    pub conversation: String,
    pub started: f64,
    pub ended: Option<f64>,
    pub cwd: Option<String>,
    pub model: String,
}

/// Every turn in a hook log, oldest first.
pub fn turn_windows(content: &str) -> Vec<TurnWindow> {
    let mut by_key: HashMap<String, TurnWindow> = HashMap::new();
    let mut order = Vec::new();
    for line in content.lines() {
        let Ok(event) = serde_json::from_str::<HookLine>(line) else { continue };
        let (Some(key), Some(conversation)) = (event.generation_id.clone(), event.conversation_id.clone()) else {
            continue;
        };
        let prompt = event.hook_event_name.as_deref() == Some("beforeSubmitPrompt");
        // Print mode logs one line when the run is over, with how long it took.
        let started = event.ts - event.duration_ms.map_or(0.0, |ms| ms / 1000.0);
        let window = by_key.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            TurnWindow { conversation, started, ended: None, cwd: None, model: AUTO_MODEL.to_string() }
        });
        window.started = window.started.min(started);
        if window.cwd.is_none() {
            window.cwd = event.cwd;
        }
        if !prompt {
            window.ended = Some(window.ended.map_or(event.ts, |e| e.max(event.ts)));
            if event.model.is_some() {
                window.model = model_name(event.model.as_deref());
            }
        }
    }
    let mut windows: Vec<TurnWindow> = order.into_iter().filter_map(|k| by_key.remove(&k)).collect();
    windows.sort_by(|a, b| a.started.total_cmp(&b.started));
    windows
}

/// How long after its parent turn ends a subagent may still start and count as that turn's.
const SUBAGENT_SLACK_SECS: f64 = 5.0;

/// Which hooked conversation each subagent transcript belongs to: one in the same project folder
/// whose turn was running when the subagent started.
pub fn link_subagents(
    windows: &[TurnWindow],
    transcripts: &[cursor_transcripts::Transcript],
) -> HashMap<String, String> {
    let hooked: std::collections::HashSet<&str> = windows.iter().map(|w| w.conversation.as_str()).collect();
    // A conversation's folder: its own transcript's, else the one its working directory names.
    let mut slug_of: HashMap<&str, String> = HashMap::new();
    for t in transcripts.iter().filter(|t| hooked.contains(t.conversation_id.as_str())) {
        slug_of.insert(t.conversation_id.as_str(), t.project_slug.clone());
    }
    let mut links = HashMap::new();
    for t in transcripts.iter().filter(|t| !hooked.contains(t.conversation_id.as_str())) {
        let parent = windows
            .iter()
            .filter(|w| {
                let slug = slug_of
                    .get(w.conversation.as_str())
                    .cloned()
                    .or_else(|| w.cwd.as_deref().map(cursor_transcripts::project_slug));
                slug.as_deref() == Some(t.project_slug.as_str())
                    && w.started - SUBAGENT_SLACK_SECS <= t.started
                    && w.ended.is_none_or(|end| t.started <= end + SUBAGENT_SLACK_SECS)
            })
            .max_by(|a, b| a.started.total_cmp(&b.started));
        if let Some(parent) = parent {
            links.insert(t.conversation_id.clone(), parent.conversation.clone());
        }
    }
    links
}

/// The charges with each subagent's billed under its parent conversation, so they count in the
/// turn that started them instead of being left out as another client's.
pub fn charges_with_subagents(mut charges: ChargeIndex, links: &HashMap<String, String>) -> ChargeIndex {
    for (child, parent) in links {
        if let Some(moved) = charges.remove(child) {
            let list = charges.entry(parent.clone()).or_default();
            list.extend(moved);
            list.sort_by_key(|c| c.ts);
        }
    }
    charges
}

/// `Read ×3 · Grep`: tool calls in a row, by name in the order first used.
fn tools_summary(names: &[String]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for name in names {
        match counts.iter_mut().find(|(n, _)| *n == name.as_str()) {
            Some((_, count)) => *count += 1,
            None => counts.push((name.as_str(), 1)),
        }
    }
    let parts: Vec<String> = counts
        .into_iter()
        .map(|(name, count)| if count > 1 { format!("{name} ×{count}") } else { name.to_string() })
        .collect();
    format!("tools: {}", parts.join(" · "))
}

/// The transcript feed: one assistant message per note or run of tool calls, with no tokens
/// (those stay on the turn's own message). A hooked conversation's prompts are already the hook
/// log's; a subagent's request is shown as a note under its parent.
pub fn transcript_messages(
    transcripts: &[cursor_transcripts::Transcript],
    windows: &[TurnWindow],
    links: &HashMap<String, String>,
    charges: &ChargeIndex,
) -> Vec<ConversationMessage> {
    let latest: HashMap<&str, &TurnWindow> = windows.iter().map(|w| (w.conversation.as_str(), w)).collect();
    let mut messages = Vec::new();
    for t in transcripts {
        let subagent = links.get(&t.conversation_id);
        let conversation = subagent.unwrap_or(&t.conversation_id);
        let Some(window) = latest.get(conversation.as_str()) else { continue };
        // A subagent billed under a named model says which; otherwise its parent's.
        let model = charges
            .get(&t.conversation_id)
            .and_then(|events| events.first())
            .map(|c| model_name(Some(&c.model)))
            .filter(|m| m != AUTO_MODEL && subagent.is_some())
            .unwrap_or_else(|| window.model.clone());
        for (index, (at, item)) in t.items.iter().enumerate() {
            let content = match item {
                cursor_transcripts::Item::Prompt(_) if subagent.is_none() => continue,
                cursor_transcripts::Item::Prompt(text) => {
                    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                    format!("↳ subagent: {}", first.chars().take(200).collect::<String>())
                }
                cursor_transcripts::Item::Text(text) => text.clone(),
                cursor_transcripts::Item::Tools(names) => tools_summary(names),
            };
            let Some(date) = DateTime::<Utc>::from_timestamp_millis((at * 1000.0) as i64) else { continue };
            let tool_calls = match item {
                cursor_transcripts::Item::Tools(names) => names.len() as u32,
                _ => 0,
            };
            messages.push(ConversationMessage {
                application: Application::CursorAgent,
                date,
                project_hash: project_name(window.cwd.as_deref()),
                conversation_hash: hash_text(conversation),
                local_hash: None,
                global_hash: hash_text(&format!("cursor-agent:transcript:{}:{index}", t.conversation_id)),
                model: Some(model.clone()),
                stats: Stats { tool_calls, ..Default::default() },
                role: MessageRole::Assistant,
                content: Some(content),
            });
        }
    }
    messages
}

/// Whether the newest turn in a hook log started within `max_age_secs` of `now` and has not
/// ended, so billing is worth fetching more often.
pub fn turn_open(content: &str, now: f64, max_age_secs: f64) -> bool {
    turn_windows(content)
        .last()
        .is_some_and(|w| w.ended.is_none() && now - w.started <= max_age_secs)
}

/// Parse a hook log into one assistant message per generation. Malformed lines (a torn write)
/// are skipped. The newest login seen is recorded for the dashboard's Login row.
#[cfg(test)]
pub fn parse_usage_log(
    content: &str,
    source: &Path,
    charges: &ChargeIndex,
) -> Vec<ConversationMessage> {
    parse_usage_log_with(content, source, charges, &std::collections::HashSet::new())
}

/// [`parse_usage_log`], leaving the reply text off turns of the conversations in `transcribed`,
/// whose transcript already shows it. A turn still running gets a message of what has been
/// billed for it so far, so its tokens show before it ends.
pub fn parse_usage_log_with(
    content: &str,
    source: &Path,
    charges: &ChargeIndex,
    transcribed: &std::collections::HashSet<String>,
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
        if let Some(email) = event.user_email.as_deref().filter(|e| !e.is_empty()) {
            if login.as_ref().is_none_or(|(ts, _)| event.ts >= *ts) {
                login = Some((event.ts, email.split('@').next().unwrap_or(email).to_string()));
            }
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
    if let Some((ts, name)) = login {
        if let Some(at) = DateTime::<Utc>::from_timestamp_millis((ts * 1000.0) as i64) {
            crate::plan_limits::observe_login(DISPLAY_NAME, at, name);
        }
    }

    // Turn end times per conversation, to split its billed events between turns.
    let mut ends: HashMap<&str, Vec<i64>> = HashMap::new();
    for turn in turns.values() {
        if let Some(conversation) = turn.conversation_id.as_deref() {
            ends.entry(conversation).or_default().push((turn.ts * 1000.0) as i64);
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
        if let Some(prompt) = prompts.remove(&key) {
            if let Some(date) = DateTime::<Utc>::from_timestamp_millis((prompt.ts * 1000.0) as i64) {
                let turn = turns.get(&key);
                let conversation = prompt
                    .conversation_id
                    .clone()
                    .or_else(|| turn.and_then(|t| t.conversation_id.clone()))
                    .unwrap_or_else(|| key.clone());
                let cwd = prompt.cwd.as_deref().or(turn.and_then(|t| t.cwd.as_deref()));
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
                    content: prompt.prompt_preview.clone(),
                });
                if turn.is_none() {
                    messages.extend(running_turn(&prompt, &key, &conversation, cwd, charges));
                }
            }
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
            if turn.conversation_id.as_ref().is_some_and(|c| transcribed.contains(c)) {
                turn.reply_preview = None; // The transcript has it, at the time it was written.
            }
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

/// What has been billed so far for a turn that has not ended: one message at the newest charge,
/// replaced by the turn's own message once its `stop` hook fires.
fn running_turn(
    prompt: &HookLine,
    key: &str,
    conversation: &str,
    cwd: Option<&str>,
    charges: &ChargeIndex,
) -> Option<ConversationMessage> {
    let since = (prompt.ts * 1000.0) as i64 - CHARGE_GRACE_MS;
    let events: Vec<&Charge> = charges.get(conversation)?.iter().filter(|c| c.ts >= since).collect();
    let last = events.iter().map(|c| c.ts).max()?;
    let model = model_name(Some(&events[0].model));
    let model = if events.iter().all(|c| c.model == events[0].model) { model } else { AUTO_MODEL.to_string() };
    let tokens = |f: fn(&Charge) -> u64| events.iter().map(|c| f(c)).sum::<u64>();
    Some(ConversationMessage {
        application: Application::CursorAgent,
        date: DateTime::<Utc>::from_timestamp_millis(last)?,
        project_hash: project_name(cwd),
        conversation_hash: hash_text(conversation),
        local_hash: None,
        global_hash: hash_text(&format!("cursor-agent:{key}")),
        model: Some(model),
        stats: Stats {
            input_tokens: tokens(|c| c.input_tokens),
            output_tokens: tokens(|c| c.output_tokens),
            cache_read_tokens: tokens(|c| c.cache_read_tokens),
            cache_creation_tokens: tokens(|c| c.cache_write_tokens),
            cached_tokens: tokens(|c| c.cache_read_tokens),
            cost: events.iter().map(|c| c.cents).sum::<f64>() / 100.0,
            ..Default::default()
        },
        role: MessageRole::Assistant,
        content: None,
    })
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
    /// Cursor's own transcripts, read again only when a file changed. The hook log (tens of KB)
    /// and the charges index are re-read on every pass: a turn's messages depend on both and on
    /// the transcripts, so caching any one alone would serve stale links.
    transcripts: cursor_transcripts::TranscriptReader,
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
            transcripts: cursor_transcripts::TranscriptReader::for_home(),
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
        // The charges file is watched too, so fetched costs show up without a new turn, and the
        // transcripts so the feed moves while a turn runs.
        self.usage_file
            .iter()
            .chain(self.charges_file.iter())
            .map(|path| path.to_string_lossy().into_owned())
            .chain(self.transcripts.glob_pattern())
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
            .filter(|s| Some(&s.path) == self.usage_file.as_ref())
            .collect();

        let mut messages = Vec::new();
        for log in logs {
            let Ok(content) = std::fs::read_to_string(&log.path) else { continue };
            let windows = turn_windows(&content);
            let hooked: std::collections::HashSet<String> =
                windows.iter().map(|w| w.conversation.clone()).collect();
            let mut folders = self.transcripts.folders_of(&hooked);
            folders.extend(windows.iter().filter_map(|w| w.cwd.as_deref().map(cursor_transcripts::project_slug)));
            let transcripts = self
                .transcripts
                .read(|slug, conversation| hooked.contains(conversation) || folders.contains(slug));
            let links = link_subagents(&windows, &transcripts);
            let transcribed: std::collections::HashSet<String> = transcripts
                .iter()
                .filter(|t| hooked.contains(&t.conversation_id))
                .map(|t| t.conversation_id.clone())
                .collect();
            let feed = transcript_messages(&transcripts, &windows, &links, &charges);
            let charges = charges_with_subagents(charges.clone(), &links);
            messages.extend(parse_usage_log_with(&content, &log.path, &charges, &transcribed));
            messages.extend(feed);
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
        let [g1, g2, g3] = &messages[..] else { panic!("{messages:?}") };
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
mod transcript_tests {
    use super::*;
    use cursor_transcripts::{Item, Transcript};

    fn charge(conversation: &str, ts: i64, output: u64, model: &str) -> String {
        format!(
            r#"{{"conversation_id":"{conversation}","ts":{ts},"model":"{model}","input_tokens":100,"output_tokens":{output},"cache_read_tokens":0,"cache_write_tokens":0,"cents":1.0}}"#
        )
    }

    #[test]
    fn a_running_turn_shows_its_transcript_subagents_and_billed_tokens() {
        // The prompt went in at 1000 s; no stop hook yet.
        let log = r#"{"ts":1000.0,"hook_event_name":"beforeSubmitPrompt","conversation_id":"main","generation_id":"g1","model":"default","prompt_preview":"review this","cwd":"/Users/x/proj"}"#;
        let windows = turn_windows(log);
        assert!(turn_open(log, 1600.0, 7200.0));
        assert!(!turn_open(log, 1000.0 + 3.0 * 3600.0, 7200.0)); // Abandoned long ago.

        let transcripts = vec![
            Transcript {
                conversation_id: "main".into(),
                project_slug: "Users-x-proj".into(),
                started: 1000.0,
                items: vec![(1000.5, Item::Prompt("review this".into())), (1010.0, Item::Tools(vec!["Read".into(), "Read".into(), "Grep".into()]))],
            },
            Transcript {
                conversation_id: "sub".into(),
                project_slug: "Users-x-proj".into(),
                started: 1020.0,
                items: vec![(1020.0, Item::Prompt("Perform a security review\nof everything".into())), (1100.0, Item::Text("Summary: fine".into()))],
            },
            // Another folder, and a conversation from before the turn: not this turn's.
            Transcript { conversation_id: "other".into(), project_slug: "Users-x-else".into(), started: 1020.0, items: vec![(1020.0, Item::Text("x".into()))] },
            Transcript { conversation_id: "old".into(), project_slug: "Users-x-proj".into(), started: 500.0, items: vec![(500.0, Item::Text("y".into()))] },
        ];
        let links = link_subagents(&windows, &transcripts);
        assert_eq!(links.len(), 1);
        assert_eq!(links.get("sub").map(String::as_str), Some("main"));

        let charges = index_charges(&[charge("main", 1_005_000, 40, "default"), charge("sub", 1_030_000, 60, "composer-2.5-fast")].join("\n"));
        let feed = transcript_messages(&transcripts, &windows, &links, &charges);
        let lines: Vec<&str> = feed.iter().filter_map(|m| m.content.as_deref()).collect();
        // The main prompt is the hook's; the subagent's request is a note under it.
        assert_eq!(lines, vec!["tools: Read ×2 · Grep", "↳ subagent: Perform a security review", "Summary: fine"]);
        assert_eq!(feed[0].stats.tool_calls, 3);
        assert!(feed.iter().all(|m| m.stats.output_tokens == 0 && m.conversation_hash == hash_text("main")));
        assert_eq!(feed[2].model.as_deref(), Some("composer-2.5-fast"));

        // Billing so far, the subagent's included, shows as the running turn's tokens.
        let charges = charges_with_subagents(charges, &links);
        let transcribed = std::collections::HashSet::from(["main".to_string()]);
        let messages = parse_usage_log_with(log, Path::new("/u.jsonl"), &charges, &transcribed);
        let [prompt, running] = &messages[..] else { panic!("{messages:?}") };
        assert_eq!(prompt.role, MessageRole::User);
        assert_eq!(running.stats.output_tokens, 100);
        assert_eq!(running.date.timestamp(), 1030);
        assert!((running.stats.cost - 0.02).abs() < 1e-9);

        // Once the turn ends, its own message replaces the running one, without the reply text
        // the transcript already shows.
        let ended = format!("{log}\n{}", r#"{"ts":1200.0,"hook_event_name":"stop","conversation_id":"main","generation_id":"g1","model":"default","output_tokens":40,"reply_preview":"Summary: fine"}"#);
        let messages = parse_usage_log_with(&ended, Path::new("/u.jsonl"), &charges, &transcribed);
        let [_, turn] = &messages[..] else { panic!("{messages:?}") };
        assert_eq!(turn.global_hash, running.global_hash);
        assert_eq!(turn.stats.output_tokens, 100);
        assert_eq!(turn.content, None);
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
            println!("{} {:?} in {} out {} cache {} cost {} {:?}", m.date, m.model, m.stats.input_tokens, m.stats.output_tokens, m.stats.cache_read_tokens, m.stats.cost, m.content.as_deref().map(|c| c.chars().take(70).collect::<String>()));
        }
        println!("conversations {} days {:?}", stats.num_conversations, stats.daily_stats.keys().collect::<Vec<_>>());
        assert!(!stats.messages.is_empty());
    }
}

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use simd_json::prelude::*;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::analyzer::{Analyzer, DataSource};
use crate::incremental::{ChunkResult, IncrementalJsonlCache, RefreshReport};
use crate::types::{
    AgenticCodingToolStats, Application, ConversationMessage, DailyStats, MessageRole, Stats,
};
use crate::utils::hash_text;
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct ClaudeCodeAnalyzer {
    /// The Claude Code config directory to read (`~/.claude` unless this is a second account).
    config_dir: Option<PathBuf>,
    /// Display name; distinct per account so each gets its own dashboard column (BZL-64).
    name: &'static str,
    /// Per-file parse cache so a reload only parses bytes appended since the last one. It holds
    /// cursors, not contents.
    cache: IncrementalJsonlCache<JsonlParseState>,
    /// Everything parsed so far, folded into per-day aggregates. Grows with days and distinct
    /// messages, never with the size of the corpus (BZL-14).
    accum: std::sync::Mutex<Accumulator>,
}

/// The models this Mac's Ollama has served, from its usage proxy's log, re-read only when the
/// log changes.
fn local_ollama_models() -> std::sync::Arc<HashSet<String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<(Option<std::time::SystemTime>, std::sync::Arc<HashSet<String>>)>> =
        std::sync::OnceLock::new();
    let Some(path) = std::env::home_dir().map(|h| h.join(".config/splitrail/ollama/usage.jsonl")) else {
        return Default::default();
    };
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(Default::default);
    let mut cached = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cached.0 != modified || modified.is_none() {
        let models: HashSet<String> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                // `"model": "…"` without parsing the whole line.
                let start = l.find("\"model\": \"")? + 10;
                let end = l[start..].find('"')? + start;
                Some(l[start..end].to_string())
            })
            .collect();
        *cached = (modified, std::sync::Arc::new(models));
    }
    cached.1.clone()
}

/// When the account's config directory was on which login, from the status line capture's
/// history (`~/.config/splitrail/logins/<label>.jsonl`, one line per change, BZL-283). `/login`
/// switches the whole directory, so the login at a time is the latest change before it.
#[derive(Default)]
struct LoginTimeline {
    changes: Vec<(f64, String)>,
}

impl LoginTimeline {
    fn load(label: Option<&str>) -> Self {
        let Some(path) = label.and_then(|l| Some(std::env::home_dir()?.join(format!(".config/splitrail/logins/{l}.jsonl")))) else {
            return Self::default();
        };
        Self::parse(&std::fs::read_to_string(path).unwrap_or_default())
    }

    fn parse(content: &str) -> Self {
        let mut changes: Vec<(f64, String)> = content
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| Some((v.get("t")?.as_f64()?, v.get("login")?.as_str()?.to_string())))
            .collect();
        changes.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { changes }
    }

    /// The login at `date`; empty before the history starts.
    fn at(&self, date: chrono::DateTime<Utc>) -> &str {
        let t = date.timestamp_millis() as f64 / 1000.0;
        let index = self.changes.partition_point(|(at, _)| *at <= t);
        index.checked_sub(1).map_or("", |i| self.changes[i].1.as_str())
    }
}

/// Fold state for one `local_hash`. Claude Code writes a single assistant turn as several JSONL
/// rows; dedup folds them back together. Their `usage` is a cumulative snapshot of the one billed
/// request (input and cache repeat, output only grows, the last row is the bill), so the tokens
/// are the largest seen, never the sum (BZL-132, upstream #222). Activity counted from each row's
/// own content still adds up. Those rows can land in different refreshes, so the fold has to
/// survive between them.
struct FoldState {
    /// Token tuples already folded in. A repeat means a redundant row, not a new part.
    fingerprints: HashSet<TokenFingerprint>,
    /// Stats currently attributed to this message.
    stats: Stats,
    /// The day its contribution was added to, so corrections land on the same day.
    day: String,
    model: Option<String>,
    /// Its slice of that day (BZL-138), so corrections land there too.
    hour: u8,
    project: String,
    /// The login it ran under (BZL-283), so corrections land on its slice.
    account: String,
}

impl FoldState {
    /// Move the day's slice for this message by a correction.
    fn add_to_slice(&self, day: &mut DailyStats, delta: &Stats, cost: f64) {
        if let Some(model) = &self.model {
            let slice = crate::utils::slice_mut_for(day, self.hour, model, &self.project, &self.account);
            crate::utils::add_to_slice(slice, delta, cost, 0);
        }
    }
}

#[derive(Default)]
struct Accumulator {
    /// Per-day sums, before finalisation.
    days: BTreeMap<String, DailyStats>,
    conversation_start: BTreeMap<String, String>,
    /// Per-day message timestamps, reduced to `active_seconds` at finalisation. Eight bytes a
    /// message, so keeping all of history here costs about a megabyte.
    timestamps_by_day: BTreeMap<String, Vec<i64>>,
    folded: HashMap<String, FoldState>,
    /// Raw messages inside the live window, for the views that need individual messages.
    recent: Vec<ConversationMessage>,
}

impl Accumulator {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn day_of(message: &ConversationMessage) -> String {
        message
            .date
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string()
    }

    /// Fold one freshly parsed message in, applying the same dedup rules as `dedup_messages`.
    fn fold(&mut self, message: &ConversationMessage, account: &str) {
        let date = Self::day_of(message);

        self.conversation_start
            .entry(message.conversation_hash.clone())
            .and_modify(|existing| {
                if date < *existing {
                    *existing = date.clone();
                }
            })
            .or_insert_with(|| date.clone());

        self.recent.push(message.clone());

        let fp = (
            message.stats.input_tokens,
            message.stats.output_tokens,
            message.stats.cache_creation_tokens,
            message.stats.cache_read_tokens,
            message.stats.cached_tokens,
        );

        if let Some(local_hash) = message.local_hash.clone()
            && let Some(state) = self.folded.get_mut(&local_hash)
        {
            let Some(day) = self.days.get_mut(&state.day) else {
                return;
            };
            if state.fingerprints.contains(&fp) {
                // A redundant repeat of a row already folded: keep the richest activity counts,
                // and move the day by exactly what changed.
                let delta = max_activity_delta(&state.stats, &message.stats);
                apply_max_activity(&mut state.stats, &message.stats);
                crate::utils::add_stats_delta_into_day(day, &delta, 0.0);
                state.add_to_slice(day, &delta, 0.0);
            } else {
                // Another snapshot of the same request: tokens rise to its larger counts, its own
                // activity adds, and the cost is recomputed from the tokens.
                state.fingerprints.insert(fp);
                let previous_cost = state.stats.cost;
                let mut delta = max_tokens_delta(&state.stats, &message.stats);
                add_activity(&mut delta, &message.stats);
                add_stats(&mut state.stats, &delta);
                if let Some(model) = &state.model {
                    state.stats.cost = crate::models::calculate_total_cost_with_cache_ttl(
                        model,
                        state.stats.input_tokens,
                        state.stats.output_tokens,
                        state.stats.cache_creation_tokens,
                        state.stats.cache_creation_1h_tokens,
                        state.stats.cache_read_tokens,
                    );
                }
                crate::utils::add_stats_delta_into_day(day, &delta, state.stats.cost - previous_cost);
                state.add_to_slice(day, &delta, state.stats.cost - previous_cost);
            }
            return;
        }

        // First time we have seen this message: count it.
        let day = self.days.entry(date.clone()).or_insert_with(|| DailyStats {
            date: date.clone(),
            ..Default::default()
        });
        crate::utils::fold_entry_into_day_for(day, message, account);
        self.timestamps_by_day
            .entry(date.clone())
            .or_default()
            .push(message.date.timestamp());

        if let Some(local_hash) = message.local_hash.clone() {
            let mut fingerprints = HashSet::new();
            fingerprints.insert(fp);
            self.folded.insert(
                local_hash,
                FoldState {
                    fingerprints,
                    stats: message.stats.clone(),
                    day: date,
                    model: message.model.clone(),
                    hour: crate::utils::local_hour(&message.date),
                    project: message.project_hash.clone(),
                    account: account.to_string(),
                },
            );
        }
    }

    /// Drop raw messages that have aged out. Their contribution is already in `days`.
    fn trim_recent(&mut self, cutoff: chrono::DateTime<Utc>) {
        if self.recent.iter().any(|m| m.date < cutoff) {
            self.recent.retain(|m| m.date >= cutoff);
            self.recent.shrink_to_fit();
        }
    }

    /// The live window as one message per reply: the first row of each, carrying the stats
    /// folded from all its rows. `recent` keeps every row, and summing those counted a reply
    /// once per content block (BZL-192).
    fn live_messages(&self) -> Vec<ConversationMessage> {
        let mut seen = HashSet::new();
        self.recent
            .iter()
            .filter(|m| m.local_hash.as_ref().is_none_or(|h| seen.insert(h.as_str())))
            .map(|m| {
                let mut m = m.clone();
                if let Some(state) = m.local_hash.as_ref().and_then(|h| self.folded.get(h)) {
                    m.stats = state.stats.clone();
                }
                m
            })
            .collect()
    }

    fn daily_stats(&self) -> BTreeMap<String, DailyStats> {
        let mut timestamps = self.timestamps_by_day.clone();
        crate::utils::finalize_daily(self.days.clone(), &self.conversation_start, &mut timestamps)
    }
}

/// How much each activity counter would rise if `from` were merged in at its maximum.
fn max_activity_delta(current: &Stats, from: &Stats) -> Stats {
    let mut delta = Stats::default();
    delta.tool_calls = from.tool_calls.saturating_sub(current.tool_calls);
    delta.files_read = from.files_read.saturating_sub(current.files_read);
    delta.files_edited = from.files_edited.saturating_sub(current.files_edited);
    delta.files_added = from.files_added.saturating_sub(current.files_added);
    delta.terminal_commands = from.terminal_commands.saturating_sub(current.terminal_commands);
    delta.file_searches = from.file_searches.saturating_sub(current.file_searches);
    delta.file_content_searches = from
        .file_content_searches
        .saturating_sub(current.file_content_searches);
    delta.todo_writes = from.todo_writes.saturating_sub(current.todo_writes);
    delta.todo_reads = from.todo_reads.saturating_sub(current.todo_reads);
    delta.todos_created = from.todos_created.saturating_sub(current.todos_created);
    delta.todos_completed = from.todos_completed.saturating_sub(current.todos_completed);
    delta.todos_in_progress = from
        .todos_in_progress
        .saturating_sub(current.todos_in_progress);
    delta
}

fn apply_max_activity(into: &mut Stats, from: &Stats) {
    into.tool_calls = into.tool_calls.max(from.tool_calls);
    into.files_read = into.files_read.max(from.files_read);
    into.files_edited = into.files_edited.max(from.files_edited);
    into.files_added = into.files_added.max(from.files_added);
    into.terminal_commands = into.terminal_commands.max(from.terminal_commands);
    into.file_searches = into.file_searches.max(from.file_searches);
    into.file_content_searches = into.file_content_searches.max(from.file_content_searches);
    into.todo_writes = into.todo_writes.max(from.todo_writes);
    into.todo_reads = into.todo_reads.max(from.todo_reads);
    into.todos_created = into.todos_created.max(from.todos_created);
    into.todos_completed = into.todos_completed.max(from.todos_completed);
    into.todos_in_progress = into.todos_in_progress.max(from.todos_in_progress);
}

/// How much each token count rises if `from`, a later snapshot of the same request, is taken at
/// its maximum.
fn max_tokens_delta(current: &Stats, from: &Stats) -> Stats {
    Stats {
        input_tokens: from.input_tokens.saturating_sub(current.input_tokens),
        output_tokens: from.output_tokens.saturating_sub(current.output_tokens),
        cache_creation_tokens: from.cache_creation_tokens.saturating_sub(current.cache_creation_tokens),
        cache_read_tokens: from.cache_read_tokens.saturating_sub(current.cache_read_tokens),
        cached_tokens: from.cached_tokens.saturating_sub(current.cached_tokens),
        ..Default::default()
    }
}

/// `from`'s activity counters (not its tokens) added into `into`.
fn add_activity(into: &mut Stats, from: &Stats) {
    into.tool_calls += from.tool_calls;
    into.files_read += from.files_read;
    into.files_edited += from.files_edited;
    into.files_added += from.files_added;
    into.terminal_commands += from.terminal_commands;
    into.file_searches += from.file_searches;
    into.file_content_searches += from.file_content_searches;
    into.todo_writes += from.todo_writes;
    into.todo_reads += from.todo_reads;
    into.todos_created += from.todos_created;
    into.todos_completed += from.todos_completed;
    into.todos_in_progress += from.todos_in_progress;
}

/// Add a delta's tokens and activity into a message's stats.
fn add_stats(into: &mut Stats, from: &Stats) {
    into.input_tokens += from.input_tokens;
    into.output_tokens += from.output_tokens;
    into.cache_creation_tokens += from.cache_creation_tokens;
    into.cache_creation_1h_tokens += from.cache_creation_1h_tokens;
    into.cache_read_tokens += from.cache_read_tokens;
    into.cached_tokens += from.cached_tokens;
    into.tool_calls += from.tool_calls;
    into.files_read += from.files_read;
    into.files_edited += from.files_edited;
    into.files_added += from.files_added;
    into.terminal_commands += from.terminal_commands;
    into.file_searches += from.file_searches;
    into.file_content_searches += from.file_content_searches;
    into.todo_writes += from.todo_writes;
    into.todo_reads += from.todo_reads;
    into.todos_created += from.todos_created;
    into.todos_completed += from.todos_completed;
    into.todos_in_progress += from.todos_in_progress;
}

impl Default for ClaudeCodeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeCodeAnalyzer {
    pub fn new() -> Self {
        Self {
            config_dir: std::env::home_dir().map(|home| home.join(".claude")),
            name: "Claude Code",
            cache: IncrementalJsonlCache::new(),
            accum: std::sync::Mutex::new(Accumulator::default()),
        }
    }

    /// An analyzer for one account's config directory. With a label it is shown as
    /// `Claude (<label>)`, kept short to fit a 13-column dashboard cell; without one it keeps the
    /// plain `Claude Code` name.
    pub fn for_account(label: Option<&str>, config_dir: PathBuf) -> Self {
        Self {
            config_dir: Some(config_dir),
            // Analyzers live for the whole process and there is one per account, so leaking the
            // name to satisfy `display_name(&self) -> &'static str` costs a few bytes once.
            name: match label {
                Some(label) => Box::leak(format!("Claude ({label})").into_boxed_str()),
                None => "Claude Code",
            },
            cache: IncrementalJsonlCache::new(),
            accum: std::sync::Mutex::new(Accumulator::default()),
        }
    }

    /// Bring the accumulator up to date with `sources` and report how much work that took.
    /// Returns the raw messages still inside the live window; history is in the accumulator.
    pub fn parse_conversations_with_report(
        &self,
        sources: &[DataSource],
    ) -> (Vec<ConversationMessage>, RefreshReport) {
        fn parse_chunk(
            path: &Path,
            reader: &mut BufReader<std::fs::File>,
            state: &mut JsonlParseState,
        ) -> ChunkResult {
            // Live files may end mid-line; leave that tail for the next refresh.
            parse_jsonl_chunk(path, reader, state, false)
        }

        let (mut chunks, mut report) = self.cache.refresh(sources, parse_chunk);
        let mut accum = self.accum.lock().unwrap_or_else(|e| e.into_inner());

        // A rewritten, shrunk, or vanished file invalidates what we folded from it, and a fold
        // cannot be undone selectively. Start over from a clean cache.
        if report.needs_rebuild {
            accum.clear();
            self.cache.clear();
            let (fresh, fresh_report) = self.cache.refresh(sources, parse_chunk);
            chunks = fresh;
            report = fresh_report;
            report.needs_rebuild = true;
        }

        // Which login the account's directory was on when each reply ran (BZL-283).
        let logins = LoginTimeline::load(crate::plan_limits::account_label(self.name).as_deref());
        let local = local_ollama_models();
        for chunk in &chunks {
            for message in chunk.iter() {
                // Claude Code pointed at this Mac's Ollama: the Ollama column has the request
                // (through its proxy, with its transcript and a price); counting it here too would
                // count it twice and put an open model in a Claude plan's numbers.
                if message.model.as_ref().is_some_and(|m| local.contains(m)) {
                    continue;
                }
                accum.fold(message, logins.at(message.date));
            }
        }
        accum.trim_recent(crate::utils::current_live_window_cutoff());
        (accum.live_messages(), report)
    }

    /// The accumulated per-day history.
    pub fn daily_stats(&self) -> BTreeMap<String, DailyStats> {
        self.accum
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .daily_stats()
    }
}

#[async_trait]
impl Analyzer for ClaudeCodeAnalyzer {
    fn display_name(&self) -> &'static str {
        self.name
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let Some(config_dir) = &self.config_dir else {
            return Vec::new();
        };
        let dir = config_dir.to_string_lossy();
        vec![
            format!("{dir}/projects/*/*.jsonl"),
            // Subagent transcripts live one level down, beside their parent session (BZL-63).
            format!("{dir}/projects/*/*/subagents/*.jsonl"),
        ]
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        for pattern in patterns {
            for entry in glob::glob(&pattern)? {
                let path = entry?;
                if path.is_file() {
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
        // Files are parsed in parallel inside the cache; unchanged files are not opened.
        let (messages, _report) = self.parse_conversations_with_report(&sources);
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        // Folds new bytes into the accumulator and hands back the live window.
        let messages = self.parse_conversations(sources).await?;
        let mut daily_stats = self.daily_stats();

        // Remove any remaining "unknown" entries from daily_stats
        daily_stats.retain(|date, _| date != "unknown");

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

// Claude Code specific implementation functions

/// For a subagent transcript (`projects/{PROJECT_ID}/{session_uuid}/subagents/agent-*.jsonl`),
/// the path of the session transcript that spawned it (`projects/{PROJECT_ID}/{session_uuid}.jsonl`).
pub fn subagent_parent_transcript(file_path: &Path) -> Option<PathBuf> {
    let subagents_dir = file_path.parent()?;
    if subagents_dir.file_name()? != "subagents" {
        return None;
    }
    let session_dir = subagents_dir.parent()?;
    let session_id = session_dir.file_name()?.to_str()?;
    Some(session_dir.parent()?.join(format!("{session_id}.jsonl")))
}

// Helper function to extract project ID from Claude Code file path and hash it
pub fn extract_project_id(file_path: &Path) -> String {
    // Claude Code path format: ~/.claude/projects/{PROJECT_ID}/{conversation_uuid}.jsonl
    // Subagents resolve through their parent session so they land in the same project.
    if let Some(parent_transcript) = subagent_parent_transcript(file_path) {
        return extract_project_id(&parent_transcript);
    }

    if let Some(parent) = file_path.parent()
        && let Some(project_id) = parent.file_name().and_then(|name| name.to_str())
    {
        return project_id.to_string();
    }

    // Fallback: return empty string if we can't extract project ID
    "".to_string()
}

// CLAUDE CODE JSONL FILES SCHEMA

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")] // Inconsistently, this data is snake_case in the JSONL files.
pub enum ContentBlock {
    ToolUse {
        id: String,
        name: String,
        input: simd_json::OwnedValue, // e.g. "toolu_01K7hbuwktKtti8mQb1wH2q8"
    },
    ToolResult {
        tool_use_id: String, // e.g. "toolu_01K7hbuwktKtti8mQb1wH2q8"
        content: Content,    // e.g. "Found 4 files\nC:\\..."
    },
    Text {
        text: serde_bytes::ByteBuf,
    },
    Thinking {
        thinking: serde_bytes::ByteBuf,
        signature: String,
    },
    Image {
        source: ImageSource,
    },
    /// Any block type this parser does not model (`tool_reference` inside ToolSearch results,
    /// `fallback`, `server_tool_use`, ...). Without this, one unfamiliar block rejects the
    /// entire JSONL line, including its token usage.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    String(serde_bytes::ByteBuf),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    /// Breakdown of `cache_creation_input_tokens` by entry lifetime. Absent on older sessions,
    /// in which case everything is treated as the 5-minute default.
    #[serde(default)]
    pub cache_creation: Option<CacheCreation>,
}

/// Anthropic reports cache writes split by TTL; the 1-hour tier costs more (BZL-136).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheCreation {
    #[serde(default)]
    pub ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    pub ephemeral_1h_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// This does NOT get renamed to camelCase, inconsistently enough.
struct Message {
    id: Option<String>,
    r#type: Option<String>, // "message"
    role: Option<String>,   // "assistant" or "user"
    model: Option<String>,  // e.g. "claude-sonnet-4-20250514"
    content: Option<Content>,
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
    usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCodeSummaryEntry {
    summary: String,
    leaf_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCodeFileHistorySnapshotEntry {
    #[serde(flatten)]
    fields: simd_json::OwnedValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCodeMessageEntry {
    r#type: Option<String>,      // "assistant" or "user"
    parent_uuid: Option<String>, // e.g. "773f9fdc-51ed-41cc-b107-19e5418bcf13"
    is_sidechain: Option<bool>,
    user_type: Option<String>,  // e.g. "external"
    cwd: Option<String>,        // e.g. "C:\test"
    session_id: Option<String>, // e.g. "92a07d6b-b12d-40d7-b184-aa04762ba0d6"
    version: Option<String>,    // e.g. "1.0.61"
    message: Option<Message>,
    tool_use_result: Option<simd_json::OwnedValue>, // For user messages only.
    request_id: Option<String>,                     // e.g. "req_0191C3ttfWOg3zRCDNdSFGv3"
    uuid: String,                                   // e.g. "a6ae4765-8274-4d00-8433-4fb28f4b387b"
    timestamp: DateTime<Utc>,                       // e.g. "2025-07-12T22:12:00.572Z"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCodeQueueOperationEntry {
    operation: String, // "enqueue" or "dequeue"
    timestamp: DateTime<Utc>,
    content: Option<String>,
    session_id: String,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClaudeCodeEntry {
    #[serde(alias = "summary")]
    Summary(ClaudeCodeSummaryEntry),
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot(ClaudeCodeFileHistorySnapshotEntry),
    #[serde(alias = "user", alias = "assistant", alias = "system")]
    Message(ClaudeCodeMessageEntry),
    #[serde(rename = "queue-operation")]
    QueueOperation(ClaudeCodeQueueOperationEntry),
    /// Any other record type Claude Code writes (attachment, cost-state, last-prompt, mode,
    /// permission-mode, custom-title, worktree-state, ...). None of these carry token usage,
    /// and new ones appear with most Claude Code releases, so they are skipped silently
    /// instead of being reported as invalid entries.
    #[serde(other)]
    Other,
}

pub mod tool_schema {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TodoWriteInputTodo {
        pub id: String,       // e.g. "1"
        pub title: String,    // e.g. "Explore current directory structure"
        pub status: String,   // e.g. "completed"
        pub priority: String, // e.g. "high"
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[allow(dead_code)]
    pub struct TodoWriteInput {
        pub todos: Vec<TodoWriteInputTodo>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[allow(dead_code)]
    pub struct TodoWriteResultTodo {
        pub content: String,
        pub status: String,
        pub priority: String,
        pub id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct TodoWriteResult {
        pub old_todos: Vec<TodoWriteInputTodo>,
        pub new_todos: Vec<TodoWriteInputTodo>,
    }
}

pub fn extract_tool_stats(
    message_content: &Content,
    tool_use_result: &Option<simd_json::OwnedValue>,
) -> Stats {
    let mut stats = Stats::default();

    if let Content::Blocks(blocks) = message_content {
        for block in blocks {
            let tool_name = match block {
                ContentBlock::ToolUse { name, .. } => name,
                _ => continue,
            };

            match tool_name.as_str() {
                "Read" => stats.files_read += 1,
                "Edit" | "MultiEdit" => stats.files_edited += 1,
                "Write" => stats.files_added += 1,
                "Bash" => stats.terminal_commands += 1,
                "Glob" => stats.file_searches += 1,
                "Grep" => stats.file_content_searches += 1,
                "TodoWrite" => stats.todo_writes += 1,
                "TodoRead" => stats.todo_reads += 1,
                _ => {}
            }
        }
    }

    if let Some(tool_result) = &tool_use_result
        && let Ok(todo_write_result) =
            simd_json::serde::from_owned_value::<tool_schema::TodoWriteResult>(tool_result.clone())
    {
        let old_todos = todo_write_result.old_todos;
        let new_todos = todo_write_result.new_todos;

        if new_todos.len() > old_todos.len() {
            let created = (new_todos.len() - old_todos.len()) as u64;
            stats.todos_created += created;
        }

        let old_completed = old_todos
            .iter()
            .filter(|todo| todo.status == "completed")
            .count();
        let new_completed = new_todos
            .iter()
            .filter(|todo| todo.status == "completed")
            .count();

        if new_completed > old_completed {
            let completed = (new_completed - old_completed) as u64;
            stats.todos_completed += completed;
        }

        let old_in_progress = old_todos
            .iter()
            .filter(|todo| todo.status == "in_progress")
            .count();
        let new_in_progress = new_todos
            .iter()
            .filter(|todo| todo.status == "in_progress")
            .count();

        if new_in_progress > old_in_progress {
            let in_progress = (new_in_progress - old_in_progress) as u64;
            stats.todos_in_progress += in_progress;
        }
    }

    stats
}

pub fn calculate_cost_from_tokens(usage: &Usage, model_name: &str) -> f64 {
    crate::models::calculate_total_cost_with_cache_ttl(
        model_name,
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_creation_input_tokens,
        usage
            .cache_creation
            .as_ref()
            .map(|c| c.ephemeral_1h_input_tokens)
            .unwrap_or(0),
        usage.cache_read_input_tokens,
    )
}

pub fn extract_text_content(content: &Content) -> Option<String> {
    match content {
        Content::String(bytes) => {
            // Try to decode as UTF-8, fallback to lossy conversion
            let text = String::from_utf8(bytes.to_vec()).ok()?;
            Some(clean_message_text(&text))
        }
        Content::Blocks(blocks) => {
            let mut all_content = Vec::new();
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        if let Ok(text_str) = String::from_utf8(text.to_vec()) {
                            let cleaned = clean_message_text(&text_str);
                            if !cleaned.trim().is_empty() {
                                all_content.push(cleaned);
                            }
                        }
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        // Show thinking content without the "THINKING:" label
                        if let Ok(thinking_str) = String::from_utf8(thinking.to_vec()) {
                            let cleaned = clean_message_text(&thinking_str);
                            if !cleaned.trim().is_empty() {
                                all_content.push(cleaned);
                            }
                        }
                    }
                    ContentBlock::ToolUse { .. } => {
                        // Skip tool use blocks entirely - "Read", "Edit", etc. are not
                        // interesting for message previews. Tool stats are tracked separately.
                    }
                    ContentBlock::ToolResult { .. } => {
                        // Skip tool results - they're usually verbose command outputs
                        // and not useful for message previews
                    }
                    ContentBlock::Image { .. } => {
                        all_content.push("Image".to_string());
                    }
                    ContentBlock::Other => {}
                }
            }

            if all_content.is_empty() {
                None
            } else {
                Some(all_content.join(" | "))
            }
        }
    }
}

// Helper function to clean up message text
fn clean_message_text(text: &str) -> String {
    let mut result = text.to_string();

    // Remove "Caveat:" messages and everything after until the next paragraph
    if let Some(caveat_pos) = result.find("Caveat:") {
        if let Some(double_newline) = result[caveat_pos..].find("\n\n") {
            result.replace_range(caveat_pos..caveat_pos + double_newline + 2, "");
        } else {
            // If no double newline, remove everything from Caveat to the end
            result.truncate(caveat_pos);
        }
    }

    // Parse command tags like <command-name>/clear</command-name> to just /clear
    while let Some(start_tag_pos) = result.find("<command-name>") {
        if let Some(end_tag_pos) = result[start_tag_pos..].find("</command-name>") {
            let command_start = start_tag_pos + "<command-name>".len();
            let command_end = start_tag_pos + end_tag_pos;
            let command = result[command_start..command_end].to_string();
            result.replace_range(
                start_tag_pos..command_end + "</command-name>".len(),
                &command,
            );
        } else {
            break;
        }
    }

    // Remove other common tags
    result = result.replace("<command-message>", "");
    result = result.replace("</command-message>", "");
    result = result.replace("<command-args>", "");
    result = result.replace("</command-args>", "");
    result = result.replace("<local-command-stdout>", "");
    result = result.replace("</local-command-stdout>", "");

    // Clean up extra whitespace
    result.trim().to_string()
}

/// Per-file parser state that must survive across incremental chunks of the same file.
#[derive(Debug, Default, Clone)]
pub struct JsonlParseState {
    /// Project label learned from the first entry's `cwd`; applied to every later message in
    /// the file, so it has to carry over when we resume parsing appended lines.
    pub project_label: Option<String>,
    /// Lines seen so far, so warnings report the true line number after a resume.
    pub lines_seen: usize,
}

/// Parse a whole file from the start. Accepts an unterminated final line, which is what you
/// want for a file that is finished (and for in-memory fixtures). The binary goes through the
/// incremental cache instead; this stays for tests and benches.
#[allow(dead_code)]
pub fn parse_jsonl_file<T>(
    file_path: &Path,
    buffer_reader: &mut BufReader<T>,
) -> Vec<ConversationMessage>
where
    T: Read,
{
    parse_jsonl_chunk(file_path, buffer_reader, &mut JsonlParseState::default(), true).0
}

/// Parse complete lines from `reader`, which may be positioned partway into the file, carrying
/// `state` across calls. Returns the messages and the exact number of bytes consumed so the
/// caller can resume there. With `accept_unterminated_tail == false`, a final line that has no
/// trailing newline is left unread: a writer may still be appending to it.
pub fn parse_jsonl_chunk<R>(
    file_path: &Path,
    reader: &mut R,
    state: &mut JsonlParseState,
    accept_unterminated_tail: bool,
) -> ChunkResult
where
    R: BufRead,
{
    // We do this instead of using the `sessionId` property on the message objects because
    // forked conversations keep the `sessionId` from the parent.
    let session_id = file_path.file_stem().unwrap().to_str().unwrap();

    let project_id_from_path = extract_project_id(file_path);
    let mut entries = Vec::new();
    // A subagent's work belongs to the session that spawned it: sharing the parent's
    // conversation hash rolls its usage in without counting it as a session of its own.
    let conversation_path = subagent_parent_transcript(file_path);
    let conversation_path = conversation_path.as_deref().unwrap_or(file_path);
    let conversation_hash = hash_text(&conversation_path.to_string_lossy());
    let mut consumed: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if buf.last() != Some(&b'\n') && !accept_unterminated_tail {
            break; // Partial trailing line: a writer is mid-append. Pick it up next time.
        }
        consumed += n as u64;
        state.lines_seen += 1;

        while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
            buf.pop();
        }
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }

        // Parse the line, filtering various invalid scenarios.
        let parsed_line = simd_json::from_slice::<ClaudeCodeEntry>(&mut buf);
        let (
            message_id,
            model,
            content,
            usage,
            timestamp,
            request_id,
            tool_use_result,
            uuid,
            entry,
        ) = match parsed_line {
                // Only get real user/AI messages; filter out summaries and any garbage message entries.
                Ok(ClaudeCodeEntry::Message(ref entry @ ClaudeCodeMessageEntry {
                                                message:
                                                Some(Message {
                                                         id: ref message_id,
                                                         ref model,
                                                         content: Some(ref content),
                                                         ref usage,
                                                         ..
                                                     }),
                                                ref timestamp,
                                                ref tool_use_result,
                                                ref request_id,
                                                ref uuid,
                                                ..
                                                // Skip messages for which no model is specified, or the model is `<synthetic>`;
                                                // i.e. Claude Code-generated system messages.  These have their token usage all
                                                // 0 anyway.
                                            })) if !matches!(model.as_deref(), Some("<synthetic>")) => {
                    if state.project_label.is_none() {
                        if let Some(cwd) = &entry.cwd {
                            if let Some(label) = Path::new(cwd).file_name().and_then(|n| n.to_str()) {
                                state.project_label = Some(label.to_string());
                            }
                        }
                    }
                    (
                        message_id.clone(),
                        model.clone(),
                        content.clone(),
                        usage.clone(),
                        *timestamp,
                        request_id.clone(),
                        tool_use_result.clone(),
                        uuid.clone(),
                        entry,
                    )
                }
                Err(e) => {
                    crate::utils::warn_once(format!(
                        "Skipping invalid entry in {} line {}: {}",
                        file_path.display(),
                        state.lines_seen,
                        e
                    ));
                    continue;
                }
                _ => continue,
            };

        let mut stats = extract_tool_stats(&content, &tool_use_result);
        let mut content_opt = extract_text_content(&content);
        let mut local_hash = None;

        match usage {
            Some(ref usage) => {
                let model = match &model {
                    Some(m) => m.to_owned(),
                    None => continue, // Invalid entry.
                };

                stats.input_tokens = usage.input_tokens;
                stats.output_tokens = usage.output_tokens;
                stats.cache_creation_tokens = usage.cache_creation_input_tokens;
                stats.cache_creation_1h_tokens = usage
                    .cache_creation
                    .as_ref()
                    .map(|c| c.ephemeral_1h_input_tokens)
                    .unwrap_or(0);
                stats.cache_read_tokens = usage.cache_read_input_tokens;
                stats.cached_tokens =
                    usage.cache_creation_input_tokens + usage.cache_read_input_tokens;
                stats.cost = calculate_cost_from_tokens(&usage, &model);
                stats.tool_calls = match content {
                    Content::Blocks(blocks) => blocks
                        .iter()
                        .filter(|c| matches!(c, ContentBlock::ToolUse { .. }))
                        .count() as u32,
                    _ => 0,
                };

                if let Some(request_id) = request_id
                    && let Some(message_id) = message_id
                {
                    // Claude Code stores the different parts of a message--thinking, tool calls, and normal
                    // text--each in their own row in the JSONL file, with the same request ID and, redundantly,
                    // the same token usage for each; therefore, we skip ones we've seen before to avoid
                    // accumulating the redundant token counts.  It would be simpler to just skip
                    // them here, but actually some message _across files_ can have the same
                    // message ID and request ID (probably due to resuming sessions), so we need to
                    // do it later when we have a complete pool of messages.
                    local_hash = Some(hash_text(&format!("{request_id}_{message_id}")));
                }
            }
            None => {
                // For user messages, try to extract content from the message if available
                if let Some(message) = &entry.message {
                    if let Some(content) = &message.content {
                        content_opt = extract_text_content(content);
                    }
                }
            }
        }

        // Determine role from nested message.role field
        // Note: entry.r#type is always None because serde's tag="type" consumes that field
        let is_assistant = entry
            .message
            .as_ref()
            .and_then(|m| m.role.as_deref())
            == Some("assistant");

        // Skip tool-result-only messages - they're API plumbing, not conversation turns
        // (tool results are passed back to Claude as "user" messages in the API)
        if !is_assistant && tool_use_result.is_some() {
            continue;
        }

        let msg = ConversationMessage {
            global_hash: hash_text(&format!("{session_id}_{uuid}")),
            local_hash,
            application: Application::ClaudeCode,
            model: model.clone(),
            date: timestamp,
            project_hash: state
                .project_label
                .clone()
                .unwrap_or_else(|| project_id_from_path.clone()),
            conversation_hash: conversation_hash.clone(),
            stats,
            role: if is_assistant {
                MessageRole::Assistant
            } else {
                MessageRole::User
            },
            content: content_opt,
        };
        entries.push(msg);
    }

    (entries, consumed)
}

// Type alias for token fingerprint tracking to avoid type complexity
type TokenFingerprint = (u64, u64, u64, u64, u64);
#[cfg(test)]
type TokenFingerprintMap = HashMap<String, HashSet<TokenFingerprint>>;

/// Batch reference for the dedup rules `Accumulator::fold` applies incrementally. The live path
/// no longer calls it (BZL-14); tests keep it to pin down what folding must reproduce.
#[cfg(test)]
pub(crate) fn dedup_messages<'a>(
    messages: impl Iterator<Item = &'a ConversationMessage>,
) -> Vec<ConversationMessage> {
    let mut seen_hashes = HashMap::<String, usize>::new(); // hash -> index in result
    // Track distinct token tuples seen per local_hash to avoid double-counting
    let mut seen_token_fingerprints: TokenFingerprintMap = HashMap::new();
    let mut deduplicated_entries: Vec<ConversationMessage> = Vec::new();

    for message in messages {
        if let Some(local_hash) = &message.local_hash {
            if let Some(&existing_index) = seen_hashes.get(local_hash) {
                // Compute the token tuple fingerprint for this message
                let fp = (
                    message.stats.input_tokens,
                    message.stats.output_tokens,
                    message.stats.cache_creation_tokens,
                    message.stats.cache_read_tokens,
                    message.stats.cached_tokens,
                );

                // Initialize set for this hash if missing
                let set = seen_token_fingerprints
                    .entry(local_hash.clone())
                    .or_default();

                // If we've already seen this exact token tuple for this local_hash,
                // it's a redundant duplicate (old format or repeated identical row).
                if set.contains(&fp) {
                    // Merge non-token stats so we don't lose tool/activity counts
                    let existing_mut = &mut deduplicated_entries[existing_index];
                    existing_mut.stats.tool_calls =
                        existing_mut.stats.tool_calls.max(message.stats.tool_calls);
                    existing_mut.stats.files_read =
                        existing_mut.stats.files_read.max(message.stats.files_read);
                    existing_mut.stats.files_edited = existing_mut
                        .stats
                        .files_edited
                        .max(message.stats.files_edited);
                    existing_mut.stats.files_added = existing_mut
                        .stats
                        .files_added
                        .max(message.stats.files_added);
                    existing_mut.stats.terminal_commands = existing_mut
                        .stats
                        .terminal_commands
                        .max(message.stats.terminal_commands);
                    existing_mut.stats.file_searches = existing_mut
                        .stats
                        .file_searches
                        .max(message.stats.file_searches);
                    existing_mut.stats.file_content_searches = existing_mut
                        .stats
                        .file_content_searches
                        .max(message.stats.file_content_searches);
                    existing_mut.stats.todo_writes = existing_mut
                        .stats
                        .todo_writes
                        .max(message.stats.todo_writes);
                    existing_mut.stats.todo_reads =
                        existing_mut.stats.todo_reads.max(message.stats.todo_reads);
                    existing_mut.stats.todos_created = existing_mut
                        .stats
                        .todos_created
                        .max(message.stats.todos_created);
                    existing_mut.stats.todos_completed = existing_mut
                        .stats
                        .todos_completed
                        .max(message.stats.todos_completed);
                    existing_mut.stats.todos_in_progress = existing_mut
                        .stats
                        .todos_in_progress
                        .max(message.stats.todos_in_progress);
                    continue;
                }

                // Mark this distinct tuple as seen and aggregate its values
                set.insert(fp);

                // Another snapshot of the same request (see `FoldState`): tokens are the
                // largest seen, not the sum; this row's own activity adds.
                let existing = &mut deduplicated_entries[existing_index];

                existing.stats.input_tokens = existing.stats.input_tokens.max(message.stats.input_tokens);
                existing.stats.output_tokens = existing.stats.output_tokens.max(message.stats.output_tokens);
                existing.stats.cache_creation_tokens =
                    existing.stats.cache_creation_tokens.max(message.stats.cache_creation_tokens);
                existing.stats.cache_creation_1h_tokens = existing
                    .stats
                    .cache_creation_1h_tokens
                    .max(message.stats.cache_creation_1h_tokens);
                existing.stats.cache_read_tokens = existing.stats.cache_read_tokens.max(message.stats.cache_read_tokens);
                existing.stats.cached_tokens = existing.stats.cached_tokens.max(message.stats.cached_tokens);

                // Aggregate tool stats
                existing.stats.tool_calls += message.stats.tool_calls;
                existing.stats.files_read += message.stats.files_read;
                existing.stats.files_edited += message.stats.files_edited;
                existing.stats.files_added += message.stats.files_added;
                existing.stats.terminal_commands += message.stats.terminal_commands;
                existing.stats.file_searches += message.stats.file_searches;
                existing.stats.file_content_searches += message.stats.file_content_searches;
                existing.stats.todo_writes += message.stats.todo_writes;
                existing.stats.todo_reads += message.stats.todo_reads;
                existing.stats.todos_created += message.stats.todos_created;
                existing.stats.todos_completed += message.stats.todos_completed;
                existing.stats.todos_in_progress += message.stats.todos_in_progress;

                // Recalculate cost with the aggregated tokens
                if let Some(model) = &existing.model {
                    existing.stats.cost = crate::models::calculate_total_cost_with_cache_ttl(
                        model,
                        existing.stats.input_tokens,
                        existing.stats.output_tokens,
                        existing.stats.cache_creation_tokens,
                        existing.stats.cache_creation_1h_tokens,
                        existing.stats.cache_read_tokens,
                    );
                }
            } else {
                // First time seeing this hash, add it
                seen_hashes.insert(local_hash.clone(), deduplicated_entries.len());
                // Seed the seen tuple set for this local_hash with this message's tokens
                let fp = (
                    message.stats.input_tokens,
                    message.stats.output_tokens,
                    message.stats.cache_creation_tokens,
                    message.stats.cache_read_tokens,
                    message.stats.cached_tokens,
                );
                seen_token_fingerprints
                    .entry(local_hash.clone())
                    .or_default()
                    .insert(fp);

                deduplicated_entries.push(message.clone());
            }
        } else {
            // No local hash, always keep
            deduplicated_entries.push(message.clone());
        }
    }

    deduplicated_entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageRole, Stats};

    #[test]
    fn a_reply_written_as_three_lines_counts_once_in_the_live_messages() {
        // Claude Code writes one line per content block (thinking, text, tool_use), each
        // repeating the request's usage. The live window must count the reply once, as the
        // daily totals do (BZL-192: "Now" tk/s read about 2.3x).
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects").join("p");
        std::fs::create_dir_all(&project).unwrap();
        let now = chrono::Utc::now();
        let mut lines = String::new();
        for (i, block) in [r#"{"type":"thinking","thinking":"hm"}"#, r#"{"type":"text","text":"ok"}"#, r#"{"type":"tool_use","id":"t","name":"Bash","input":{}}"#]
            .iter()
            .enumerate()
        {
            let at = (now - chrono::Duration::seconds(60 - i as i64)).to_rfc3339();
            lines.push_str(&format!(
                r#"{{"type":"assistant","uuid":"u{i}","timestamp":"{at}","sessionId":"s","cwd":"/tmp/p","requestId":"req_1","message":{{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5-20250929","content":[{block}],"usage":{{"input_tokens":100,"output_tokens":300,"cache_read_input_tokens":5000}}}}}}"#
            ));
            lines.push('\n');
        }
        std::fs::write(project.join("s.jsonl"), lines).unwrap();

        let analyzer = ClaudeCodeAnalyzer::for_account(None, dir.path().to_path_buf());
        let sources = analyzer.discover_data_sources().unwrap();
        let (messages, _) = analyzer.parse_conversations_with_report(&sources);
        let replies: Vec<&ConversationMessage> = messages.iter().filter(|m| m.role == MessageRole::Assistant).collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stats.output_tokens, 300);
        assert_eq!(replies[0].stats.tool_calls, 1);
        let day = analyzer.daily_stats();
        let day = day.values().next().unwrap();
        assert_eq!(day.stats.output_tokens, 300);
        assert!((replies[0].stats.cost - day.stats.cost).abs() < 1e-9);
        // Its hour/model/project slice carries the same, once (BZL-138).
        assert_eq!(day.slices.len(), 1);
        assert_eq!((day.slices[0].replies, day.slices[0].output_tokens, day.slices[0].project.as_str()), (1, 300, "p"));
        assert!((day.slices[0].cost - day.stats.cost).abs() < 1e-9);
    }

    #[test]
    fn replies_are_credited_to_the_login_the_account_was_on_then() {
        let t0 = chrono::Utc::now() - chrono::Duration::hours(3);
        let timeline = LoginTimeline::parse(&format!(
            "{{\"t\": {}, \"session\": \"s\", \"login\": \"a@virti.us (personal)\"}}\n{{\"t\": {}, \"session\": \"s\", \"login\": \"support@virti.us (Virtius)\"}}\n",
            t0.timestamp(),
            (t0 + chrono::Duration::hours(1)).timestamp()
        ));
        assert_eq!(timeline.at(t0 - chrono::Duration::minutes(1)), "");
        assert_eq!(timeline.at(t0 + chrono::Duration::minutes(30)), "a@virti.us (personal)");
        assert_eq!(timeline.at(t0 + chrono::Duration::hours(2)), "support@virti.us (Virtius)");

        let reply = |minutes: i64, id: &str| ConversationMessage {
            global_hash: id.into(),
            local_hash: Some(id.into()),
            application: Application::ClaudeCode,
            model: Some("claude-opus-5-5".into()),
            date: t0 + chrono::Duration::minutes(minutes),
            project_hash: "p".into(),
            conversation_hash: "c".into(),
            role: MessageRole::Assistant,
            content: None,
            stats: Stats { output_tokens: 100, ..Default::default() },
        };
        let mut accum = Accumulator::default();
        for message in [reply(30, "a"), reply(120, "b")] {
            accum.fold(&message, timeline.at(message.date));
        }
        let accounts: Vec<(String, u64)> = accum
            .daily_stats()
            .values()
            .flat_map(|d| d.slices.iter().map(|s| (s.account.clone(), s.output_tokens)))
            .collect();
        assert!(accounts.contains(&("a@virti.us (personal)".into(), 100)));
        assert!(accounts.contains(&("support@virti.us (Virtius)".into(), 100)));
    }

    #[test]
    fn a_growing_reply_moves_its_slice_with_the_day() {
        // Cumulative snapshots: output 10, then 400. The day and the slice both end at 400.
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects").join("p");
        std::fs::create_dir_all(&project).unwrap();
        let at = (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339();
        let mut lines = String::new();
        for (i, output) in [10, 400].iter().enumerate() {
            lines.push_str(&format!(
                r#"{{"type":"assistant","uuid":"u{i}","timestamp":"{at}","sessionId":"s","cwd":"/tmp/p","requestId":"req_1","message":{{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5-20250929","content":[{{"type":"text","text":"x"}}],"usage":{{"input_tokens":100,"output_tokens":{output}}}}}}}"#
            ));
            lines.push('\n');
        }
        std::fs::write(project.join("s.jsonl"), lines).unwrap();
        let analyzer = ClaudeCodeAnalyzer::for_account(None, dir.path().to_path_buf());
        let sources = analyzer.discover_data_sources().unwrap();
        analyzer.parse_conversations_with_report(&sources);
        let days = analyzer.daily_stats();
        let day = days.values().next().unwrap();
        let slice_output: u64 = day.slices.iter().map(|s| s.output_tokens).sum();
        let slice_cost: f64 = day.slices.iter().map(|s| s.cost).sum();
        assert_eq!((day.stats.output_tokens, slice_output), (400, 400));
        assert!((slice_cost - day.stats.cost).abs() < 1e-9);
    }

    #[test]
    fn test_deduplicate_partial_split_messages() {
        // One request written as three rows with differing usage: cumulative snapshots, so the
        // message's tokens are the largest seen (BZL-132), while each row's activity adds.
        let hash = "test_partial_split".to_string();

        let messages = vec![
            ConversationMessage {
                global_hash: "unique1".to_string(),
                local_hash: Some(hash.clone()),
                application: Application::ClaudeCode,
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                date: chrono::Utc::now(),
                project_hash: "proj1".to_string(),
                conversation_hash: "conv1".to_string(),
                role: MessageRole::Assistant,
                content: None,
                stats: Stats {
                    input_tokens: 10, // Different from others
                    output_tokens: 2, // thinking block
                    tool_calls: 0,
                    ..Default::default()
                },
            },
            ConversationMessage {
                global_hash: "unique2".to_string(),
                local_hash: Some(hash.clone()),
                application: Application::ClaudeCode,
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                date: chrono::Utc::now(),
                project_hash: "proj1".to_string(),
                conversation_hash: "conv1".to_string(),
                role: MessageRole::Assistant,
                content: None,
                stats: Stats {
                    input_tokens: 5,  // Different from others
                    output_tokens: 2, // text block
                    tool_calls: 0,
                    ..Default::default()
                },
            },
            ConversationMessage {
                global_hash: "unique3".to_string(),
                local_hash: Some(hash.clone()),
                application: Application::ClaudeCode,
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                date: chrono::Utc::now(),
                project_hash: "proj1".to_string(),
                conversation_hash: "conv1".to_string(),
                role: MessageRole::Assistant,
                content: None,
                stats: Stats {
                    input_tokens: 0,    // Different from others
                    output_tokens: 447, // tool_use block
                    tool_calls: 1,
                    ..Default::default()
                },
            },
        ];

        let deduplicated = dedup_messages(messages.iter());

        // Should have exactly 1 message (all 3 merged)
        assert_eq!(deduplicated.len(), 1);

        // The final snapshot's output, not 2 + 2 + 447.
        assert_eq!(deduplicated[0].stats.output_tokens, 447);

        // The largest input seen, not 10 + 5 + 0.
        assert_eq!(deduplicated[0].stats.input_tokens, 10);

        // Tool calls come from each row's own content, so they add
        assert_eq!(deduplicated[0].stats.tool_calls, 1);
    }

    #[test]
    fn test_deduplicate_redundant_split_messages() {
        // Test the old format (Oct 16-17 2025): Split messages with IDENTICAL redundant tokens
        let hash = "test_redundant_split".to_string();

        let messages = vec![
            ConversationMessage {
                global_hash: "unique1".to_string(),
                local_hash: Some(hash.clone()),
                application: Application::ClaudeCode,
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                date: chrono::Utc::now(),
                project_hash: "proj1".to_string(),
                conversation_hash: "conv1".to_string(),
                role: MessageRole::Assistant,
                content: None,
                stats: Stats {
                    output_tokens: 4, // All blocks report same total
                    input_tokens: 100,
                    tool_calls: 2,
                    ..Default::default()
                },
            },
            ConversationMessage {
                global_hash: "unique2".to_string(),
                local_hash: Some(hash.clone()),
                application: Application::ClaudeCode,
                model: Some("claude-sonnet-4-5-20250929".to_string()),
                date: chrono::Utc::now(),
                project_hash: "proj1".to_string(),
                conversation_hash: "conv1".to_string(),
                role: MessageRole::Assistant,
                content: None,
                stats: Stats {
                    output_tokens: 4,  // Identical
                    input_tokens: 100, // Identical
                    tool_calls: 2,     // Not checked for identity, but will be kept
                    ..Default::default()
                },
            },
        ];

        let deduplicated = dedup_messages(messages.iter());

        // Should have exactly 1 message (duplicate skipped)
        assert_eq!(deduplicated.len(), 1);

        // Tokens should NOT be summed (identical entries)
        assert_eq!(deduplicated[0].stats.output_tokens, 4);
        assert_eq!(deduplicated[0].stats.input_tokens, 100);

        // Tool calls are from the first entry only
        assert_eq!(deduplicated[0].stats.tool_calls, 2);
    }

    #[test]
    fn test_identical_tokens_merge_tool_stats() {
        // First row has no tools; second row has tool_calls=1, tokens identical
        let hash = "identical_merge_tools".to_string();

        let msg1 = ConversationMessage {
            global_hash: "g1".to_string(),
            local_hash: Some(hash.clone()),
            application: Application::ClaudeCode,
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            date: chrono::Utc::now(),
            project_hash: "proj".to_string(),
            conversation_hash: "conv".to_string(),
            role: MessageRole::Assistant,
            content: None,
            stats: Stats {
                input_tokens: 100,
                output_tokens: 4,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                cached_tokens: 0,
                tool_calls: 0,
                ..Default::default()
            },
        };

        let msg2 = ConversationMessage {
            global_hash: "g2".to_string(),
            local_hash: Some(hash.clone()),
            application: Application::ClaudeCode,
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            date: chrono::Utc::now(),
            project_hash: "proj".to_string(),
            conversation_hash: "conv".to_string(),
            role: MessageRole::Assistant,
            content: None,
            stats: Stats {
                input_tokens: 100,
                output_tokens: 4,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
                cached_tokens: 0,
                tool_calls: 1,
                ..Default::default()
            },
        };

        let dedup = dedup_messages([msg1, msg2].iter());
        assert_eq!(dedup.len(), 1);
        // Tokens unchanged
        assert_eq!(dedup[0].stats.input_tokens, 100);
        assert_eq!(dedup[0].stats.output_tokens, 4);
        // Tool calls merged from second row
        assert_eq!(dedup[0].stats.tool_calls, 1);
    }

    #[test]
    fn test_deduplicate_skips_identical_after_partial_aggregate() {
        // Mix of partials and redundant duplicates for the same local_hash
        let hash = "test_mixed_duplicates".to_string();

        let a1 = ConversationMessage {
            global_hash: "ga1".to_string(),
            local_hash: Some(hash.clone()),
            application: Application::ClaudeCode,
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            date: chrono::Utc::now(),
            project_hash: "proj".to_string(),
            conversation_hash: "conv".to_string(),
            role: MessageRole::Assistant,
            content: None,
            stats: Stats {
                input_tokens: 10,
                output_tokens: 2,
                ..Default::default()
            },
        };
        // Exact duplicate of a1 (should be skipped)
        let a2 = ConversationMessage {
            global_hash: "ga2".to_string(),
            ..a1.clone()
        };

        // Tool-use partial
        let b1 = ConversationMessage {
            global_hash: "gb1".to_string(),
            local_hash: Some(hash.clone()),
            application: Application::ClaudeCode,
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            date: chrono::Utc::now(),
            project_hash: "proj".to_string(),
            conversation_hash: "conv".to_string(),
            role: MessageRole::Assistant,
            content: None,
            stats: Stats {
                input_tokens: 0,
                output_tokens: 447,
                tool_calls: 1,
                ..Default::default()
            },
        };
        // Exact duplicate of b1 (should be skipped)
        let b2 = ConversationMessage {
            global_hash: "gb2".to_string(),
            ..b1.clone()
        };

        // Another duplicate of a1 after aggregation (should still be skipped)
        let a3 = ConversationMessage {
            global_hash: "ga3".to_string(),
            ..a1.clone()
        };

        let messages = vec![a1, a2, b1, b2, a3];
        let deduplicated = dedup_messages(messages.iter());

        assert_eq!(deduplicated.len(), 1);
        // Should include A once and B once
        assert_eq!(deduplicated[0].stats.input_tokens, 10);
        assert_eq!(deduplicated[0].stats.output_tokens, 447);
        assert_eq!(deduplicated[0].stats.tool_calls, 1);
    }
}

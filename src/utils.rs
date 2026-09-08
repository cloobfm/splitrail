use std::collections::{BTreeMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
mod tests;

use anyhow::Result;
use chrono::{DateTime, Datelike, Local, Utc};
use num_format::{Locale, ToFormattedString};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{ConversationMessage, DailyStats};

/// Quota limits for different time periods and types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct QuotaLimits {
    /// Daily token limit (input + output)
    pub daily_tokens: Option<u64>,
    /// Weekly token limit
    pub weekly_tokens: Option<u64>,
    /// Monthly token limit
    pub monthly_tokens: Option<u64>,
    /// Session time limit in minutes
    pub session_time_minutes: Option<u64>,
    /// Daily request limit
    pub daily_requests: Option<u64>,
}

struct WarningEntry {
    message: String,
    timestamp: DateTime<Utc>,
}

static WARNED_MESSAGES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static WARNINGS: OnceLock<Mutex<Vec<WarningEntry>>> = OnceLock::new();
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const MAX_LOG_BYTES: u64 = 1_000_000; // ~1MB
const RETAIN_LOG_BYTES: u64 = 512_000; // keep last ~512KB when rotating

pub fn warn_once(message: impl Into<String>) {
    let message = message.into();
    let cache = WARNED_MESSAGES.get_or_init(|| Mutex::new(HashSet::new()));
    let warnings = WARNINGS.get_or_init(|| Mutex::new(Vec::new()));

    if let Ok(mut warned) = cache.lock()
        && warned.insert(message.clone())
    {
        if let Ok(mut warns) = warnings.lock() {
            warns.push(WarningEntry {
                message,
                timestamp: Utc::now(),
            });
        }
    }
}

pub fn get_warnings() -> Vec<String> {
    WARNINGS.get().map_or(Vec::new(), |w| {
        if let Ok(warns) = w.lock() {
            let now = Utc::now();
            // Filter out warnings older than 1 minute
            warns
                .iter()
                .filter(|entry| {
                    let duration = now.signed_duration_since(entry.timestamp);
                    duration.num_seconds() < 60
                })
                .map(|entry| entry.message.clone())
                .collect()
        } else {
            Vec::new()
        }
    })
}

pub fn clear_old_warnings() {
    if let Some(warnings) = WARNINGS.get() {
        if let Ok(mut warns) = warnings.lock() {
            let now = Utc::now();
            warns.retain(|entry| {
                let duration = now.signed_duration_since(entry.timestamp);
                duration.num_seconds() < 60
            });
        }
    }
}

fn log_file_path() -> Option<PathBuf> {
    std::env::home_dir().map(|mut home| {
        home.push(".splitrail.log");
        home
    })
}

fn write_log_line(level: &str, message: &str) {
    let guard = LOG_LOCK.get_or_init(|| Mutex::new(())).lock();
    let timestamp = chrono::Utc::now().to_rfc3339();
    if let Ok(_guard) = guard
        && let Some(path) = log_file_path()
    {
        let line = format!("[{timestamp}] {level}: {message}\n");
        let _ = path.parent().map(std::fs::create_dir_all);
        rotate_log_if_needed(&path);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn log_error(message: impl AsRef<str>) {
    write_log_line("ERROR", message.as_ref());
}

pub fn read_log_tail(max_lines: usize) -> Option<Vec<String>> {
    if max_lines == 0 {
        return Some(Vec::new());
    }
    let path = log_file_path()?;
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut buffer: Vec<String> = Vec::new();
    for line in reader.lines().flatten() {
        buffer.push(line);
        if buffer.len() > max_lines {
            buffer.remove(0);
        }
    }
    Some(buffer)
}

fn rotate_log_if_needed(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };

    if meta.len() <= MAX_LOG_BYTES {
        return;
    }

    if let Ok(mut file) = std::fs::OpenOptions::new().read(true).open(path) {
        let keep_from = meta
            .len()
            .saturating_sub(RETAIN_LOG_BYTES)
            .try_into()
            .unwrap_or(0);
        if file.seek(SeekFrom::Start(keep_from)).is_ok() {
            let mut buffer = Vec::new();
            let _ = file.read_to_end(&mut buffer);
            if let Ok(mut out) = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
            {
                let _ = out.write_all(&buffer);
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct NumberFormatOptions {
    pub use_comma: bool,
    pub use_human: bool,
    pub locale: String,
    pub decimal_places: usize,
}

pub fn format_number(n: u64, options: &NumberFormatOptions) -> String {
    let locale = match options.locale.as_str() {
        "de" => Locale::de,
        "fr" => Locale::fr,
        "es" => Locale::es,
        "it" => Locale::it,
        "ja" => Locale::ja,
        "ko" => Locale::ko,
        "zh" => Locale::zh,
        _ => Locale::en,
    };

    if options.use_human {
        if n >= 1_000_000_000_000 {
            format!(
                "{:.prec$}t",
                n as f64 / 1_000_000_000_000.0,
                prec = options.decimal_places
            )
        } else if n >= 1_000_000_000 {
            format!(
                "{:.prec$}b",
                n as f64 / 1_000_000_000.0,
                prec = options.decimal_places
            )
        } else if n >= 1_000_000 {
            format!(
                "{:.prec$}m",
                n as f64 / 1_000_000.0,
                prec = options.decimal_places
            )
        } else if n >= 1_000 {
            format!(
                "{:.prec$}k",
                n as f64 / 1_000.0,
                prec = options.decimal_places
            )
        } else {
            n.to_string()
        }
    } else if options.use_comma {
        n.to_formatted_string(&locale)
    } else {
        n.to_string()
    }
}

pub fn format_date_for_display(date: &str) -> String {
    if date == "unknown" {
        return "Unknown".to_string();
    }

    if let Ok(parsed) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        // Format with non-padded month and day
        let month = parsed.month();
        let day = parsed.day();
        let year = parsed.year();
        let formatted = format!("{month}/{day}/{year}");

        // Check if this is today's date
        let today = chrono::Local::now().date_naive();
        if parsed == today {
            format!("{formatted}*")
        } else {
            formatted
        }
    } else {
        date.to_string()
    }
}

pub fn format_timestamp_for_live_view(timestamp: &DateTime<Utc>) -> String {
    let now = Local::now();
    let local_timestamp = timestamp.with_timezone(&Local);
    if local_timestamp.date_naive() == now.date_naive() {
        local_timestamp.format("%H:%M:%S").to_string()
    } else {
        local_timestamp.format("%y-%m-%d %H:%M").to_string()
    }
}

/// Incrementally aggregate new messages into existing daily stats
/// This is more efficient than re-aggregating everything when only a few messages changed
#[allow(dead_code)] // Retained for potential incremental ingestion path; currently unused
pub fn aggregate_by_date_incremental(
    existing_stats: &mut BTreeMap<String, DailyStats>,
    existing_conversations: &mut BTreeMap<String, String>,
    new_entries: &[ConversationMessage],
) {
    for entry in new_entries {
        let timestamp = &entry.date.with_timezone(&Local);
        let conversation_hash = &entry.conversation_hash;
        let date = timestamp.format("%Y-%m-%d").to_string();

        // Track conversation start dates
        existing_conversations
            .entry(conversation_hash.clone())
            .and_modify(|existing_date| {
                if date < *existing_date {
                    *existing_date = date.clone();
                }
            })
            .or_insert(date.clone());

        let daily_stats_entry = existing_stats
            .entry(date.clone())
            .or_insert_with(|| DailyStats {
                date: date.clone(),
                ..Default::default()
            });

        match &entry.model {
            Some(model) => {
                // AI message
                daily_stats_entry.ai_messages += 1;
                *daily_stats_entry
                    .models
                    .entry(model.to_string())
                    .or_insert(0) += 1;

                // Aggregate all stats
                daily_stats_entry.stats.cost += entry.stats.cost;
                daily_stats_entry.stats.input_tokens += entry.stats.input_tokens;
                daily_stats_entry.stats.output_tokens += entry.stats.output_tokens;
                daily_stats_entry.stats.reasoning_tokens += entry.stats.reasoning_tokens;
                daily_stats_entry.stats.cache_creation_tokens += entry.stats.cache_creation_tokens;
                daily_stats_entry.stats.cache_read_tokens += entry.stats.cache_read_tokens;
                daily_stats_entry.stats.cached_tokens += entry.stats.cached_tokens;
                daily_stats_entry.stats.tool_calls += entry.stats.tool_calls;
                daily_stats_entry.stats.terminal_commands += entry.stats.terminal_commands;
                daily_stats_entry.stats.file_searches += entry.stats.file_searches;
                daily_stats_entry.stats.file_content_searches += entry.stats.file_content_searches;
                daily_stats_entry.stats.files_read += entry.stats.files_read;
                daily_stats_entry.stats.files_added += entry.stats.files_added;
                daily_stats_entry.stats.files_edited += entry.stats.files_edited;
                daily_stats_entry.stats.files_deleted += entry.stats.files_deleted;
                daily_stats_entry.stats.lines_read += entry.stats.lines_read;
                daily_stats_entry.stats.lines_added += entry.stats.lines_added;
                daily_stats_entry.stats.lines_edited += entry.stats.lines_edited;
                daily_stats_entry.stats.lines_deleted += entry.stats.lines_deleted;
                daily_stats_entry.stats.bytes_read += entry.stats.bytes_read;
                daily_stats_entry.stats.bytes_added += entry.stats.bytes_added;
                daily_stats_entry.stats.bytes_edited += entry.stats.bytes_edited;
                daily_stats_entry.stats.bytes_deleted += entry.stats.bytes_deleted;
                daily_stats_entry.stats.todo_writes += entry.stats.todo_writes;
                daily_stats_entry.stats.todos_completed += entry.stats.todos_completed;
                daily_stats_entry.stats.todo_reads += entry.stats.todo_reads;
            }
            None => {
                // User message
                daily_stats_entry.user_messages += 1;
            }
        }
    }

    // Count unique conversations per day
    for (conversation_hash, start_date) in existing_conversations.iter() {
        if let Some(day_stats) = existing_stats.get_mut(start_date) {
            // This is a simple approximation - we're counting all conversations that started on this day
            // A more accurate version would track which conversations we've already counted
            let mut conversations = HashSet::new();
            for entry in new_entries {
                if entry.conversation_hash == *conversation_hash {
                    conversations.insert(conversation_hash.clone());
                }
            }
            // Only increment if this is a new conversation for this batch
            if !conversations.is_empty() {
                day_stats.conversations = existing_conversations
                    .values()
                    .filter(|d| d == &start_date)
                    .count() as u32;
            }
        }
    }
}

/// How much raw message history stays resident. Every live view reads at most the last hour
/// (token rate, sparkline, waiting-conversation detection); a day gives generous headroom.
/// Older messages live on only as `DailyStats`, which is the durable form of history.
pub const LIVE_WINDOW_HOURS: i64 = 24;

/// The oldest message worth keeping in memory.
///
/// Normally `now - LIVE_WINDOW_HOURS`. When uploading is configured and its watermark is further
/// back, the cutoff moves back to the watermark so trimming can never drop something the uploader
/// still owes the server.
pub fn live_window_cutoff(
    now: DateTime<Utc>,
    upload_watermark_millis: Option<i64>,
) -> DateTime<Utc> {
    let window_start = now - chrono::Duration::hours(LIVE_WINDOW_HOURS);
    match upload_watermark_millis.and_then(DateTime::from_timestamp_millis) {
        Some(watermark) if watermark < window_start => watermark,
        _ => window_start,
    }
}

/// The cutoff for this machine right now: the live window, pulled back to the upload watermark
/// when uploading is configured and lagging. Reads config directly so callers cannot forget it.
pub fn current_live_window_cutoff() -> DateTime<Utc> {
    let watermark = crate::config::Config::load()
        .ok()
        .flatten()
        .filter(|c| c.is_configured())
        .map(|c| c.upload.last_date_uploaded);
    live_window_cutoff(Utc::now(), watermark)
}

/// Drop messages older than `cutoff`. `daily_stats` is left alone: it already carries their
/// contribution, so history stays visible in aggregate form.
pub fn retain_live_window(stats: &mut crate::types::AgenticCodingToolStats, cutoff: DateTime<Utc>) {
    stats.messages.retain(|msg| msg.date >= cutoff);
    stats.messages.shrink_to_fit();
}

/// Gaps longer than this are treated as "away", not as time spent working.
const ACTIVE_GAP_LIMIT_SECS: i64 = 900;

/// Sum the gaps under [`ACTIVE_GAP_LIMIT_SECS`] between consecutive timestamps. `timestamps` is
/// sorted in place: callers collect them per day in arbitrary order.
fn active_seconds_for_day(timestamps: &mut [i64]) -> u64 {
    timestamps.sort_unstable();
    timestamps
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|gap| *gap > 0 && *gap < ACTIVE_GAP_LIMIT_SECS)
        .sum::<i64>() as u64
}

pub fn aggregate_by_date(entries: &[ConversationMessage]) -> BTreeMap<String, DailyStats> {
    let mut daily_stats: BTreeMap<String, DailyStats> = BTreeMap::new();
    let mut conversation_start_dates: BTreeMap<String, String> = BTreeMap::new();
    // Collected per day, then reduced to `active_seconds` once every entry has been seen, since
    // entries arrive per-file and are not globally ordered.
    let mut day_timestamps: BTreeMap<String, Vec<i64>> = BTreeMap::new();

    for entry in entries {
        let timestamp = &entry.date.with_timezone(&Local);
        let conversation_hash = &entry.conversation_hash;
        let date = timestamp.format("%Y-%m-%d").to_string();

        // Only update if this is earlier than what we've seen, or if we haven't seen this
        // conversation before.  This is to handle the case where a conversation spans
        // multiple days, we'd want to ascribe it to the day on which it was started.
        conversation_start_dates
            .entry(conversation_hash.clone())
            .and_modify(|existing_date| {
                if date < *existing_date {
                    *existing_date = date.clone();
                }
            })
            .or_insert(date.clone());

        day_timestamps
            .entry(date.clone())
            .or_default()
            .push(entry.date.timestamp());

        let daily_stats_entry = daily_stats
            .entry(date.clone())
            .or_insert_with(|| DailyStats {
                date: date.clone(),
                ..Default::default()
            });

        match &entry.model {
            Some(model) => {
                // AI message
                daily_stats_entry.ai_messages += 1;
                *daily_stats_entry
                    .models
                    .entry(model.to_string())
                    .or_insert(0) += 1;

                // Aggregate all stats
                daily_stats_entry.stats.cost += entry.stats.cost;
                daily_stats_entry.stats.input_tokens += entry.stats.input_tokens;
                daily_stats_entry.stats.output_tokens += entry.stats.output_tokens;
                daily_stats_entry.stats.reasoning_tokens += entry.stats.reasoning_tokens;
                daily_stats_entry.stats.cache_creation_tokens += entry.stats.cache_creation_tokens;
                daily_stats_entry.stats.cache_read_tokens += entry.stats.cache_read_tokens;
                daily_stats_entry.stats.cached_tokens += entry.stats.cached_tokens;
                daily_stats_entry.stats.tool_calls += entry.stats.tool_calls;
                daily_stats_entry.stats.terminal_commands += entry.stats.terminal_commands;
                daily_stats_entry.stats.file_searches += entry.stats.file_searches;
                daily_stats_entry.stats.file_content_searches += entry.stats.file_content_searches;
                daily_stats_entry.stats.files_read += entry.stats.files_read;
                daily_stats_entry.stats.files_added += entry.stats.files_added;
                daily_stats_entry.stats.files_edited += entry.stats.files_edited;
                daily_stats_entry.stats.files_deleted += entry.stats.files_deleted;
                daily_stats_entry.stats.lines_read += entry.stats.lines_read;
                daily_stats_entry.stats.lines_added += entry.stats.lines_added;
                daily_stats_entry.stats.lines_edited += entry.stats.lines_edited;
                daily_stats_entry.stats.lines_deleted += entry.stats.lines_deleted;
                daily_stats_entry.stats.bytes_read += entry.stats.bytes_read;
                daily_stats_entry.stats.bytes_added += entry.stats.bytes_added;
                daily_stats_entry.stats.bytes_edited += entry.stats.bytes_edited;
                daily_stats_entry.stats.bytes_deleted += entry.stats.bytes_deleted;
                daily_stats_entry.stats.todos_created += entry.stats.todos_created;
                daily_stats_entry.stats.todos_completed += entry.stats.todos_completed;
                daily_stats_entry.stats.todos_in_progress += entry.stats.todos_in_progress;
                daily_stats_entry.stats.todo_writes += entry.stats.todo_writes;
                daily_stats_entry.stats.todo_reads += entry.stats.todo_reads;
                daily_stats_entry.stats.code_lines += entry.stats.code_lines;
                daily_stats_entry.stats.docs_lines += entry.stats.docs_lines;
                daily_stats_entry.stats.data_lines += entry.stats.data_lines;
                daily_stats_entry.stats.media_lines += entry.stats.media_lines;
                daily_stats_entry.stats.config_lines += entry.stats.config_lines;
                daily_stats_entry.stats.other_lines += entry.stats.other_lines;
            }
            None => {
                // User message
                daily_stats_entry.user_messages += 1;

                // Aggregate user stats too (mostly todo-related)
                daily_stats_entry.stats.todos_created += entry.stats.todos_created;
                daily_stats_entry.stats.todos_completed += entry.stats.todos_completed;
                daily_stats_entry.stats.todos_in_progress += entry.stats.todos_in_progress;
                daily_stats_entry.stats.todo_writes += entry.stats.todo_writes;
                daily_stats_entry.stats.todo_reads += entry.stats.todo_reads;
            }
        };
    }

    // Track conversations started on each date and update daily stats
    for start_date in conversation_start_dates.values() {
        if let Some(daily_stats_entry) = daily_stats.get_mut(start_date) {
            daily_stats_entry.conversations += 1;
        }
    }

    // Reduce each day's timestamps to its active time, now that every entry has been seen.
    for (date, timestamps) in day_timestamps.iter_mut() {
        if let Some(daily_stats_entry) = daily_stats.get_mut(date) {
            daily_stats_entry.active_seconds = active_seconds_for_day(timestamps);
        }
    }

    // If there are any gaps (days Claude Code wasn't run) fill them in with
    // empty stats.  (TODO: This should be a utility.)
    if !daily_stats.is_empty() {
        let mut filled_stats = BTreeMap::new();

        let earliest_date = daily_stats.keys().min().unwrap();
        let today_str = chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string();
        let latest_date = daily_stats.keys().max().unwrap().max(&today_str); // Either today or the highest date in data.

        let start_date = match chrono::NaiveDate::parse_from_str(earliest_date, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => return daily_stats, // Ignore.
        };

        let end_date = match chrono::NaiveDate::parse_from_str(latest_date, "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => return daily_stats, // Ignore.
        };

        // Fill in the gaps.
        let mut current_date = start_date;
        while current_date <= end_date {
            let date_str = current_date.format("%Y-%m-%d").to_string();

            if let Some(existing_stats) = daily_stats.get(&date_str) {
                filled_stats.insert(date_str, existing_stats.clone());
            } else {
                filled_stats.insert(
                    date_str.clone(),
                    DailyStats {
                        date: date_str,
                        ..Default::default()
                    },
                );
            }

            current_date += chrono::Duration::days(1);
        }

        return filled_stats;
    }

    daily_stats
}

/// Filters messages to only include those created after a specific date
pub async fn get_messages_later_than(
    date: i64,
    messages: Vec<ConversationMessage>,
) -> Result<Vec<ConversationMessage>> {
    let mut messages_later_than_date = Vec::new();
    for msg in messages {
        if msg.date.timestamp_millis() >= date {
            messages_later_than_date.push(msg);
        }
    }

    Ok(messages_later_than_date)
}

pub fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text);
    format!("{:x}", hasher.finalize())
}

/// Custom serde deserializer for RFC3339 timestamp strings to DateTime<Utc>
pub fn deserialize_utc_timestamp<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.into())
        .map_err(serde::de::Error::custom)
}

pub fn truncate_project_label(label: &str, max_len: usize) -> String {
    let mut result = String::new();
    if label.len() <= max_len {
        result.push_str(label);
    } else {
        let hash = hash_text(label);
        let short_hash = &hash[..4]; // 4 characters for hash

        // max_len - (ellipsis char + hash chars)
        let take_chars = max_len.saturating_sub(5);
        let truncated_prefix = label.chars().take(take_chars).collect::<String>();

        result.push_str(&truncated_prefix);
        result.push('…');
        result.push_str(short_hash);
    }
    // Pad with spaces to ensure fixed width
    format!("{:width$}", result, width = max_len)
}

/// Calculate overall health percentage (0.0 to 1.0) based on output token usage
/// Daily target: 250,000 output tokens, Weekly target: 1,750,000 output tokens
pub fn calculate_overall_health(messages: &[ConversationMessage], current_date: &str) -> f64 {
    // Parse current date
    let current_date_parsed = chrono::NaiveDate::parse_from_str(current_date, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());

    let mut daily_output_tokens = 0u64;
    let mut weekly_output_tokens = 0u64;

    for message in messages {
        let message_date = message.date.date_naive();
        let output_tokens = message.stats.output_tokens;

        // Daily usage (same day) - only output tokens
        if message_date == current_date_parsed {
            daily_output_tokens += output_tokens;
        }

        // Weekly usage (last 7 days) - only output tokens
        let days_diff = (current_date_parsed - message_date).num_days();
        if days_diff >= 0 && days_diff < 7 {
            weekly_output_tokens += output_tokens;
        }
    }

    const DAILY_TARGET: u64 = 250_000;
    const WEEKLY_TARGET: u64 = 1_750_000; // 250K * 7 days

    let daily_health = 1.0 - (daily_output_tokens as f64 / DAILY_TARGET as f64).min(1.0);
    let weekly_health = 1.0 - (weekly_output_tokens as f64 / WEEKLY_TARGET as f64).min(1.0);

    // Return the more restrictive health value
    daily_health.min(weekly_health).max(0.0)
}

/// Get health status description
pub fn get_health_status(health: f64) -> &'static str {
    match health {
        h if h >= 0.8 => "Healthy",
        h if h >= 0.6 => "Good",
        h if h >= 0.4 => "Caution",
        h if h >= 0.2 => "Warning",
        _ => "Critical",
    }
}

/// Get health color for display
pub fn get_health_color(health: f64) -> &'static str {
    match health {
        h if h >= 0.8 => "green",
        h if h >= 0.6 => "yellow",
        h if h >= 0.4 => "orange",
        h if h >= 0.2 => "red",
        _ => "red",
    }
}

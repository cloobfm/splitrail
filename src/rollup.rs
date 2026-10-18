//! Rolling other machines' usage into this dashboard, over the MQTT broker (see `mqtt`).
//!
//! Every publisher (`splitrail-dashboard mqtt` on a devbox, or a dashboard) also sends, retained:
//!
//! * `splitrail/<host>/daily`: each tool's daily totals for the last 30 days.
//! * `splitrail/<host>/recent`: each tool's messages from the last hour, numbers only (plus its
//!   newest message, however old, for "last active"), and with `share_previews` the newest
//!   message's text, truncated and with anything that looks like a secret redacted.
//! * `splitrail/<host>/limits`: plan limit snapshots, with the login replaced by a hash.
//!
//! The dashboard subscribes to every other host's topics and appends a column per remote tool
//! (`Claude@dev-a`), built from those: daily totals feed the summary, recent messages
//! feed tk/s, the sparklines and activity state. Remote limit snapshots for an account this
//! machine also uses replace the local one when they are newer (`plan_limits`), since a limit
//! belongs to the account, not to the machine that last saw it.

use crate::config::MqttConfig;
use crate::types::{
    AgenticCodingToolStats, Application, ConversationMessage, DailyStats, MessageRole,
    MultiAnalyzerStats, Stats,
};
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::watch;

/// Days of daily totals published.
const DAILY_DAYS: i64 = 30;
/// Messages this recent are published in full (numbers only).
const RECENT_SECS: i64 = 3600;
const PREVIEW_CHARS: usize = 120;
/// Messages this recent are sent one by one; older ones are merged per bucket.
const RAW_SECS: i64 = 120;
/// Texts shared per tool, for the verbose view's last few lines (it shows 5).
const RECENT_TEXTS: usize = 5;
/// The sparkline's slot width.
const BUCKET_MS: i64 = 30_000;
/// Limit snapshots older than this are not published (they are ignored when read anyway).
const LIMITS_MAX_AGE_SECS: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DailyPayload {
    pub host: String,
    pub tools: BTreeMap<String, BTreeMap<String, DailyStats>>,
}

/// One message, numbers only. Short keys: there are up to a few hundred per tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecentMessage {
    /// Epoch milliseconds.
    pub t: i64,
    /// Assistant (true) or user.
    pub a: bool,
    #[serde(default)]
    pub i: u64,
    #[serde(default)]
    pub o: u64,
    #[serde(default)]
    pub r: u64,
    #[serde(default)]
    pub cr: u64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tc: u32,
    #[serde(default)]
    pub conv: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Project label, for the `#project` column in the verbose view. Publishers before 0.97
    /// leave it out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub p: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecentPayload {
    pub host: String,
    pub tools: BTreeMap<String, Vec<RecentMessage>>,
    /// Tool -> its newest message's text, when the publisher shares previews. Kept for
    /// dashboards before 0.98; newer ones read `texts`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub previews: BTreeMap<String, String>,
    /// Tool -> its last few messages with text, oldest first, when the publisher shares
    /// previews. Sent apart from `tools`, whose older entries are merged into time buckets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, Vec<RecentText>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecentText {
    /// Epoch milliseconds.
    pub t: i64,
    /// Assistant (true) or user.
    pub a: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub p: String,
    #[serde(default)]
    pub conv: String,
    pub text: String,
}

pub fn daily_payload(host: &str, stats: &MultiAnalyzerStats, now: DateTime<Utc>) -> DailyPayload {
    let since = (now.with_timezone(&Local).date_naive() - ChronoDuration::days(DAILY_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    let tools = stats
        .analyzer_stats
        .iter()
        .filter_map(|tool| {
            let days: BTreeMap<String, DailyStats> = tool
                .daily_stats
                .range(since.clone()..)
                .filter(|(_, day)| day.user_messages + day.ai_messages > 0 || day.stats.cost > 0.0)
                .map(|(date, day)| (date.clone(), day.clone()))
                .collect();
            (!days.is_empty()).then(|| (tool.analyzer_name.clone(), days))
        })
        .collect();
    DailyPayload {
        host: host.to_string(),
        tools,
    }
}

fn recent_message(message: &ConversationMessage) -> RecentMessage {
    RecentMessage {
        t: message.date.timestamp_millis(),
        a: message.role == MessageRole::Assistant,
        i: message.stats.input_tokens,
        o: message.stats.output_tokens,
        r: message.stats.reasoning_tokens,
        cr: message.stats.cache_read_tokens,
        cost: message.stats.cost,
        tc: message.stats.tool_calls,
        conv: message.conversation_hash.chars().take(16).collect(),
        model: message.model.clone(),
        p: message.project_hash.clone(),
    }
}

/// Messages from the last `RAW_SECS` as they are (activity state looks at single messages);
/// older ones merged into one per `BUCKET_MS` slot, which is all the sparkline and tk/s need.
/// A busy hour is hundreds of messages; this keeps it to at most ~120 entries per tool.
fn compact(messages: &[&ConversationMessage], now: DateTime<Utc>) -> Vec<RecentMessage> {
    let raw_since = (now - ChronoDuration::seconds(RAW_SECS)).timestamp_millis();
    let mut out: Vec<RecentMessage> = Vec::new();
    for message in messages {
        let m = recent_message(message);
        if m.t >= raw_since {
            out.push(m);
            continue;
        }
        let slot = m.t - m.t.rem_euclid(BUCKET_MS);
        match out.last_mut() {
            Some(last) if last.t == slot => {
                last.a |= m.a;
                last.i += m.i;
                last.o += m.o;
                last.r += m.r;
                last.cr += m.cr;
                last.cost += m.cost;
                last.tc += m.tc;
            }
            _ => out.push(RecentMessage { t: slot, ..m }),
        }
    }
    out
}

pub fn recent_payload(
    host: &str,
    stats: &MultiAnalyzerStats,
    now: DateTime<Utc>,
    share_previews: bool,
) -> RecentPayload {
    let since = now - ChronoDuration::seconds(RECENT_SECS);
    let mut payload = RecentPayload {
        host: host.to_string(),
        ..Default::default()
    };
    for tool in &stats.analyzer_stats {
        let Some(newest) = tool
            .messages
            .iter()
            .filter(|m| m.date <= now)
            .max_by_key(|m| m.date)
        else {
            continue;
        };
        let mut recent: Vec<&ConversationMessage> = tool
            .messages
            .iter()
            .filter(|m| m.date >= since && m.date <= now)
            .collect();
        if recent.is_empty() {
            recent.push(newest);
        }
        recent.sort_by_key(|m| m.date);
        payload
            .tools
            .insert(tool.analyzer_name.clone(), compact(&recent, now));
        if share_previews {
            let mut with_text: Vec<&ConversationMessage> = tool
                .messages
                .iter()
                .filter(|m| {
                    m.date <= now && m.content.as_deref().is_some_and(|c| !c.trim().is_empty())
                })
                .collect();
            with_text.sort_by_key(|m| m.date);
            let last = &with_text[with_text.len().saturating_sub(RECENT_TEXTS)..];
            if let Some(newest) = last.last().and_then(|m| m.content.as_deref()) {
                payload
                    .previews
                    .insert(tool.analyzer_name.clone(), preview(newest));
                payload.texts.insert(
                    tool.analyzer_name.clone(),
                    last.iter()
                        .map(|m| RecentText {
                            t: m.date.timestamp_millis(),
                            a: m.role == MessageRole::Assistant,
                            p: m.project_hash.clone(),
                            conv: m.conversation_hash.chars().take(16).collect(),
                            text: preview(m.content.as_deref().unwrap_or_default()),
                        })
                        .collect(),
                );
            }
        }
    }
    payload
}

/// A token that looks like a credential: a known key prefix, or a long unbroken run of key-ish
/// characters (hex, base64, JWT).
fn looks_secret(token: &str) -> bool {
    let word = token.trim_matches(|c: char| "\"'`,;:()[]{}<>".contains(c));
    const PREFIXES: [&str; 12] = [
        "sk-",
        "sk_",
        "pk_",
        "rk_",
        "ghp_",
        "gho_",
        "ghs_",
        "github_pat_",
        "xox",
        "AKIA",
        "eyJ",
        "AIza",
    ];
    if PREFIXES.iter().any(|p| word.starts_with(p)) && word.len() >= 12 {
        return true;
    }
    let keyish = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c));
    let has_digit = word.chars().any(|c| c.is_ascii_digit());
    let has_alpha = word.chars().any(|c| c.is_ascii_alphabetic());
    keyish && has_digit && has_alpha && word.len() >= 32 && !word.contains('/')
}

/// One line, at most `PREVIEW_CHARS`, secrets replaced by `[redacted]`; a token after
/// `Bearer`, `token=`, `password=` and the like is redacted whatever it looks like.
pub fn preview(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    for token in text.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if redact_next || looks_secret(token) {
            out.push("[redacted]".into());
            redact_next = false;
            continue;
        }
        if let Some((key, value)) = token.split_once(['=', ':'])
            && [
                "token", "password", "passwd", "secret", "api_key", "apikey", "key",
            ]
            .iter()
            .any(|k| key.to_ascii_lowercase().ends_with(k))
            && !value.is_empty()
        {
            out.push(format!("{key}=[redacted]"));
            continue;
        }
        redact_next = lower == "bearer" || lower == "authorization:";
        out.push(token.to_string());
    }
    let line = out.join(" ");
    match line.char_indices().nth(PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

pub fn login_hash(login: &str) -> String {
    crate::utils::hash_text(&login.trim().to_ascii_lowercase())[..16].to_string()
}

/// This machine's limit snapshots as published: what `plan_limits` reads, with the login
/// swapped for its hash (enough to match accounts across machines without sending the address).
pub fn limits_payload(now: f64) -> BTreeMap<String, Value> {
    let Some(dir) = std::env::home_dir().map(|h| h.join(".config/splitrail/limits")) else {
        return BTreeMap::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let label = path.file_stem()?.to_str()?.to_string();
            if path.extension()? != "json" {
                return None;
            }
            let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            Some((label, reduce_snapshot(&raw, now)?))
        })
        .collect()
}

pub fn reduce_snapshot(raw: &Value, now: f64) -> Option<Value> {
    let captured_at = raw.get("captured_at")?.as_f64()?;
    if now - captured_at > LIMITS_MAX_AGE_SECS {
        return None;
    }
    let mut out = serde_json::Map::new();
    out.insert("captured_at".into(), captured_at.into());
    for key in ["rate_limits", "monthly"] {
        if let Some(value) = raw.get(key).filter(|v| !v.is_null()) {
            out.insert(key.into(), value.clone());
        }
    }
    if let Some(login) = raw.get("login").and_then(Value::as_str) {
        out.insert("login_hash".into(), login_hash(login).into());
    }
    // The short name the dashboard shows (`support·team`), never the full address.
    let name = raw
        .get("login_name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .or_else(|| raw.get("login")?.as_str()?.split('@').next());
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        out.insert("login_name".into(), name.into());
    }
    Some(Value::Object(out))
}

// --- Subscriber ------------------------------------------------------------------------------

#[derive(Default)]
struct Remote {
    daily: DailyPayload,
    recent: RecentPayload,
}

static REMOTES: OnceLock<Mutex<HashMap<String, Remote>>> = OnceLock::new();

fn remotes() -> &'static Mutex<HashMap<String, Remote>> {
    REMOTES.get_or_init(Default::default)
}

/// The application a remote tool's messages are attributed to, from its column name.
fn application_for(tool: &str) -> Application {
    let name = tool.to_ascii_lowercase();
    match () {
        _ if name.starts_with("claude") => Application::ClaudeCode,
        _ if name.starts_with("codex") => Application::CodexCli,
        _ if name.starts_with("opencode") => Application::OpenCode,
        _ if name.starts_with("droid") => Application::DroidCli,
        _ if name.starts_with("cursor") => Application::CursorAgent,
        _ if name.starts_with("antigravity") => Application::Antigravity,
        _ if name.starts_with("ollama") => Application::Ollama,
        _ if name.starts_with("kilo cli") => Application::KiloCli,
        _ if name.starts_with("kilo") => Application::KiloCode,
        _ if name.starts_with("kiro") => Application::KiroCli,
        _ if name.starts_with("copilot cli") => Application::CopilotCli,
        _ if name.starts_with("pi agent") => Application::PiAgent,
        _ if name.starts_with("qwen") => Application::QwenCode,
        _ if name.starts_with("cline") => Application::Cline,
        _ if name.starts_with("warp") => Application::Warp,
        _ => Application::ClaudeCode,
    }
}

/// `slug`'s tools as columns, each registered so its limits and login can be found.
fn remote_tools(
    slug: &str,
    host: &str,
    daily: &DailyPayload,
    recent: &RecentPayload,
) -> Vec<AgenticCodingToolStats> {
    let mut names: Vec<&String> = daily.tools.keys().chain(recent.tools.keys()).collect();
    names.sort();
    names.dedup();
    // Two accounts of one tool (`Claude (main)`, `Claude (v)`) keep their full names.
    let shared_first_word = |tool: &str| {
        let first = tool.split_whitespace().next();
        names
            .iter()
            .filter(|n| n.split_whitespace().next() == first)
            .count()
            > 1
    };
    let columns: Vec<(&String, String)> = names
        .iter()
        .map(|tool| {
            let column = remote_name(tool, host, shared_first_word(tool));
            register_remote_column(&column, slug, tool);
            (*tool, column)
        })
        .collect();
    columns
        .into_iter()
        .map(|(tool, column)| {
            let daily_stats = daily.tools.get(tool).cloned().unwrap_or_default();
            let application = application_for(tool);
            let recent_messages = recent
                .tools
                .get(tool)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let newest = recent_messages.iter().map(|m| m.t).max();
            let texts = recent
                .texts
                .get(tool)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let text_for = |m: &RecentMessage| {
                texts
                    .iter()
                    .find(|x| x.t == m.t && x.a == m.a)
                    .map(|x| x.text.clone())
            };
            let mut messages: Vec<ConversationMessage> = recent_messages
                .iter()
                .filter_map(|m| {
                    Some(ConversationMessage {
                        application: application.clone(),
                        date: DateTime::from_timestamp_millis(m.t)?,
                        project_hash: m.p.clone(),
                        conversation_hash: m.conv.clone(),
                        local_hash: None,
                        global_hash: format!("remote:{host}:{tool}:{}:{}", m.t, m.o),
                        model: m.model.clone(),
                        stats: Stats {
                            input_tokens: m.i,
                            output_tokens: m.o,
                            reasoning_tokens: m.r,
                            cache_read_tokens: m.cr,
                            cached_tokens: m.cr,
                            cost: m.cost,
                            tool_calls: m.tc,
                            ..Default::default()
                        },
                        role: if m.a {
                            MessageRole::Assistant
                        } else {
                            MessageRole::User
                        },
                        content: if texts.is_empty() {
                            // A publisher before 0.98: only its newest message's text.
                            (Some(m.t) == newest)
                                .then(|| recent.previews.get(tool).cloned())
                                .flatten()
                        } else {
                            text_for(m)
                        },
                    })
                })
                .collect();
            // Texts whose message was merged into a time bucket: their own entry, with no
            // tokens, so the verbose view shows them without changing any count.
            for text in texts
                .iter()
                .filter(|x| !recent_messages.iter().any(|m| m.t == x.t && m.a == x.a))
            {
                let Some(date) = DateTime::from_timestamp_millis(text.t) else {
                    continue;
                };
                messages.push(ConversationMessage {
                    application: application.clone(),
                    date,
                    project_hash: text.p.clone(),
                    conversation_hash: text.conv.clone(),
                    local_hash: None,
                    global_hash: format!("remote:{host}:{tool}:{}:text", text.t),
                    model: None,
                    stats: Stats::default(),
                    role: if text.a {
                        MessageRole::Assistant
                    } else {
                        MessageRole::User
                    },
                    content: Some(text.text.clone()),
                });
            }
            messages.sort_by_key(|m| m.date);
            AgenticCodingToolStats {
                num_conversations: daily_stats
                    .values()
                    .map(|d| u64::from(d.conversations))
                    .sum(),
                daily_stats,
                messages,
                analyzer_name: column,
            }
        })
        .collect()
}

/// Width of a summary column; remote column names are fitted to it.
const COLUMN_CHARS: usize = 13;

/// A remote tool's column, fitted to the summary's 13-character headers: its first word and the
/// machine (`Claude@nas`), else with the machine shortened (`devbox-a` → `Claude@dev-a`), else the
/// tool abbreviated (`CC@devbox-a`), else the tool's initial with the machine cut in the middle.
/// `keep_label` keeps an account label (`Claude (v)` → `Cld(v)@dev-a`) for a tool with two
/// accounts.
pub fn remote_name(tool: &str, host: &str, keep_label: bool) -> String {
    let (base, label) = match tool.find('(').filter(|_| keep_label) {
        Some(i) => (tool[..i].trim(), tool[i..].replace(' ', "")),
        None => (tool, String::new()),
    };
    let first = base.split_whitespace().next().unwrap_or(base);
    let initial: String = base.chars().take(1).collect();
    let short_host = shorten_host(host);
    let first = format!("{first}{label}");
    let abbreviated = format!("{}{label}", abbreviate(base));
    let initial = format!("{initial}{label}");
    let candidates = [
        (&first, host),
        (&first, short_host.as_str()),
        (&abbreviated, host),
        (&abbreviated, short_host.as_str()),
        (&initial, short_host.as_str()),
    ];
    for (form, host) in candidates {
        let name = format!("{form}@{host}");
        if name.chars().count() <= COLUMN_CHARS {
            return name;
        }
    }
    let room = COLUMN_CHARS.saturating_sub(initial.chars().count() + 1);
    format!("{initial}@{}", squeeze(&short_host, room))
}

/// A machine name with the part before its last `-` cut to three characters, keeping the suffix
/// that tells siblings apart (`devbox-a` → `dev-a`); unchanged without a `-`.
fn shorten_host(host: &str) -> String {
    match host.rsplit_once('-') {
        Some((stem, suffix)) if !suffix.is_empty() => {
            format!("{}-{suffix}", stem.chars().take(3).collect::<String>())
        }
        _ => host.to_string(),
    }
}

/// Initials of a tool's words and inner capitals (`Claude Code` → `CC`, `OpenCode` → `OC`), or
/// for a one-word name its first letter and next consonants (`Codex` → `Cdx`).
fn abbreviate(tool: &str) -> String {
    let mut initials = String::new();
    for word in tool.split_whitespace() {
        let mut chars = word.chars();
        initials.extend(chars.next());
        initials.extend(chars.filter(|c| c.is_uppercase()));
    }
    if initials.chars().count() > 1 {
        return initials;
    }
    let mut chars = tool.chars();
    chars
        .next()
        .into_iter()
        .chain(chars.filter(|c| c.is_alphabetic() && !"aeiouAEIOU".contains(*c)))
        .take(3)
        .collect()
}

/// `text` cut to `room` characters by dropping its middle (`devbox-a`, 5 → `de…-a`).
fn squeeze(text: &str, room: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_string();
    }
    if room < 3 {
        return chars[..room].iter().collect();
    }
    let tail = (room - 1) / 2;
    let head = room - 1 - tail;
    chars[..head]
        .iter()
        .chain(['…'].iter())
        .chain(chars[chars.len() - tail..].iter())
        .collect()
}

/// `local` with a column appended for every remote tool.
pub fn merged(local: &MultiAnalyzerStats) -> MultiAnalyzerStats {
    let mut out = local.clone();
    let Ok(remotes) = remotes().lock() else {
        return out;
    };
    let mut hosts: Vec<_> = remotes.iter().collect();
    hosts.sort_by(|a, b| a.0.cmp(b.0));
    for (slug, remote) in hosts {
        let host = [&remote.daily.host, &remote.recent.host]
            .into_iter()
            .find(|h| !h.is_empty())
            .cloned()
            .unwrap_or_else(|| slug.clone());
        let tools = remote_tools(slug, &host, &remote.daily, &remote.recent);
        out.analyzer_stats.extend(tools.into_iter().map(Arc::new));
    }
    out
}

/// Remote column name -> the publishing machine's slug and the tool's own name there.
fn remote_columns() -> &'static Mutex<HashMap<String, (String, String)>> {
    static COLUMNS: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    COLUMNS.get_or_init(Default::default)
}

pub(crate) fn register_remote_column(column: &str, slug: &str, tool: &str) {
    if let Ok(mut columns) = remote_columns().lock() {
        columns.insert(column.to_string(), (slug.to_string(), tool.to_string()));
    }
}

/// Where a remote column (`Claude@dev-a`) comes from: the machine's slug and the tool's name
/// there (`Claude Code`); `None` for a local column.
pub fn remote_source(column: &str) -> Option<(String, String)> {
    remote_columns().lock().ok()?.get(column).cloned()
}

/// Apply one message from the broker; `true` if the dashboard needs a refresh.
fn apply(topic: &str, payload: &[u8], own_slug: &str) -> bool {
    let mut parts = topic.split('/');
    let (Some("splitrail"), Some(slug), Some(kind), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if slug == own_slug {
        return false;
    }
    match kind {
        "limits" => {
            let snapshots: BTreeMap<String, Value> =
                serde_json::from_slice(payload).unwrap_or_default();
            crate::plan_limits::set_remote_snapshots(slug, snapshots.into_values().collect());
            true
        }
        "daily" | "recent" => {
            let Ok(mut remotes) = remotes().lock() else {
                return false;
            };
            let remote = remotes.entry(slug.to_string()).or_default();
            if kind == "daily" {
                remote.daily = serde_json::from_slice(payload).unwrap_or_default();
            } else {
                remote.recent = serde_json::from_slice(payload).unwrap_or_default();
            }
            if remote.daily.tools.is_empty() && remote.recent.tools.is_empty() {
                remotes.remove(slug);
            }
            true
        }
        "state" => {
            let state: Value = serde_json::from_slice(payload).unwrap_or_default();
            let text = |key: &str| state.get(key).and_then(Value::as_str).map(str::to_string);
            update_publisher(slug, |p| {
                p.version = text("version");
                p.rev = text("publisher_rev");
            });
            false
        }
        "status" => {
            update_publisher(slug, |p| p.online = payload == b"online");
            false
        }
        _ => false,
    }
}

// --- Publishers ------------------------------------------------------------------------------

/// Where updating a machine's publisher from here stands.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PublisherUpdate {
    #[default]
    Idle,
    Updating,
    Failed,
}

/// Another machine publishing to the broker, as its state and status topics describe it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Publisher {
    pub slug: String,
    /// Its host name (`devbox-a`), which is also how it is reached over SSH.
    pub host: String,
    pub version: Option<String>,
    /// Fingerprint of its publishing code; `None` from builds before it was reported.
    pub rev: Option<String>,
    pub online: bool,
    pub update: PublisherUpdate,
}

impl Publisher {
    /// Running different publishing code from this build's, or another version: a release that
    /// changes only the dashboard still updates the publishers, so every machine shows the
    /// same version.
    pub fn behind(&self) -> bool {
        self.rev.as_deref() != Some(crate::mqtt::PUBLISHER_REV)
            || self.version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
    }
}

fn publishers_map() -> &'static Mutex<BTreeMap<String, Publisher>> {
    static PUBLISHERS: OnceLock<Mutex<BTreeMap<String, Publisher>>> = OnceLock::new();
    PUBLISHERS.get_or_init(Default::default)
}

fn update_publisher(slug: &str, change: impl FnOnce(&mut Publisher)) {
    if let Ok(mut map) = publishers_map().lock() {
        let entry = map.entry(slug.to_string()).or_insert_with(|| Publisher {
            slug: slug.to_string(),
            ..Default::default()
        });
        change(entry);
    }
}

pub fn set_publisher_update(slug: &str, update: PublisherUpdate) {
    update_publisher(slug, |p| p.update = update);
}

/// Every other machine seen on the broker, by slug, with its host name filled in from its
/// roll-up payloads (or the slug with `-` for `_`, before one arrives).
pub fn publishers() -> Vec<Publisher> {
    let mut list: Vec<Publisher> = publishers_map()
        .lock()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    let remotes = remotes().lock().ok();
    for publisher in &mut list {
        publisher.host = remotes
            .as_ref()
            .and_then(|r| r.get(&publisher.slug))
            .map(|r| {
                if r.daily.host.is_empty() {
                    r.recent.host.clone()
                } else {
                    r.daily.host.clone()
                }
            })
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| publisher.slug.replace('_', "-"));
    }
    list
}

/// A machine name as the title shows it (`devbox-a` → `dev-a`).
pub fn short_host(host: &str) -> String {
    shorten_host(host)
}

/// Subscribe to the other machines on the broker. `None` without a broker or with `roll_up`
/// off; otherwise a counter that ticks whenever remote data changed.
pub fn start(config: &MqttConfig, own_slug: String) -> Option<watch::Receiver<u64>> {
    let host = config.host.clone().filter(|h| !h.trim().is_empty())?;
    if !config.roll_up.unwrap_or(true) {
        return None;
    }
    let mut options = MqttOptions::new(
        format!("splitrail-rollup-{own_slug}-{}", std::process::id()),
        host,
        config.port.unwrap_or(1883),
    );
    options.set_keep_alive(Duration::from_secs(30));
    options.set_max_packet_size(4 * 1024 * 1024, 4 * 1024 * 1024);
    if let Some(username) = &config.username {
        options.set_credentials(username, crate::mqtt::password(config).unwrap_or_default());
    }
    let (client, mut eventloop) = AsyncClient::new(options, 64);
    let (tx, rx) = watch::channel(0u64);
    tokio::spawn(async move {
        let mut failing = false;
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    failing = false;
                    for kind in ["daily", "recent", "limits", "state", "status"] {
                        let _ =
                            client.try_subscribe(format!("splitrail/+/{kind}"), QoS::AtMostOnce);
                    }
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    if apply(&publish.topic, &publish.payload, &own_slug) {
                        tx.send_modify(|n| *n += 1);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    if !failing {
                        crate::utils::log_error(format!("MQTT roll-up: {e}"));
                        failing = true;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });
    Some(rx)
}

/// A stats channel carrying `local` plus every remote machine's columns, updated when either
/// changes.
pub fn merged_channel(
    mut local: watch::Receiver<MultiAnalyzerStats>,
    mut remote: watch::Receiver<u64>,
) -> watch::Receiver<MultiAnalyzerStats> {
    let (tx, rx) = watch::channel(merged(&local.borrow()));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = local.changed() => if changed.is_err() { break },
                changed = remote.changed() => if changed.is_err() { break },
            }
            let stats = merged(&local.borrow_and_update());
            remote.borrow_and_update();
            if tx.send(stats).is_err() {
                break;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        date: DateTime<Utc>,
        role: MessageRole,
        output: u64,
        content: Option<&str>,
    ) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date,
            project_hash: "p".into(),
            conversation_hash: "conversation-hash-0123456789".into(),
            local_hash: None,
            global_hash: format!("{date}{output}"),
            model: Some("claude-opus-5-5".into()),
            stats: Stats {
                output_tokens: output,
                cost: 0.5,
                ..Default::default()
            },
            role,
            content: content.map(str::to_string),
        }
    }

    fn day(messages: u32, cost: f64) -> DailyStats {
        DailyStats {
            ai_messages: messages,
            conversations: 1,
            stats: Stats {
                cost,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn stats(now: DateTime<Utc>) -> MultiAnalyzerStats {
        let today = now.with_timezone(&Local).date_naive();
        let date = |back: i64| {
            (today - ChronoDuration::days(back))
                .format("%Y-%m-%d")
                .to_string()
        };
        MultiAnalyzerStats {
            analyzer_stats: vec![
                Arc::new(AgenticCodingToolStats {
                    daily_stats: BTreeMap::from([
                        (date(0), day(3, 1.5)),
                        (date(40), day(9, 9.0)),
                        (date(1), day(0, 0.0)),
                    ]),
                    num_conversations: 2,
                    messages: vec![
                        message(
                            now - ChronoDuration::hours(3),
                            MessageRole::User,
                            0,
                            Some("old"),
                        ),
                        message(
                            now - ChronoDuration::minutes(5),
                            MessageRole::User,
                            0,
                            Some("fix it"),
                        ),
                        message(
                            now - ChronoDuration::minutes(4),
                            MessageRole::Assistant,
                            900,
                            Some("Done, key sk-abcdefghijklmnop1234 rotated"),
                        ),
                    ],
                    analyzer_name: "Claude Code".into(),
                }),
                Arc::new(AgenticCodingToolStats {
                    daily_stats: BTreeMap::new(),
                    num_conversations: 1,
                    messages: vec![message(
                        now - ChronoDuration::days(2),
                        MessageRole::Assistant,
                        10,
                        None,
                    )],
                    analyzer_name: "OpenCode".into(),
                }),
            ],
        }
    }

    #[test]
    fn daily_keeps_30_active_days() {
        let now = Utc::now();
        let payload = daily_payload("devbox-a", &stats(now), now);
        let claude = &payload.tools["Claude Code"];
        assert_eq!(
            claude.len(),
            1,
            "40 days back and the empty day are left out"
        );
        assert!(!payload.tools.contains_key("OpenCode"));
    }

    #[test]
    fn recent_sends_the_last_hour_or_the_newest() {
        let now = Utc::now();
        let payload = recent_payload("devbox-a", &stats(now), now, false);
        assert_eq!(payload.tools["Claude Code"].len(), 2);
        assert_eq!(
            payload.tools["OpenCode"].len(),
            1,
            "an idle tool still says when it last ran"
        );
        assert_eq!(payload.tools["Claude Code"][0].conv, "conversation-has");
        assert!(payload.previews.is_empty(), "no text unless sharing is on");

        let shared = recent_payload("devbox-a", &stats(now), now, true);
        assert_eq!(
            shared.previews["Claude Code"],
            "Done, key [redacted] rotated"
        );
    }

    #[test]
    fn older_messages_are_merged_per_slot() {
        let now = Utc::now();
        let at = |secs: i64| now - ChronoDuration::seconds(secs);
        // Two in one 30s slot 10 minutes back (aligned to the slot), two in the raw window.
        let slot_start = (at(600).timestamp_millis() / BUCKET_MS) * BUCKET_MS;
        let base = DateTime::from_timestamp_millis(slot_start).unwrap();
        let messages = [
            message(
                base + ChronoDuration::seconds(1),
                MessageRole::User,
                0,
                None,
            ),
            message(
                base + ChronoDuration::seconds(20),
                MessageRole::Assistant,
                300,
                None,
            ),
            message(at(30), MessageRole::User, 0, None),
            message(at(20), MessageRole::Assistant, 50, None),
        ];
        let refs: Vec<&ConversationMessage> = messages.iter().collect();
        let out = compact(&refs, now);
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].t, out[0].o, out[0].a), (slot_start, 300, true));
        assert_eq!(out[2].o, 50);
    }

    #[test]
    fn previews_redact_and_truncate() {
        assert_eq!(
            preview("export TOKEN=abc123 now"),
            "export TOKEN=[redacted] now"
        );
        assert_eq!(
            preview("curl -H 'Authorization: Bearer abc.def' x"),
            "curl -H 'Authorization: Bearer [redacted] x"
        );
        assert_eq!(
            preview("hash 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 ok"),
            "hash [redacted] ok"
        );
        assert_eq!(
            preview("see src/analyzers/antigravity_store.rs for it"),
            "see src/analyzers/antigravity_store.rs for it"
        );
        assert_eq!(
            preview(&"word ".repeat(100)).chars().count(),
            PREVIEW_CHARS + 1
        );
        assert_eq!(preview("two\nlines"), "two lines");
    }

    #[test]
    fn remote_columns_round_trip() {
        let now = Utc::now();
        let local = stats(now);
        let daily = daily_payload("devbox-a", &local, now);
        let recent = recent_payload("devbox-a", &local, now, true);
        // Over the wire and back.
        let daily: DailyPayload =
            serde_json::from_str(&serde_json::to_string(&daily).unwrap()).unwrap();
        let recent: RecentPayload =
            serde_json::from_str(&serde_json::to_string(&recent).unwrap()).unwrap();
        let tools = remote_tools("devbox_a", "devbox-a", &daily, &recent);
        let claude = tools
            .iter()
            .find(|t| t.analyzer_name == "Claude@dev-a")
            .unwrap();
        assert_eq!(claude.daily_stats.len(), 1);
        // The last messages with text all arrive (the verbose view shows 5), oldest first,
        // without changing the tokens: they are older than the raw window, so their tokens
        // sit in time buckets and the texts come as entries of their own.
        let texts: Vec<&str> = claude
            .messages
            .iter()
            .filter_map(|m| m.content.as_deref())
            .collect();
        assert_eq!(texts, ["old", "fix it", "Done, key [redacted] rotated"]);
        assert_eq!(
            claude
                .messages
                .iter()
                .map(|m| m.stats.output_tokens)
                .sum::<u64>(),
            900
        );
        assert_eq!(
            claude
                .messages
                .iter()
                .filter(|m| m.stats.output_tokens > 0)
                .count(),
            1
        );
        // The project travels too, for the verbose view's `#project` column.
        assert!(claude.messages.iter().all(|m| m.project_hash == "p"));
        assert!(tools.iter().any(|t| t.analyzer_name == "OC@devbox-a"));
    }

    #[test]
    fn a_fresh_message_carries_its_own_text_once() {
        let now = Utc::now();
        let local = MultiAnalyzerStats {
            analyzer_stats: vec![Arc::new(AgenticCodingToolStats {
                daily_stats: BTreeMap::new(),
                num_conversations: 1,
                messages: vec![message(
                    now - ChronoDuration::seconds(30),
                    MessageRole::Assistant,
                    50,
                    Some("Now a JS parity test."),
                )],
                analyzer_name: "Claude Code".into(),
            })],
        };
        let daily = daily_payload("devbox-a", &local, now);
        let recent: RecentPayload = serde_json::from_str(
            &serde_json::to_string(&recent_payload("devbox-a", &local, now, true)).unwrap(),
        )
        .unwrap();
        let tools = remote_tools("devbox_a", "devbox-a", &daily, &recent);
        let claude = &tools[0];
        assert_eq!(claude.messages.len(), 1);
        assert_eq!(
            claude.messages[0].content.as_deref(),
            Some("Now a JS parity test.")
        );
        assert_eq!(claude.messages[0].stats.output_tokens, 50);
    }

    #[test]
    fn remote_names_fit_a_column() {
        assert_eq!(
            remote_name("Claude Code", "devbox-a", false),
            "Claude@dev-a"
        );
        assert_eq!(remote_name("OpenCode", "devbox-a", false), "OC@devbox-a");
        assert_eq!(remote_name("Codex", "devbox-a", false), "Codex@dev-a");
        assert_eq!(remote_name("Claude Code", "nas", false), "Claude@nas");
        assert_eq!(
            remote_name("Claude Code", "buildserver", false),
            "C@buildserver"
        );
        assert_eq!(remote_name("Claude (v)", "devbox-a", true), "Cld(v)@dev-a");
        for name in [
            remote_name("Claude Code", "a-very-long-hostname", false),
            remote_name("Claude Code", "averylonghostname", false),
            remote_name("Claude (main)", "devbox-a", true),
        ] {
            assert!(name.chars().count() <= COLUMN_CHARS, "{name}");
        }
    }

    #[test]
    fn publishers_report_version_rev_and_status() {
        let state = format!(
            r#"{{"version": "{}", "publisher_rev": "{}", "tks": 1.0}}"#,
            env!("CARGO_PKG_VERSION"),
            crate::mqtt::PUBLISHER_REV
        );
        apply("splitrail/pub_test/state", state.as_bytes(), "me");
        apply("splitrail/pub_test/status", b"online", "me");
        apply("splitrail/pub_old/state", br#"{"tks": 0.0}"#, "me");
        let all = publishers();
        let current = all.iter().find(|p| p.slug == "pub_test").unwrap();
        assert_eq!(
            (
                current.host.as_str(),
                current.version.as_deref(),
                current.online
            ),
            ("pub-test", Some(env!("CARGO_PKG_VERSION")), true)
        );
        assert!(!current.behind());
        // Same code, older version: still updated, so the versions match.
        let older = format!(
            r#"{{"version": "2.0.0-dash-0.1", "publisher_rev": "{}"}}"#,
            crate::mqtt::PUBLISHER_REV
        );
        apply("splitrail/pub_ver/state", older.as_bytes(), "me");
        assert!(
            publishers()
                .iter()
                .find(|p| p.slug == "pub_ver")
                .unwrap()
                .behind()
        );
        let old = all.iter().find(|p| p.slug == "pub_old").unwrap();
        assert!(
            old.behind() && !old.online,
            "no fingerprint: an older build"
        );
        apply("splitrail/pub_test/status", b"offline", "me");
        assert!(
            !publishers()
                .iter()
                .find(|p| p.slug == "pub_test")
                .unwrap()
                .online
        );
    }

    #[test]
    fn own_and_foreign_topics_are_ignored() {
        assert!(!apply("splitrail/me/daily", b"{}", "me"));
        assert!(!apply("homeassistant/sensor/x/config", b"{}", "me"));
        assert!(!apply("splitrail/other/state", b"{}", "me"));
    }

    #[test]
    fn snapshots_keep_only_the_short_login_name() {
        let raw = serde_json::json!({
            "captured_at": 1000.0, "login": "Me@X.com", "login_name": "me",
            "rate_limits": {"five_hour": {"used_percentage": 25.0, "resets_at": 5000.0}},
            "sessions": {"a": 1},
        });
        let reduced = reduce_snapshot(&raw, 2000.0).unwrap();
        assert_eq!(reduced["login_hash"], login_hash("me@x.com"));
        assert!(reduced.get("login").is_none() && reduced.get("sessions").is_none());
        assert_eq!(reduced["login_name"], "me");
        let unnamed = serde_json::json!({"captured_at": 1000.0, "login": "someone@x.com"});
        assert_eq!(
            reduce_snapshot(&unnamed, 2000.0).unwrap()["login_name"],
            "someone"
        );
        assert!(reduce_snapshot(&raw, 1000.0 + LIMITS_MAX_AGE_SECS + 1.0).is_none());
    }
}

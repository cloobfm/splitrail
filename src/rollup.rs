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
//! (`CC@devbox-a`), built from those: daily totals feed the summary, recent messages
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecentPayload {
    pub host: String,
    pub tools: BTreeMap<String, Vec<RecentMessage>>,
    /// Tool -> its newest message's text, when the publisher shares previews.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub previews: BTreeMap<String, String>,
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
    DailyPayload { host: host.to_string(), tools }
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
    let mut payload = RecentPayload { host: host.to_string(), ..Default::default() };
    for tool in &stats.analyzer_stats {
        let Some(newest) = tool.messages.iter().filter(|m| m.date <= now).max_by_key(|m| m.date) else {
            continue;
        };
        let mut recent: Vec<&ConversationMessage> =
            tool.messages.iter().filter(|m| m.date >= since && m.date <= now).collect();
        if recent.is_empty() {
            recent.push(newest);
        }
        recent.sort_by_key(|m| m.date);
        payload
            .tools
            .insert(tool.analyzer_name.clone(), compact(&recent, now));
        if share_previews
            && let Some(text) = tool
                .messages
                .iter()
                .filter(|m| m.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
                .max_by_key(|m| m.date)
                .and_then(|m| m.content.as_deref())
        {
            payload.previews.insert(tool.analyzer_name.clone(), preview(text));
        }
    }
    payload
}

/// A token that looks like a credential: a known key prefix, or a long unbroken run of key-ish
/// characters (hex, base64, JWT).
fn looks_secret(token: &str) -> bool {
    let word = token.trim_matches(|c: char| "\"'`,;:()[]{}<>".contains(c));
    const PREFIXES: [&str; 12] = [
        "sk-", "sk_", "pk_", "rk_", "ghp_", "gho_", "ghs_", "github_pat_", "xox", "AKIA", "eyJ",
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
            && ["token", "password", "passwd", "secret", "api_key", "apikey", "key"]
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
        _ if name.starts_with("kilo") => Application::KiloCode,
        _ if name.starts_with("kiro") => Application::KiroCli,
        _ if name.starts_with("qwen") => Application::QwenCode,
        _ if name.starts_with("cline") => Application::Cline,
        _ if name.starts_with("warp") => Application::Warp,
        _ => Application::ClaudeCode,
    }
}

/// `slug`'s tools as columns, each registered so its limits and login can be found.
fn remote_tools(slug: &str, host: &str, daily: &DailyPayload, recent: &RecentPayload) -> Vec<AgenticCodingToolStats> {
    let mut names: Vec<&String> = daily.tools.keys().chain(recent.tools.keys()).collect();
    names.sort();
    names.dedup();
    // Two accounts of one tool (`Claude (main)`, `Claude (v)`) keep their full names.
    let shared_first_word = |tool: &str| {
        let first = tool.split_whitespace().next();
        names.iter().filter(|n| n.split_whitespace().next() == first).count() > 1
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
            let recent_messages = recent.tools.get(tool).map(Vec::as_slice).unwrap_or_default();
            let newest = recent_messages.iter().map(|m| m.t).max();
            let messages = recent_messages
                .iter()
                .filter_map(|m| {
                    Some(ConversationMessage {
                        application: application.clone(),
                        date: DateTime::from_timestamp_millis(m.t)?,
                        project_hash: String::new(),
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
                        role: if m.a { MessageRole::Assistant } else { MessageRole::User },
                        content: (Some(m.t) == newest)
                            .then(|| recent.previews.get(tool).cloned())
                            .flatten(),
                    })
                })
                .collect();
            AgenticCodingToolStats {
                num_conversations: daily_stats.values().map(|d| u64::from(d.conversations)).sum(),
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
/// machine (`Claude@nas`), else the tool abbreviated (`Claude Code` → `CC@devbox-a`), else the
/// tool's initial with the machine shortened in the middle. `keep_label` keeps an account label
/// (`Claude (v)` → `C(v)@devbox-a`) for a tool with two accounts.
pub fn remote_name(tool: &str, host: &str, keep_label: bool) -> String {
    let (base, label) = match tool.find('(').filter(|_| keep_label) {
        Some(i) => (tool[..i].trim(), tool[i..].replace(' ', "")),
        None => (tool, String::new()),
    };
    let first = base.split_whitespace().next().unwrap_or(base);
    let initial: String = base.chars().take(1).collect();
    let forms = [
        format!("{first}{label}"),
        format!("{}{label}", abbreviate(base)),
        format!("{initial}{label}"),
    ];
    for form in &forms {
        let name = format!("{form}@{host}");
        if name.chars().count() <= COLUMN_CHARS {
            return name;
        }
    }
    let form = &forms[2];
    let room = COLUMN_CHARS.saturating_sub(form.chars().count() + 1);
    format!("{form}@{}", squeeze(host, room))
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
    chars[..head].iter().chain(['…'].iter()).chain(chars[chars.len() - tail..].iter()).collect()
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

/// Where a remote column (`CC@devbox-a`) comes from: the machine's slug and the tool's name
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
        _ => false,
    }
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
                    for kind in ["daily", "recent", "limits"] {
                        let _ = client.try_subscribe(format!("splitrail/+/{kind}"), QoS::AtMostOnce);
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

    fn message(date: DateTime<Utc>, role: MessageRole, output: u64, content: Option<&str>) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date,
            project_hash: "p".into(),
            conversation_hash: "conversation-hash-0123456789".into(),
            local_hash: None,
            global_hash: format!("{date}{output}"),
            model: Some("claude-opus-5-5".into()),
            stats: Stats { output_tokens: output, cost: 0.5, ..Default::default() },
            role,
            content: content.map(str::to_string),
        }
    }

    fn day(messages: u32, cost: f64) -> DailyStats {
        DailyStats {
            ai_messages: messages,
            conversations: 1,
            stats: Stats { cost, ..Default::default() },
            ..Default::default()
        }
    }

    fn stats(now: DateTime<Utc>) -> MultiAnalyzerStats {
        let today = now.with_timezone(&Local).date_naive();
        let date = |back: i64| (today - ChronoDuration::days(back)).format("%Y-%m-%d").to_string();
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
                        message(now - ChronoDuration::hours(3), MessageRole::User, 0, Some("old")),
                        message(now - ChronoDuration::minutes(5), MessageRole::User, 0, Some("fix it")),
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
                    messages: vec![message(now - ChronoDuration::days(2), MessageRole::Assistant, 10, None)],
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
        assert_eq!(claude.len(), 1, "40 days back and the empty day are left out");
        assert!(!payload.tools.contains_key("OpenCode"));
    }

    #[test]
    fn recent_sends_the_last_hour_or_the_newest() {
        let now = Utc::now();
        let payload = recent_payload("devbox-a", &stats(now), now, false);
        assert_eq!(payload.tools["Claude Code"].len(), 2);
        assert_eq!(payload.tools["OpenCode"].len(), 1, "an idle tool still says when it last ran");
        assert_eq!(payload.tools["Claude Code"][0].conv, "conversation-has");
        assert!(payload.previews.is_empty(), "no text unless sharing is on");

        let shared = recent_payload("devbox-a", &stats(now), now, true);
        assert_eq!(shared.previews["Claude Code"], "Done, key [redacted] rotated");
    }

    #[test]
    fn older_messages_are_merged_per_slot() {
        let now = Utc::now();
        let at = |secs: i64| now - ChronoDuration::seconds(secs);
        // Two in one 30s slot 10 minutes back (aligned to the slot), two in the raw window.
        let slot_start = (at(600).timestamp_millis() / BUCKET_MS) * BUCKET_MS;
        let base = DateTime::from_timestamp_millis(slot_start).unwrap();
        let messages = [
            message(base + ChronoDuration::seconds(1), MessageRole::User, 0, None),
            message(base + ChronoDuration::seconds(20), MessageRole::Assistant, 300, None),
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
        assert_eq!(preview("export TOKEN=abc123 now"), "export TOKEN=[redacted] now");
        assert_eq!(preview("curl -H 'Authorization: Bearer abc.def' x"), "curl -H 'Authorization: Bearer [redacted] x");
        assert_eq!(
            preview("hash 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 ok"),
            "hash [redacted] ok"
        );
        assert_eq!(preview("see src/analyzers/antigravity_store.rs for it"), "see src/analyzers/antigravity_store.rs for it");
        assert_eq!(preview(&"word ".repeat(100)).chars().count(), PREVIEW_CHARS + 1);
        assert_eq!(preview("two\nlines"), "two lines");
    }

    #[test]
    fn remote_columns_round_trip() {
        let now = Utc::now();
        let local = stats(now);
        let daily = daily_payload("devbox-a", &local, now);
        let recent = recent_payload("devbox-a", &local, now, true);
        // Over the wire and back.
        let daily: DailyPayload = serde_json::from_str(&serde_json::to_string(&daily).unwrap()).unwrap();
        let recent: RecentPayload = serde_json::from_str(&serde_json::to_string(&recent).unwrap()).unwrap();
        let tools = remote_tools("devbox_a", "devbox-a", &daily, &recent);
        let claude = tools.iter().find(|t| t.analyzer_name == "CC@devbox-a").unwrap();
        assert_eq!(claude.daily_stats.len(), 1);
        assert_eq!(claude.messages.len(), 2);
        assert_eq!(claude.messages[1].stats.output_tokens, 900);
        assert_eq!(claude.messages[1].content.as_deref(), Some("Done, key [redacted] rotated"));
        assert_eq!(claude.messages[0].content, None);
        assert!(tools.iter().any(|t| t.analyzer_name == "OC@devbox-a"));
    }

    #[test]
    fn remote_names_fit_a_column() {
        assert_eq!(remote_name("Codex", "devbox-a", false), "Cdx@devbox-a");
        assert_eq!(remote_name("Claude Code", "devbox-a", false), "CC@devbox-a");
        assert_eq!(remote_name("OpenCode", "devbox-a", false), "OC@devbox-a");
        assert_eq!(remote_name("Claude Code", "nas", false), "Claude@nas");
        assert_eq!(remote_name("Claude (v)", "devbox-a", true), "C(v)@devbox-a");
        assert_eq!(remote_name("Claude (main)", "devbox-a", true), "C(main)@de…-a");
        for name in [remote_name("Claude Code", "a-very-long-hostname", false), remote_name("Claude (main)", "devbox-a", true)] {
            assert!(name.chars().count() <= COLUMN_CHARS, "{name}");
        }
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
        assert_eq!(reduce_snapshot(&unnamed, 2000.0).unwrap()["login_name"], "someone");
        assert!(reduce_snapshot(&raw, 1000.0 + LIMITS_MAX_AGE_SECS + 1.0).is_none());
    }
}

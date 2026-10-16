//! Claude plan-limit snapshots captured by the status line (BZL-66), and health derived from
//! them (BZL-65).
//!
//! Transcripts never record plan usage. Claude Code hands it to the `statusLine` command instead,
//! and `scripts/claude_statusline_capture.py` saves the latest payload per account to
//! `~/.config/splitrail/limits/<label>.json`:
//!
//! ```json
//! {"account": "main", "captured_at": 1790519606.1,
//!  "rate_limits": {"five_hour": {"used_percentage": 23, "resets_at": 1790525400},
//!                  "seven_day": {"used_percentage": 16, "resets_at": 1791050400}}}
//! ```
//!
//! One config directory can be logged in to different accounts over time, so the script keeps
//! limits per login (`logins`) and puts the current login's in `rate_limits`, naming it in
//! `login`.
//!
//! A snapshot is only as fresh as the last time that account's status line ran, so a window
//! whose reset time has passed is read as empty rather than at its last-seen usage.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Snapshots older than this are ignored: after a week even the weekly window has rolled over.
const MAX_SNAPSHOT_AGE_SECS: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct LimitWindow {
    #[serde(alias = "utilization")]
    pub used_percentage: Option<f64>,
    pub resets_at: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RateLimits {
    five_hour: Option<LimitWindow>,
    seven_day: Option<LimitWindow>,
}

#[derive(Debug, Clone, Deserialize)]
struct SnapshotFile {
    captured_at: f64,
    rate_limits: Option<RateLimits>,
    /// A monthly allowance, for tools billed by the month (Cursor, Droid) rather than windows.
    #[serde(default)]
    monthly: Option<LimitWindow>,
    #[serde(default)]
    login: Option<String>,
    /// Short display name, with `·team` for a team organization; older snapshots lack it.
    #[serde(default)]
    login_name: Option<String>,
}

/// One window as it stands now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowStatus {
    /// 0-100. Zero once the window has reset since the snapshot.
    pub used_percentage: f64,
    /// Seconds until the window resets; `None` when it already reset and the next reset time is
    /// not known yet (it becomes known when the status line next runs).
    pub resets_in_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccountLimits {
    pub five_hour: Option<WindowStatus>,
    pub seven_day: Option<WindowStatus>,
    pub monthly: Option<WindowStatus>,
}

impl AccountLimits {
    fn any(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some() || self.monthly.is_some()
    }

    /// Health as 0.0-1.0: the room left in whichever window is fuller. `None` without data.
    pub fn health(&self) -> Option<f64> {
        let fullest = [self.five_hour, self.seven_day, self.monthly]
            .into_iter()
            .flatten()
            .map(|w| w.used_percentage)
            .reduce(f64::max)?;
        Some((1.0 - fullest / 100.0).clamp(0.0, 1.0))
    }
}

fn window_status(window: LimitWindow, now: f64) -> Option<WindowStatus> {
    let used = window.used_percentage?;
    match window.resets_at {
        Some(reset) if reset <= now => Some(WindowStatus {
            used_percentage: 0.0,
            resets_in_secs: None,
        }),
        reset => Some(WindowStatus {
            used_percentage: used,
            resets_in_secs: reset.map(|r| (r - now) as i64),
        }),
    }
}

/// Parse a snapshot file's contents as of `now` (epoch seconds).
pub fn parse_snapshot(content: &str, now: f64) -> Option<AccountLimits> {
    let file: SnapshotFile = serde_json::from_str(content).ok()?;
    if now - file.captured_at > MAX_SNAPSHOT_AGE_SECS {
        return None;
    }
    let limits = file.rate_limits.unwrap_or_default();
    let status = AccountLimits {
        five_hour: limits.five_hour.and_then(|w| window_status(w, now)),
        seven_day: limits.seven_day.and_then(|w| window_status(w, now)),
        monthly: file.monthly.and_then(|w| window_status(w, now)),
    };
    status.any().then_some(status)
}

/// Limits an analyzer read from its own logs rather than a status line snapshot. Codex writes
/// `rate_limits` into every token-count event, timestamped, so the newest observation of each
/// window across all sessions is the current state; there is no stale-session problem to merge
/// around. Keyed by analyzer display name.
#[derive(Default, Clone, Copy)]
struct Observed {
    five_hour: Option<(DateTime<Utc>, LimitWindow)>,
    seven_day: Option<(DateTime<Utc>, LimitWindow)>,
}

static OBSERVED: OnceLock<Mutex<HashMap<String, Observed>>> = OnceLock::new();

fn keep_newer(
    slot: &mut Option<(DateTime<Utc>, LimitWindow)>,
    at: DateTime<Utc>,
    window: Option<LimitWindow>,
) {
    if let Some(window) = window
        && slot.is_none_or(|(seen, _)| at >= seen)
    {
        *slot = Some((at, window));
    }
}

/// Record limits an analyzer saw in its logs at `at`. Order-independent: each window keeps its
/// newest observation, so parsing files in parallel or re-parsing them is harmless.
pub fn observe(
    analyzer_name: &str,
    at: DateTime<Utc>,
    five_hour: Option<LimitWindow>,
    seven_day: Option<LimitWindow>,
) {
    let map = OBSERVED.get_or_init(Default::default);
    if let Ok(mut map) = map.lock() {
        let entry = map.entry(analyzer_name.to_string()).or_default();
        keep_newer(&mut entry.five_hour, at, five_hour);
        keep_newer(&mut entry.seven_day, at, seven_day);
    }
}

static OBSERVED_LOGINS: OnceLock<Mutex<HashMap<String, (DateTime<Utc>, String)>>> = OnceLock::new();

/// Record the account an analyzer's own logs name (e.g. Codex's plan type), keeping the newest.
pub fn observe_login(analyzer_name: &str, at: DateTime<Utc>, login: String) {
    let map = OBSERVED_LOGINS.get_or_init(Default::default);
    if let Ok(mut map) = map.lock() {
        match map.get(analyzer_name) {
            Some((seen, _)) if *seen > at => {}
            _ => {
                map.insert(analyzer_name.to_string(), (at, login));
            }
        }
    }
}

fn observed_limits(analyzer_name: &str, now: f64) -> Option<AccountLimits> {
    let observed = *OBSERVED.get()?.lock().ok()?.get(analyzer_name)?;
    let fresh = |slot: Option<(DateTime<Utc>, LimitWindow)>| {
        slot.filter(|(at, _)| now - at.timestamp() as f64 <= MAX_SNAPSHOT_AGE_SECS)
            .and_then(|(_, window)| window_status(window, now))
    };
    let status = AccountLimits {
        five_hour: fresh(observed.five_hour),
        seven_day: fresh(observed.seven_day),
        monthly: None,
    };
    status.any().then_some(status)
}

/// Capacity of a local model server, for tools with no plan limits (Ollama): how much of what
/// the machine could generate running nonstop was actually generated in a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capacity {
    /// Output tokens generated in the window.
    pub used_tokens: u64,
    /// Output tokens the machine could generate in the window at its measured throughput.
    pub capacity_tokens: f64,
}

impl Capacity {
    pub fn used_percentage(&self) -> f64 {
        if self.capacity_tokens <= 0.0 {
            0.0
        } else {
            (self.used_tokens as f64 / self.capacity_tokens * 100.0).min(100.0)
        }
    }
}

/// One finished request: when it ended (epoch seconds), how long it kept the model busy, and
/// the output tokens it produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusySample {
    pub ended_at: f64,
    pub busy_secs: f64,
    pub output_tokens: u64,
}

static BUSY: OnceLock<Mutex<HashMap<String, Vec<BusySample>>>> = OnceLock::new();

/// Replace an analyzer's request history, used to work out its capacity at render time.
pub fn record_busy(analyzer_name: &str, samples: Vec<BusySample>) {
    if let Ok(mut map) = BUSY.get_or_init(Default::default).lock() {
        map.insert(analyzer_name.to_string(), samples);
    }
}

/// Capacity over the `window_secs` before `now`. Throughput is output tokens per busy second
/// across the whole history, so it reflects the actual workload (prompt processing included)
/// rather than a benchmark.
pub fn capacity_in(samples: &[BusySample], window_secs: f64, now: f64) -> Option<Capacity> {
    let busy: f64 = samples.iter().map(|s| s.busy_secs).sum();
    let output: u64 = samples.iter().map(|s| s.output_tokens).sum();
    if busy <= 0.0 || output == 0 {
        return None;
    }
    let used_tokens = samples
        .iter()
        .filter(|s| s.ended_at > now - window_secs && s.ended_at <= now)
        .map(|s| s.output_tokens)
        .sum();
    Some(Capacity {
        used_tokens,
        capacity_tokens: output as f64 / busy * window_secs,
    })
}

/// Capacity for the 5h and weekly windows, for an analyzer that recorded its requests.
pub fn capacity_for_analyzer(analyzer_name: &str) -> Option<(Capacity, Capacity)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    let map = BUSY.get()?.lock().ok()?;
    let samples = map.get(analyzer_name)?;
    Some((
        capacity_in(samples, 5.0 * 3600.0, now)?,
        capacity_in(samples, 7.0 * 86_400.0, now)?,
    ))
}

/// A capacity cell: `4% of 1.6M`.
pub fn format_capacity(capacity: &Capacity) -> String {
    let tokens = capacity.capacity_tokens;
    let short = if tokens >= 1e6 {
        format!("{:.1}M", tokens / 1e6)
    } else if tokens >= 1e3 {
        format!("{:.0}K", tokens / 1e3)
    } else {
        format!("{tokens:.0}")
    };
    format!("{:.0}% of {short}", capacity.used_percentage())
}

fn limits_dir() -> Option<PathBuf> {
    Some(
        std::env::home_dir()?
            .join(".config")
            .join("splitrail")
            .join("limits"),
    )
}

/// The snapshot label behind an analyzer's display name: `Claude (<label>)`, the single
/// configured account for a plain `Claude Code`, or `antigravity` (written by
/// `scripts/agy_statusline_capture.py`). `None` for other tools.
pub fn account_label(analyzer_name: &str) -> Option<String> {
    if analyzer_name == crate::analyzers::antigravity::DISPLAY_NAME {
        return Some("antigravity".to_string());
    }
    if analyzer_name == "Droid CLI" {
        return Some("droid".to_string()); // Polled from Factory by `droid_limits`.
    }
    if analyzer_name == crate::analyzers::cursor_agent::DISPLAY_NAME {
        return Some(crate::cursor_limits::SNAPSHOT_LABEL.to_string());
    }
    if let Some(label) = analyzer_name
        .strip_prefix("Claude (")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return Some(label.to_string());
    }
    if analyzer_name == "Claude Code" {
        static SOLE_LABEL: OnceLock<String> = OnceLock::new();
        return Some(
            SOLE_LABEL
                .get_or_init(|| {
                    crate::config::Config::load()
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .claude_code
                        .resolved_accounts()
                        .into_iter()
                        .next()
                        .map(|(label, _)| label)
                        .unwrap_or_else(|| "main".to_string())
                })
                .clone(),
        );
    }
    None
}

/// Limit snapshots other machines published (see `rollup`), by host.
static REMOTE_SNAPSHOTS: OnceLock<Mutex<HashMap<String, Vec<serde_json::Value>>>> = OnceLock::new();

pub fn set_remote_snapshots(host: &str, snapshots: Vec<serde_json::Value>) {
    if let Ok(mut all) = REMOTE_SNAPSHOTS.get_or_init(Default::default).lock() {
        all.insert(host.to_string(), snapshots);
    }
}

/// A snapshot from another machine for the same login, captured after `local` was: the account
/// is shared, so whichever machine saw it last knows its limits best.
fn newer_remote_snapshot(local: &str) -> Option<String> {
    let local: serde_json::Value = serde_json::from_str(local).ok()?;
    let local_at = local.get("captured_at")?.as_f64()?;
    let hash = crate::rollup::login_hash(local.get("login")?.as_str()?);
    let all = REMOTE_SNAPSHOTS.get()?.lock().ok()?;
    all.values()
        .flatten()
        .filter(|s| s.get("login_hash").and_then(|h| h.as_str()) == Some(hash.as_str()))
        .filter_map(|s| Some((s.get("captured_at")?.as_f64()?, s)))
        .filter(|(at, _)| *at > local_at)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s.to_string())
}

/// The snapshot behind a remote Claude column (`Claude@dev-a`): that machine's newest, or a
/// newer one for the same login from anywhere, this machine included.
fn remote_column_snapshot(column: &str) -> Option<String> {
    let (slug, tool) = crate::rollup::remote_source(column)?;
    if !tool.starts_with("Claude") {
        return None;
    }
    let captured = |s: &serde_json::Value| s.get("captured_at").and_then(|a| a.as_f64());
    let hash_of = |s: &serde_json::Value| {
        s.get("login_hash").and_then(|h| h.as_str()).map(str::to_string)
    };
    let mut candidates: Vec<serde_json::Value> = Vec::new();
    {
        let all = REMOTE_SNAPSHOTS.get()?.lock().ok()?;
        let own = all
            .get(&slug)?
            .iter()
            .filter_map(|s| Some((captured(s)?, s)))
            .max_by(|a, b| a.0.total_cmp(&b.0))?
            .1
            .clone();
        if let Some(hash) = hash_of(&own) {
            candidates.extend(all.values().flatten().filter(|s| hash_of(s).as_ref() == Some(&hash)).cloned());
        }
        candidates.push(own);
    }
    let hash = candidates.last().and_then(hash_of);
    if let (Some(hash), Some(dir)) = (hash, limits_dir())
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(local) = serde_json::from_str::<serde_json::Value>(&content)
                && local.get("login").and_then(|l| l.as_str()).map(crate::rollup::login_hash).as_ref()
                    == Some(&hash)
            {
                candidates.push(local);
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|s| Some((captured(&s)?, s)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s.to_string())
}

/// Current plan limits for an analyzer, from its own logs or a status line snapshot.
pub fn limits_for_analyzer(analyzer_name: &str) -> Option<AccountLimits> {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    if let Some(content) = remote_column_snapshot(analyzer_name) {
        return parse_snapshot(&content, now_secs);
    }
    if let Some(limits) = observed_limits(analyzer_name, now_secs) {
        return Some(limits);
    }
    let label = account_label(analyzer_name)?;
    let content = std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?;
    let content = newer_remote_snapshot(&content).unwrap_or(content);
    parse_snapshot(&content, now_secs)
}

/// The account behind an analyzer's column: what its own logs report (Codex's ChatGPT plan),
/// `local` for Ollama, or the account a status line snapshot was last logged in to, as the email's local part with
/// `·team` for a team organization (`support·team` for `support@virti.us` on a team plan). `None` for tools without snapshots or older snapshots.
pub fn login_for_analyzer(analyzer_name: &str) -> Option<String> {
    if analyzer_name == "Ollama" {
        return short_hostname(); // Models on this machine; no account.
    }
    if let Some(content) = remote_column_snapshot(analyzer_name) {
        return parse_login(&content);
    }
    if let Some((_, login)) = OBSERVED_LOGINS
        .get()
        .and_then(|map| map.lock().ok()?.get(analyzer_name).cloned())
    {
        return Some(login);
    }
    let label = account_label(analyzer_name)?;
    let content = std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?;
    parse_login(&content)
}

/// This machine's name without the domain (`Amberts-MacBook-Pro`), like `hostname -s`.
fn short_hostname() -> Option<String> {
    static NAME: OnceLock<Option<String>> = OnceLock::new();
    NAME.get_or_init(|| {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname NUL-terminates on success.
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let name = String::from_utf8_lossy(&buf[..end]);
        let short = name.split('.').next().unwrap_or_default().to_string();
        (!short.is_empty()).then_some(short)
    })
    .clone()
}

fn parse_login(content: &str) -> Option<String> {
    let file = serde_json::from_str::<SnapshotFile>(content).ok()?;
    if let Some(name) = file.login_name.filter(|n| !n.is_empty()) {
        return Some(name);
    }
    let login = file.login?;
    let name = login.split('@').next().unwrap_or(&login);
    (!name.is_empty()).then(|| name.to_string())
}

/// Short countdown for a 13-column cell: `45m`, `2h13m`, `3d4h`.
pub fn format_countdown(secs: i64) -> String {
    if secs <= 0 {
        return "now".to_string();
    }
    let minutes = secs / 60;
    let (days, hours, mins) = (minutes / 1440, (minutes % 1440) / 60, minutes % 60);
    if days > 0 {
        format!("{days}d{hours}h")
    } else if hours > 0 {
        format!("{hours}h{mins:02}m")
    } else {
        format!("{mins}m")
    }
}

/// A window as `23% ↻2h13m`, or `0% ↻?` when it has reset but the next reset is not known yet.
pub fn format_window(window: &WindowStatus) -> String {
    let reset = window
        .resets_in_secs
        .map(format_countdown)
        .unwrap_or_else(|| "?".to_string());
    format!("{:.0}% ↻{reset}", window.used_percentage)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_790_519_606.0;

    fn snapshot(captured_at: f64, five_used: f64, five_reset: f64) -> String {
        format!(
            r#"{{"account":"main","captured_at":{captured_at},"rate_limits":{{"five_hour":{{"used_percentage":{five_used},"resets_at":{five_reset}}},"seven_day":{{"used_percentage":16,"resets_at":{}}}}}}}"#,
            NOW + 500_000.0
        )
    }

    #[test]
    fn health_is_room_left_in_the_fuller_window() {
        let limits = parse_snapshot(&snapshot(NOW, 23.0, NOW + 5_794.0), NOW).unwrap();
        assert_eq!(limits.five_hour.unwrap().resets_in_secs, Some(5_794));
        assert!((limits.health().unwrap() - 0.77).abs() < 1e-9);

        // Weekly fuller than 5h: weekly decides.
        let limits = parse_snapshot(&snapshot(NOW, 5.0, NOW + 60.0), NOW).unwrap();
        assert!((limits.health().unwrap() - 0.84).abs() < 1e-9);
    }

    #[test]
    fn a_window_past_its_reset_reads_as_empty() {
        let limits = parse_snapshot(&snapshot(NOW - 20_000.0, 97.0, NOW - 100.0), NOW).unwrap();
        let five = limits.five_hour.unwrap();
        assert_eq!(five.used_percentage, 0.0);
        assert_eq!(five.resets_in_secs, None);
        assert_eq!(format_window(&five), "0% ↻?");
        // Weekly still counts.
        assert!((limits.health().unwrap() - 0.84).abs() < 1e-9);
    }

    #[test]
    fn stale_or_empty_snapshots_are_ignored() {
        assert!(parse_snapshot(&snapshot(NOW - 8.0 * 86_400.0, 10.0, NOW + 60.0), NOW).is_none());
        let no_limits = r#"{"account":"main","captured_at":1790519606,"rate_limits":null}"#;
        assert!(parse_snapshot(no_limits, NOW).is_none());
        assert!(parse_snapshot("not json", NOW).is_none());
    }

    #[test]
    fn ollama_and_droid_columns_have_a_login() {
        let host = login_for_analyzer("Ollama").unwrap();
        assert!(!host.is_empty() && !host.contains('.'));
        assert_eq!(account_label("Droid CLI").as_deref(), Some("droid"));
    }

    #[test]
    fn login_is_the_email_local_part() {
        let with_login = r#"{"captured_at":1,"rate_limits":null,"login":"support@virti.us"}"#;
        assert_eq!(parse_login(with_login).as_deref(), Some("support"));
        assert_eq!(parse_login(&snapshot(1.0, 2.0, 3.0)), None);
        let team = r#"{"captured_at":1,"rate_limits":null,"login":"s@v.us (Virtius)","login_name":"s·team"}"#;
        assert_eq!(parse_login(team).as_deref(), Some("s·team"));
    }

    #[test]
    fn account_labels_come_from_analyzer_names() {
        assert_eq!(account_label("Claude (v)").as_deref(), Some("v"));
        assert_eq!(account_label("Codex CLI"), None);
        assert_eq!(account_label("Antigravity").as_deref(), Some("antigravity"));
    }

    #[test]
    fn observed_limits_keep_the_newest_report_per_window() {
        use chrono::TimeZone;
        let name = "Observed Test Tool";
        let window = |pct: f64| LimitWindow {
            used_percentage: Some(pct),
            resets_at: Some(NOW + 3_600.0),
        };
        let t = |s: i64| Utc.timestamp_opt(NOW as i64 - s, 0).unwrap();
        // Newer report first, older second: the older one must not win.
        observe(name, t(10), Some(window(30.0)), Some(window(5.0)));
        observe(name, t(500), Some(window(80.0)), None);
        // A report with only the weekly window keeps the known 5h one.
        observe(name, t(5), None, Some(window(6.0)));
        let limits = observed_limits(name, NOW).unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percentage, 30.0);
        assert_eq!(limits.seven_day.unwrap().used_percentage, 6.0);
    }

    #[test]
    fn capacity_is_output_against_what_nonstop_running_would_produce() {
        let sample = |ended_at: f64, busy_secs: f64, output_tokens: u64| BusySample {
            ended_at,
            busy_secs,
            output_tokens,
        };
        // 10 tok/s over the history; 300 tokens in the last 5h.
        let samples = [
            sample(NOW - 86_400.0, 70.0, 700),
            sample(NOW - 600.0, 20.0, 200),
            sample(NOW - 60.0, 10.0, 100),
        ];
        let five = capacity_in(&samples, 5.0 * 3600.0, NOW).unwrap();
        assert_eq!(five.used_tokens, 300);
        assert!((five.capacity_tokens - 180_000.0).abs() < 1e-6);
        assert_eq!(format_capacity(&five), "0% of 180K");
        let week = capacity_in(&samples, 7.0 * 86_400.0, NOW).unwrap();
        assert_eq!(week.used_tokens, 1000);
        assert_eq!(format_capacity(&week), "0% of 6.0M");
        assert!(capacity_in(&[], 3600.0, NOW).is_none());
    }

    #[test]
    fn countdowns_fit_a_cell() {
        assert_eq!(format_countdown(45 * 60), "45m");
        assert_eq!(format_countdown(2 * 3600 + 13 * 60), "2h13m");
        assert_eq!(format_countdown(3 * 86_400 + 4 * 3600), "3d4h");
        assert_eq!(format_countdown(-5), "now");
    }

    #[test]
    fn a_newer_snapshot_from_another_machine_wins_for_the_same_login() {
        let local = r#"{"captured_at": 1000.0, "login": "me@x.com",
            "rate_limits": {"five_hour": {"used_percentage": 100.0, "resets_at": 9000.0}}}"#;
        assert_eq!(newer_remote_snapshot(local), None);
        let remote = |at: f64, login: &str, pct: f64| {
            serde_json::json!({"captured_at": at, "login_hash": crate::rollup::login_hash(login),
                "rate_limits": {"five_hour": {"used_percentage": pct, "resets_at": 9000.0}}})
        };
        set_remote_snapshots(
            "test-devbox",
            vec![remote(900.0, "me@x.com", 1.0), remote(2000.0, "other@x.com", 2.0), remote(1500.0, "ME@x.com", 25.0)],
        );
        let chosen = newer_remote_snapshot(local).unwrap();
        let limits = parse_snapshot(&chosen, 1600.0).unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percentage, 25.0);
        set_remote_snapshots("test-devbox", vec![]);
    }

    #[test]
    fn a_remote_claude_column_shows_its_machines_login_and_limits() {
        assert_eq!(remote_column_snapshot("Claude@tes-box"), None);
        crate::rollup::register_remote_column("Claude@tes-box", "test_box", "Claude Code");
        crate::rollup::register_remote_column("Codex@test-box", "test_box", "Codex");
        set_remote_snapshots(
            "test_box",
            vec![serde_json::json!({"captured_at": 1000.0, "login_hash": crate::rollup::login_hash("dev@x.com"),
                "login_name": "dev", "rate_limits": {"five_hour": {"used_percentage": 40.0, "resets_at": 9000.0}}})],
        );
        let snapshot = remote_column_snapshot("Claude@tes-box").unwrap();
        assert_eq!(parse_login(&snapshot).as_deref(), Some("dev"));
        assert_eq!(parse_snapshot(&snapshot, 1500.0).unwrap().five_hour.unwrap().used_percentage, 40.0);
        assert_eq!(remote_column_snapshot("Codex@test-box"), None, "only Claude has snapshots");
        set_remote_snapshots("test_box", vec![]);
    }
}

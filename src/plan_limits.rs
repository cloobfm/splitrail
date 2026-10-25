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
    let limits = file.rate_limits.unwrap_or_default();
    if now - file.captured_at > MAX_SNAPSHOT_AGE_SECS {
        // Too old to say what is used, except a weekly window past its reset: that is empty,
        // with the reset at the same hour each week (a plan idle for a while still shows).
        let status = AccountLimits { five_hour: None, seven_day: limits.seven_day.filter(|w| w.resets_at.is_some_and(|r| r <= now)).and_then(|w| rolled_week(w, now)), monthly: None };
        return status.any().then_some(status);
    }
    let status = AccountLimits {
        five_hour: limits.five_hour.and_then(|w| window_status(w, now)),
        // A weekly window resets at the same time each week: past its reset it is empty, with
        // the next reset a week on, rather than of unknown length until the tool runs again.
        seven_day: limits.seven_day.and_then(|w| rolled_week(w, now).or_else(|| window_status(w, now))),
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

/// Seconds covered by at least one request: overlapping requests (a server batching sixteen at
/// once) count their shared time once.
fn busy_wall_secs<'a>(samples: impl Iterator<Item = &'a BusySample>) -> f64 {
    let mut spans: Vec<(f64, f64)> = samples.map(|s| (s.ended_at - s.busy_secs, s.ended_at)).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (start, end) in spans {
        match current {
            Some((s, e)) if start <= e => current = Some((s, e.max(end))),
            Some((s, e)) => {
                total += e - s;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    total + current.map_or(0.0, |(s, e)| e - s)
}

/// Capacity over the `window_secs` before `now`: what the server could generate running nonstop
/// at the rate it reaches while working, output tokens per second it was busy, overlapping
/// requests counted once (so a batching server's sixteen streams add up), over the last week's
/// work where there is some (the current setup), else the whole history. It reflects the actual
/// workload, prompt processing included, rather than a benchmark.
pub fn capacity_in(samples: &[BusySample], window_secs: f64, now: f64) -> Option<Capacity> {
    let recent: Vec<&BusySample> = samples.iter().filter(|s| s.ended_at > now - 7.0 * 86_400.0).collect();
    let basis: Vec<&BusySample> = if recent.is_empty() { samples.iter().collect() } else { recent };
    let busy = busy_wall_secs(basis.iter().copied());
    let output: u64 = basis.iter().map(|s| s.output_tokens).sum();
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

/// Capacity for the 5h and weekly windows, for an analyzer that recorded its requests, or a
/// remote server column as its machine published it.
pub fn capacity_for_analyzer(analyzer_name: &str) -> Option<(Capacity, Capacity)> {
    let local = || {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
        let map = BUSY.get()?.lock().ok()?;
        let samples = map.get(analyzer_name)?;
        Some((capacity_in(samples, 5.0 * 3600.0, now)?, capacity_in(samples, 7.0 * 86_400.0, now)?))
    };
    local().or_else(|| {
        let (slug, tool) = crate::rollup::remote_source(analyzer_name)?;
        REMOTE_CAPACITY.get()?.lock().ok()?.get(&slug)?.get(&tool).copied()
    })
}

static REMOTE_CAPACITY: OnceLock<Mutex<HashMap<String, HashMap<String, (Capacity, Capacity)>>>> = OnceLock::new();

/// This machine's server columns' capacity, to publish with its limits: `{tool: {five_hour:
/// {used, capacity}, seven_day: {...}}}`.
pub fn capacity_payload() -> serde_json::Value {
    let names: Vec<String> = BUSY
        .get()
        .and_then(|m| m.lock().ok().map(|m| m.keys().cloned().collect()))
        .unwrap_or_default();
    let window = |c: &Capacity| serde_json::json!({"used": c.used_tokens, "capacity": c.capacity_tokens.round()});
    serde_json::Value::Object(
        names
            .into_iter()
            .filter_map(|name| {
                let (five, week) = capacity_for_analyzer(&name)?;
                Some((name, serde_json::json!({"five_hour": window(&five), "seven_day": window(&week)})))
            })
            .collect(),
    )
}

/// Another machine's published capacity (see `capacity_payload`).
pub fn set_remote_capacity(host: &str, payload: &serde_json::Value) {
    let window = |v: &serde_json::Value| {
        Some(Capacity { used_tokens: v.get("used")?.as_u64()?, capacity_tokens: v.get("capacity")?.as_f64()? })
    };
    let parsed: HashMap<String, (Capacity, Capacity)> = payload
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(tool, v)| Some((tool.clone(), (window(v.get("five_hour")?)?, window(v.get("seven_day")?)?))))
        .collect();
    if let Ok(mut all) = REMOTE_CAPACITY.get_or_init(Default::default).lock() {
        all.insert(host.to_string(), parsed);
    }
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
    if analyzer_name == "Kiro CLI" {
        return Some(crate::kiro_limits::SNAPSHOT_LABEL.to_string()); // From Kiro's `/usage`.
    }
    if analyzer_name == crate::analyzers::warp_dev::DISPLAY_NAME {
        return Some(crate::warp_limits::SNAPSHOT_LABEL.to_string()); // From Warp's preferences.
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

/// Reset times of one window differ slightly between reports; beyond this they are different
/// windows (as in `scripts/claude_statusline_capture.py`).
const SAME_WINDOW_TOLERANCE_SECS: f64 = 30.0 * 60.0;

/// The more current of two reports of one window: a later window wins, and within one window the
/// higher figure, since usage only grows until the reset. Taking the newest report instead would
/// flip between machines: a session that has not called the API lately keeps re-reporting the
/// figure from its last call (devbox-a's 70% against this Mac's 79%).
fn better_window<'a>(a: &'a serde_json::Value, b: &'a serde_json::Value) -> &'a serde_json::Value {
    let used = |w: &serde_json::Value| w.get("used_percentage").and_then(|u| u.as_f64());
    let reset = |w: &serde_json::Value| w.get("resets_at").and_then(|r| r.as_f64());
    match (used(a), used(b)) {
        (None, _) => return b,
        (_, None) => return a,
        _ => {}
    }
    if let (Some(ra), Some(rb)) = (reset(a), reset(b)) {
        if rb > ra + SAME_WINDOW_TOLERANCE_SECS {
            return b;
        }
        if rb < ra - SAME_WINDOW_TOLERANCE_SECS {
            return a;
        }
    }
    if used(b) > used(a) { b } else { a }
}

/// The newest of `snapshots` (for the login and capture time) with each window taken from
/// whichever snapshot reports it best (`better_window`). They must all be the same login's.
fn merge_snapshots(snapshots: Vec<serde_json::Value>) -> Option<serde_json::Value> {
    let captured = |s: &serde_json::Value| s.get("captured_at").and_then(|a| a.as_f64());
    let mut merged = snapshots
        .iter()
        .filter_map(|s| Some((captured(s)?, s)))
        .max_by(|a, b| a.0.total_cmp(&b.0))?
        .1
        .clone();
    let mut windows = serde_json::Map::new();
    for limits in snapshots.iter().filter_map(|s| s.get("rate_limits")?.as_object()) {
        for (key, window) in limits {
            let best = match windows.get(key) {
                Some(current) => better_window(current, window).clone(),
                None => window.clone(),
            };
            windows.insert(key.clone(), best);
        }
    }
    if !windows.is_empty() {
        merged["rate_limits"] = serde_json::Value::Object(windows);
    }
    Some(merged)
}

/// `local` merged with other machines' snapshots for the same login (`merge_snapshots`): the
/// account is shared, so between them they know its limits best.
fn with_remote_snapshots(local: &str) -> Option<String> {
    let local: serde_json::Value = serde_json::from_str(local).ok()?;
    let hash = crate::rollup::login_hash(local.get("login")?.as_str()?);
    let mut snapshots: Vec<serde_json::Value> = REMOTE_SNAPSHOTS
        .get()?
        .lock()
        .ok()?
        .values()
        .flatten()
        .filter(|s| s.get("login_hash").and_then(|h| h.as_str()) == Some(hash.as_str()))
        .cloned()
        .collect();
    if snapshots.is_empty() {
        return None;
    }
    snapshots.push(local);
    merge_snapshots(snapshots).map(|s| s.to_string())
}

/// The snapshot behind a remote Claude column (`Claude@dev-a`): that machine's newest, merged
/// with every snapshot for the same login from anywhere, this machine included.
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
    merge_snapshots(candidates).map(|s| s.to_string())
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
    let content = with_remote_snapshots(&content).unwrap_or(content);
    parse_snapshot(&content, now_secs)
}

/// A login on a Claude account other than the one signed in now, with its weekly window as it
/// stands: the status line recorded it when that login last ran.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherLogin {
    /// `a@virti.us (Virtius)`, `ambertyeung@gmail.com (personal)`.
    pub account: String,
    pub seven_day: WindowStatus,
    /// Its 5-hour window as last seen: full means it is locked out until the reset, signed in
    /// or not. Past its reset, empty.
    pub five_hour: Option<WindowStatus>,
    /// Its plan as last seen (`Max 20x`, `Team Std`), when the capture recorded one.
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PlanEntry {
    label: Option<String>,
    tier: Option<String>,
    #[serde(default)]
    extra_usage: bool,
}

static PLAN_CONFIG: OnceLock<HashMap<String, crate::config::PlanConfig>> = OnceLock::new();

/// Plans stated in the config (`[plans]`), for tools whose own files do not name theirs. Set once
/// at startup.
pub fn set_plan_config(plans: HashMap<String, crate::config::PlanConfig>) {
    let _ = PLAN_CONFIG.set(plans);
}

fn plan_config(analyzer_name: &str) -> Option<&'static crate::config::PlanConfig> {
    PLAN_CONFIG.get()?.get(analyzer_name)
}

/// The plan a Claude column is signed in on (`Max 20x`, `Team Std`), from the capture.
pub fn plan_for_analyzer(analyzer_name: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct File {
        plan: Option<PlanEntry>,
    }
    if let Some(name) = plan_config(analyzer_name).and_then(|p| p.name.clone()) {
        return Some(name);
    }
    let label = account_label(analyzer_name)?;
    let content = std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?;
    if let Some(plan) = serde_json::from_str::<File>(&content).ok().and_then(|f| f.plan?.label) {
        return Some(plan);
    }
    // Other tools' plans, from what their limit files record.
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_f64);
    match analyzer_name {
        "Cursor Agent" => number("limit_cents").map(|c| format!("${:.0}/mo", c / 100.0)),
        "Warp" => number("credits_limit").map(|c| format!("{:.0} cr", c)),
        "Kiro CLI" => value.get("login_name").and_then(|v| v.as_str()).map(|n| n.trim_start_matches("Kiro ").to_string()),
        _ => None,
    }
}

/// A plan's allowance in dollars a week at API prices, where its limit file says (credit
/// plans: Cursor's included spend, Warp's included usage, Kiro's credits at $0.02), for sizing it
/// against other plans. Monthly allowances are spread over the month.
pub fn plan_value_dollars(analyzer_name: &str) -> Option<f64> {
    let label = account_label(analyzer_name)?;
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?).ok()?;
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_f64);
    let monthly = match analyzer_name {
        "Cursor Agent" => number("limit_cents")? / 100.0,
        "Warp" => number("included_usage_cents")? / 100.0,
        "Kiro CLI" => number("credits_limit")? * crate::analyzers::kiro_sessions::DOLLARS_PER_CREDIT,
        _ => return None,
    };
    Some(monthly)
}

/// Every plan this machine's accounts are on, as `(key, signature)`: a Claude login's plan
/// (label, rate tier, extra usage), Cursor's and Warp's monthly limits, Kiro's plan and credits,
/// Codex's ChatGPT plan. A signature that changes is an upgrade or a downgrade.
pub fn plan_signatures() -> Vec<(String, String)> {
    #[derive(Deserialize)]
    struct LoginEntry {
        plan: Option<PlanEntry>,
    }
    #[derive(Deserialize)]
    struct ClaudeFile {
        #[serde(default)]
        logins: HashMap<String, LoginEntry>,
    }
    let Some(dir) = limits_dir() else { return Vec::new() };
    let read = |name: &str| std::fs::read_to_string(dir.join(format!("{name}.json"))).ok();
    let mut out = Vec::new();
    for label in ["main", "v"] {
        let Some(file) = read(label).and_then(|c| serde_json::from_str::<ClaudeFile>(&c).ok()) else { continue };
        for (login, entry) in file.logins {
            if let Some(plan) = entry.plan.filter(|p| p.label.is_some()) {
                let sig = format!(
                    "{}{}{}",
                    plan.label.unwrap_or_default(),
                    plan.tier.map(|t| format!(" ({t})")).unwrap_or_default(),
                    if plan.extra_usage { " + extra usage" } else { "" }
                );
                out.push((format!("Claude ({label}) · {login}"), sig));
            }
        }
    }
    let fields = |name: &str, keys: &[&str]| -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(&read(name)?).ok()?;
        let parts: Vec<String> = keys
            .iter()
            .filter_map(|k| value.get(*k).filter(|v| !v.is_null()).map(|v| format!("{k} {}", v.as_str().map_or_else(|| v.to_string(), str::to_string))))
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    };
    for (tool, file, keys) in [
        ("Cursor Agent", "cursor", &["limit_cents"][..]),
        ("Warp", "warp", &["credits_limit", "included_usage_cents"][..]),
        ("Kiro CLI", "kiro", &["login_name", "credits_limit"][..]),
    ] {
        if let Some(sig) = fields(file, keys) {
            out.push((tool.to_string(), sig));
        }
    }
    if let Some(plan) = account_for_analyzer("Codex CLI") {
        out.push(("Codex CLI".to_string(), plan));
    }
    out.sort();
    out
}

const WEEK_SECS: f64 = 7.0 * 24.0 * 3600.0;

/// The weekly window of a login last seen with `window`, as of `now`. A weekly window resets at
/// the same time each week, so a reset that has passed is followed by another a week later:
/// the window is then empty (no one used the login since) with a known next reset.
fn rolled_week(window: LimitWindow, now: f64) -> Option<WindowStatus> {
    let used = window.used_percentage?;
    let mut reset = window.resets_at?;
    if reset > now {
        return Some(WindowStatus { used_percentage: used, resets_in_secs: Some((reset - now) as i64) });
    }
    while reset <= now {
        reset += WEEK_SECS;
    }
    Some(WindowStatus { used_percentage: 0.0, resets_in_secs: Some((reset - now) as i64) })
}

/// The other logins a Claude column's status line has seen (BZL-238 follow-up): each a
/// separate allowance, reached by `/login`.
pub fn other_logins(analyzer_name: &str) -> Vec<OtherLogin> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let Some(content) = account_label(analyzer_name)
        .and_then(|label| std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok())
    else {
        return Vec::new();
    };
    other_logins_in(&content, now)
}

fn other_logins_in(content: &str, now: f64) -> Vec<OtherLogin> {
    #[derive(Deserialize)]
    struct Entry {
        rate_limits: Option<RateLimits>,
        plan: Option<PlanEntry>,
    }
    #[derive(Deserialize)]
    struct File {
        login: Option<String>,
        #[serde(default)]
        logins: HashMap<String, Entry>,
    }
    let Ok(file) = serde_json::from_str::<File>(content) else { return Vec::new() };
    let mut out: Vec<OtherLogin> = file
        .logins
        .into_iter()
        .filter(|(account, _)| Some(account) != file.login.as_ref())
        .filter_map(|(account, entry)| {
            let plan = entry.plan.and_then(|p| p.label);
            let limits = entry.rate_limits?;
            let five_hour = limits.five_hour.and_then(|w| window_status(w, now));
            let seven_day = rolled_week(limits.seven_day?, now)?;
            Some(OtherLogin { account, seven_day, five_hour, plan })
        })
        .collect();
    out.sort_by(|a, b| a.account.cmp(&b.account));
    out
}

/// The account behind an analyzer's column: what its own logs report (Codex's ChatGPT plan),
/// `local` for Ollama, or the account a status line snapshot was last logged in to, as the email's local part with
/// `·team` for a team organization (`support·team` for `support@virti.us` on a team plan). `None` for tools without snapshots or older snapshots.
pub fn login_for_analyzer(analyzer_name: &str) -> Option<String> {
    if crate::analyzers::ollama::ENGINE_COLUMNS.contains(&analyzer_name) {
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

/// The full account a column is signed in to, for the Quotas view: the email with its
/// organisation (`ambertyeung@gmail.com (personal)`), else the plan or short name.
pub fn account_for_analyzer(analyzer_name: &str) -> Option<String> {
    if let Some(account) = plan_config(analyzer_name).and_then(|p| p.account.clone()) {
        return Some(account);
    }
    let full = |content: &str| {
        let file = serde_json::from_str::<SnapshotFile>(content).ok()?;
        file.login.filter(|l| !l.is_empty()).or(file.login_name.filter(|n| !n.is_empty()))
    };
    if let Some(content) = remote_column_snapshot(analyzer_name) {
        return full(&content);
    }
    if let Some(content) = account_label(analyzer_name)
        .and_then(|label| std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok())
        && let Some(account) = full(&content)
    {
        return Some(account);
    }
    login_for_analyzer(analyzer_name)
}

/// How much of a login's week one full 5-hour window uses, in percent, as its status line has
/// measured it (`week_per_5h`, learned while 5-hour windows fill). `login` is the full login
/// key; `None` reads the signed-in one.
pub fn week_per_5h(analyzer_name: &str, login: Option<&str>) -> Option<f64> {
    if cfg!(test) {
        return None;
    }
    let label = account_label(analyzer_name)?;
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?).ok()?;
    let login = login.map(str::to_string).or_else(|| value.get("login")?.as_str().map(str::to_string))?;
    value.get("logins")?.get(&login)?.get("week_per_5h")?.as_f64().filter(|v| *v > 0.0)
}

/// This machine's name without the domain (`Amberts-MacBook-Pro`), like `hostname -s`.
fn short_hostname() -> Option<String> {
    static NAME: OnceLock<Option<String>> = OnceLock::new();
    NAME.get_or_init(|| {
        let name = raw_hostname()?;
        let short = name.split('.').next().unwrap_or_default().to_string();
        (!short.is_empty()).then_some(short)
    })
    .clone()
}

/// This machine's name as the OS reports it, possibly fully qualified.
#[cfg(unix)]
fn raw_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates on success.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// Windows has no `gethostname` in libc; `COMPUTERNAME` is what the OS sets for this.
#[cfg(windows)]
fn raw_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
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
    fn an_old_snapshot_keeps_its_week_rolled_forward() {
        // Last seen ten days ago, its week reset since: empty now, resetting a week after.
        let old = format!(
            r#"{{"captured_at":{},"rate_limits":{{"seven_day":{{"used_percentage":100,"resets_at":{}}}}}}}"#,
            NOW - 10.0 * 86_400.0,
            NOW - 5.0 * 86_400.0
        );
        let week = parse_snapshot(&old, NOW).unwrap().seven_day.unwrap();
        assert_eq!(week.used_percentage, 0.0);
        assert_eq!(week.resets_in_secs, Some(2 * 86_400));
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
    fn other_logins_carry_their_week_rolled_forward_to_now() {
        let day = 86_400.0;
        let now = 1_000.0 * day;
        let content = format!(
            r#"{{"login": "a@virti.us (personal)", "captured_at": {now},
                "logins": {{
                  "a@virti.us (personal)": {{"captured_at": {now}, "rate_limits": {{"seven_day": {{"used_percentage": 34, "resets_at": {}}}}}}},
                  "a@virti.us (Virtius)": {{"captured_at": {}, "rate_limits": {{"seven_day": {{"used_percentage": 60, "resets_at": {}}}}}}},
                  "b@virti.us (Virtius)": {{"captured_at": {}, "rate_limits": {{"seven_day": {{"used_percentage": 20, "resets_at": {}}}}}}}
                }}}}"#,
            now + day,
            now - 10.0 * day, now - 9.0 * day,
            now - 1.0 * day, now + 2.0 * day,
        );
        let others = other_logins_in(&content, now);
        assert_eq!(others.iter().map(|o| o.account.as_str()).collect::<Vec<_>>(), ["a@virti.us (Virtius)", "b@virti.us (Virtius)"]);
        // Reset 9 days ago: a week later (2 days ago) it reset again, and next resets in 5 days, empty.
        assert_eq!(others[0].seven_day, WindowStatus { used_percentage: 0.0, resets_in_secs: Some((5.0 * day) as i64) });
        assert_eq!(others[0].plan, None);
        // Not reset yet: as last seen.
        assert_eq!(others[1].seven_day, WindowStatus { used_percentage: 20.0, resets_in_secs: Some((2.0 * day) as i64) });
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
    fn parallel_requests_add_up_to_the_servers_rate() {
        // Sixteen requests at once, each 100 s and 1,000 tokens: 160 tok/s, not 10.
        let samples: Vec<BusySample> =
            (0..16).map(|_| BusySample { ended_at: NOW - 60.0, busy_secs: 100.0, output_tokens: 1000 }).collect();
        let five = capacity_in(&samples, 5.0 * 3600.0, NOW).unwrap();
        assert!((five.capacity_tokens - 160.0 * 5.0 * 3600.0).abs() < 1e-6);
    }

    #[test]
    fn countdowns_fit_a_cell() {
        assert_eq!(format_countdown(45 * 60), "45m");
        assert_eq!(format_countdown(2 * 3600 + 13 * 60), "2h13m");
        assert_eq!(format_countdown(3 * 86_400 + 4 * 3600), "3d4h");
        assert_eq!(format_countdown(-5), "now");
    }

    #[test]
    fn other_machines_snapshots_for_the_same_login_keep_the_highest_figure_per_window() {
        let local = r#"{"captured_at": 1000.0, "login": "me@x.com",
            "rate_limits": {"five_hour": {"used_percentage": 10.0, "resets_at": 9000.0},
                            "seven_day": {"used_percentage": 79.0, "resets_at": 90000.0}}}"#;
        assert_eq!(with_remote_snapshots(local), None);
        let remote = |at: f64, login: &str, pct: f64| {
            serde_json::json!({"captured_at": at, "login_hash": crate::rollup::login_hash(login),
                "rate_limits": {"five_hour": {"used_percentage": pct, "resets_at": 9000.0},
                                "seven_day": {"used_percentage": 70.0, "resets_at": 90000.0}}})
        };
        set_remote_snapshots(
            "test-devbox",
            vec![remote(900.0, "me@x.com", 1.0), remote(2000.0, "other@x.com", 2.0), remote(1500.0, "ME@x.com", 25.0)],
        );
        let chosen = with_remote_snapshots(local).unwrap();
        let limits = parse_snapshot(&chosen, 1600.0).unwrap();
        assert_eq!(limits.five_hour.unwrap().used_percentage, 25.0, "the other login's 2% is ignored");
        // The newest snapshot (1500) repeats an older 70% for the week; the higher figure stays.
        assert_eq!(limits.seven_day.unwrap().used_percentage, 79.0);
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

    #[test]
    fn a_later_window_beats_a_higher_figure_from_the_last_one() {
        let w = |used: f64, reset: f64| serde_json::json!({"used_percentage": used, "resets_at": reset});
        let (old, new) = (w(95.0, 1000.0), w(3.0, 1000.0 + 5.0 * 3600.0));
        assert_eq!(better_window(&old, &new), &new);
        assert_eq!(better_window(&new, &old), &new);
        let (stale, current) = (w(70.0, 1000.0), w(79.0, 1060.0));
        assert_eq!(better_window(&current, &stale), &current, "same window: the higher figure");
    }
}

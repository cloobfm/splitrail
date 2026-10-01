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
}

impl AccountLimits {
    /// Health as 0.0-1.0: the room left in whichever window is fuller. `None` without data.
    pub fn health(&self) -> Option<f64> {
        let fullest = [self.five_hour, self.seven_day]
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
    let limits = file.rate_limits?;
    let status = AccountLimits {
        five_hour: limits.five_hour.and_then(|w| window_status(w, now)),
        seven_day: limits.seven_day.and_then(|w| window_status(w, now)),
    };
    (status.five_hour.is_some() || status.seven_day.is_some()).then_some(status)
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

fn observed_limits(analyzer_name: &str, now: f64) -> Option<AccountLimits> {
    let observed = *OBSERVED.get()?.lock().ok()?.get(analyzer_name)?;
    let fresh = |slot: Option<(DateTime<Utc>, LimitWindow)>| {
        slot.filter(|(at, _)| now - at.timestamp() as f64 <= MAX_SNAPSHOT_AGE_SECS)
            .and_then(|(_, window)| window_status(window, now))
    };
    let status = AccountLimits {
        five_hour: fresh(observed.five_hour),
        seven_day: fresh(observed.seven_day),
    };
    (status.five_hour.is_some() || status.seven_day.is_some()).then_some(status)
}

fn limits_dir() -> Option<PathBuf> {
    Some(
        std::env::home_dir()?
            .join(".config")
            .join("splitrail")
            .join("limits"),
    )
}

/// The account label behind a Claude Code analyzer's display name: `Claude (<label>)`, or the
/// single configured account for a plain `Claude Code`. `None` for other tools.
pub fn account_label(analyzer_name: &str) -> Option<String> {
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

/// Current plan limits for a Claude Code analyzer, if its status line has captured any.
pub fn limits_for_analyzer(analyzer_name: &str) -> Option<AccountLimits> {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    if let Some(limits) = observed_limits(analyzer_name, now_secs) {
        return Some(limits);
    }
    let label = account_label(analyzer_name)?;
    let content = std::fs::read_to_string(limits_dir()?.join(format!("{label}.json"))).ok()?;
    parse_snapshot(&content, now_secs)
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
    fn account_labels_come_from_analyzer_names() {
        assert_eq!(account_label("Claude (v)").as_deref(), Some("v"));
        assert_eq!(account_label("Codex CLI"), None);
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
    fn countdowns_fit_a_cell() {
        assert_eq!(format_countdown(45 * 60), "45m");
        assert_eq!(format_countdown(2 * 3600 + 13 * 60), "2h13m");
        assert_eq!(format_countdown(3 * 86_400 + 4 * 3600), "3d4h");
        assert_eq!(format_countdown(-5), "now");
    }
}

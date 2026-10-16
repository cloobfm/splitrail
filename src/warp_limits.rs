//! Warp's monthly AI credits, from what the Warp app itself caches: no API call, no token.
//!
//! Warp keeps its last `GetRequestLimitInfo` answer in its preferences (`dev.warp.Warp-Stable`,
//! key `AIRequestLimitInfo`, a JSON string), refreshed while the app runs:
//!
//! ```json
//! {"limit": 1500, "num_requests_used_since_refresh": 8, "included_usage_cents": 2700.0,
//!  "usage_cents_used_since_last_refresh": 14.6, "next_refresh_time": "2026-10-17T05:44:38Z",
//!  "is_unlimited": false, "request_limit_refresh_duration": "Monthly"}
//! ```
//!
//! Every few minutes this writes `~/.config/splitrail/limits/warp.json` in the shape the other
//! monthly allowances use (`plan_limits`), with the signed-in account from Warp's database.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const SNAPSHOT_LABEL: &str = "warp";
const PREFERENCES_DOMAIN: &str = "dev.warp.Warp-Stable";
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// What a credit cost on the plan the format was read from ($27 for 1,500), for when the
/// preferences cannot be read.
const DEFAULT_CENTS_PER_CREDIT: f64 = 1.8;

/// `AIRequestLimitInfo` as stored: a JSON string, sometimes encoded twice.
fn decode(raw: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    match value {
        Value::String(inner) => serde_json::from_str(&inner).ok(),
        other => Some(other),
    }
}

/// The snapshot for a cached limit record captured at `captured_at` (epoch seconds), or `None`
/// for an unlimited plan or one without a limit.
pub fn snapshot_from_limit_info(raw: &str, captured_at: f64, email: Option<&str>) -> Option<Value> {
    let info = decode(raw)?;
    if info.get("is_unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let limit = info.get("limit")?.as_f64().filter(|l| *l > 0.0)?;
    let used = info.get("num_requests_used_since_refresh")?.as_f64()?;
    let resets_at = info
        .get("next_refresh_time")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp() as f64);
    Some(json!({
        "account": SNAPSHOT_LABEL,
        "captured_at": captured_at,
        "monthly": { "used_percentage": used / limit * 100.0, "resets_at": resets_at },
        "credits_limit": limit,
        "credits_used": used,
        "included_usage_cents": info.get("included_usage_cents"),
        "usage_cents_used": info.get("usage_cents_used_since_last_refresh"),
        "login": email,
        "login_name": email.and_then(|e| e.split('@').next()),
    }))
}

/// The cached limit record and when Warp last wrote its preferences.
fn read_limit_info() -> Result<(String, f64)> {
    let output = std::process::Command::new("defaults")
        .args(["read", PREFERENCES_DOMAIN, "AIRequestLimitInfo"])
        .output()
        .context("Could not run `defaults`")?;
    anyhow::ensure!(output.status.success(), "Warp has no cached credit limit");
    let plist = std::env::home_dir()
        .context("No home directory")?
        .join(format!("Library/Preferences/{PREFERENCES_DOMAIN}.plist"));
    let written = std::fs::metadata(plist)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(now, |d| d.as_secs_f64());
    Ok((String::from_utf8_lossy(&output.stdout).into_owned(), written))
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Cents per credit on the current plan: included dollars over included credits.
pub fn cents_per_credit() -> f64 {
    read_limit_info()
        .ok()
        .and_then(|(raw, _)| decode(&raw))
        .and_then(|info| {
            let cents = info.get("included_usage_cents")?.as_f64()?;
            let limit = info.get("limit")?.as_f64()?;
            (cents > 0.0 && limit > 0.0).then(|| cents / limit)
        })
        .unwrap_or(DEFAULT_CENTS_PER_CREDIT)
}

fn write_snapshot() -> Result<()> {
    let (raw, captured_at) = read_limit_info()?;
    let email = crate::analyzers::warp_dev::signed_in_email();
    let snapshot = snapshot_from_limit_info(&raw, captured_at, email.as_deref())
        .context("Warp's cached credit limit has no monthly limit")?;
    let dir = std::env::home_dir()
        .context("No home directory")?
        .join(".config/splitrail/limits");
    std::fs::create_dir_all(&dir)?;
    // Write-then-rename so the dashboard never reads a half-written file.
    let tmp = dir.join(format!(".{SNAPSHOT_LABEL}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&snapshot)?)?;
    std::fs::rename(&tmp, dir.join(format!("{SNAPSHOT_LABEL}.json")))?;
    Ok(())
}

fn warp_installed() -> bool {
    std::env::home_dir()
        .map(|h| h.join(format!("Library/Preferences/{PREFERENCES_DOMAIN}.plist")))
        .is_some_and(|p: PathBuf| p.is_file())
}

/// Refresh the snapshot every few minutes while the dashboard runs. Does nothing without Warp.
pub async fn run() {
    if !warp_installed() {
        return;
    }
    let mut reported = false;
    loop {
        match tokio::task::spawn_blocking(write_snapshot).await {
            Ok(Ok(())) => reported = false,
            Ok(Err(e)) if !reported => {
                crate::utils::log_error(format!("Warp credits: {e:#}"));
                reported = true; // Once per failure streak, not every poll.
            }
            _ => {}
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = r#""{\"limit\":1500,\"num_requests_used_since_refresh\":75,\"included_usage_cents\":2700.0,\"usage_cents_used_since_last_refresh\":14.6,\"next_refresh_time\":\"2026-10-17T05:44:38Z\",\"is_unlimited\":false,\"request_limit_refresh_duration\":\"Monthly\"}""#;

    #[test]
    fn cached_limit_becomes_a_monthly_snapshot() {
        let snapshot = snapshot_from_limit_info(INFO, 1000.0, Some("me@example.com")).unwrap();
        assert_eq!(snapshot["monthly"]["used_percentage"], 5.0);
        assert_eq!(snapshot["monthly"]["resets_at"], 1_792_215_878.0);
        assert_eq!(snapshot["login_name"], "me");
        assert_eq!(snapshot["captured_at"], 1000.0);
        let limits = crate::plan_limits::parse_snapshot(&snapshot.to_string(), 1001.0).unwrap();
        assert_eq!(limits.monthly.unwrap().used_percentage, 5.0);
    }

    #[test]
    fn unlimited_or_unreadable_records_give_nothing() {
        assert!(snapshot_from_limit_info(r#"{"limit": 0, "num_requests_used_since_refresh": 1}"#, 0.0, None).is_none());
        assert!(snapshot_from_limit_info(r#"{"limit": 10, "num_requests_used_since_refresh": 1, "is_unlimited": true}"#, 0.0, None).is_none());
        assert!(snapshot_from_limit_info("not json", 0.0, None).is_none());
        // Decoded once as well as twice.
        assert!(snapshot_from_limit_info(r#"{"limit": 10, "num_requests_used_since_refresh": 1}"#, 0.0, None).is_some());
    }

    /// Writes the real snapshot: `cargo test warp_limits_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn warp_limits_live_writes_the_snapshot() {
        write_snapshot().unwrap();
        let limits = crate::plan_limits::limits_for_analyzer("Warp").unwrap();
        let login = crate::plan_limits::login_for_analyzer("Warp");
        println!("monthly {:?} login set {}", limits.monthly, login.is_some());
    }
}

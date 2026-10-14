//! Droid (Factory) plan limits. Droid's status line payload carries no usage, and Droid keeps
//! none on disk; its `/usage` panel asks `GET /api/billing/limits`. With a Factory API key in
//! `~/.config/splitrail/factory_api_key` (or `FACTORY_API_KEY`), this polls the same endpoint and
//! writes a snapshot in the status line format (`limits/droid.json`), so the dashboard's limit
//! rows read it like any Claude account's.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

const LIMITS_URL: &str = "https://api.factory.ai/api/billing/limits";
pub const SNAPSHOT_LABEL: &str = "droid";
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Deserialize)]
struct Window {
    #[serde(rename = "usedPercent")]
    used_percent: Option<f64>,
    /// ISO-8601; null while no window is open (nothing used since the last one ended).
    #[serde(rename = "windowEnd")]
    window_end: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Pool {
    #[serde(rename = "fiveHour")]
    five_hour: Option<Window>,
    weekly: Option<Window>,
    monthly: Option<Window>,
}

#[derive(Debug, Deserialize)]
struct Limits {
    standard: Option<Pool>,
}

#[derive(Debug, Deserialize)]
struct Response {
    limits: Option<Limits>,
    #[serde(rename = "extraUsageBalanceCents")]
    extra_usage_balance_cents: Option<f64>,
}

fn window_json(window: &Option<Window>) -> Value {
    let Some(window) = window else {
        return Value::Null;
    };
    let resets_at = window
        .window_end
        .as_deref()
        .and_then(|end| chrono::DateTime::parse_from_rfc3339(end).ok())
        .map(|end| end.timestamp() as f64);
    json!({ "used_percentage": window.used_percent.unwrap_or(0.0), "resets_at": resets_at })
}

/// The snapshot file for an API response body, or `None` if it has no standard limits.
pub fn snapshot_from_response(body: &str, now: f64) -> Option<Value> {
    let response: Response = serde_json::from_str(body).ok()?;
    let standard = response.limits?.standard?;
    Some(json!({
        "account": SNAPSHOT_LABEL,
        "login": "factory",
        "captured_at": now,
        "rate_limits": {
            "five_hour": window_json(&standard.five_hour),
            "seven_day": window_json(&standard.weekly),
        },
        "monthly": window_json(&standard.monthly),
        "extra_usage_balance_cents": response.extra_usage_balance_cents,
    }))
}

fn api_key() -> Option<String> {
    if let Ok(key) = std::env::var("FACTORY_API_KEY") {
        if !key.trim().is_empty() {
            return Some(key.trim().to_string());
        }
    }
    let path = config_dir()?.join("factory_api_key");
    let key = std::fs::read_to_string(path).ok()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

fn config_dir() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config").join("splitrail"))
}

async fn poll_once(client: &reqwest::Client, key: &str) -> Result<()> {
    let body = client
        .get(LIMITS_URL)
        .bearer_auth(key)
        .send()
        .await
        .context("Failed to reach Factory")?
        .error_for_status()
        .context("Factory rejected the limits request")?
        .text()
        .await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    let snapshot =
        snapshot_from_response(&body, now).context("Factory limits response had no limits")?;
    let dir = config_dir().context("No home directory")?.join("limits");
    std::fs::create_dir_all(&dir)?;
    // Write-then-rename so the dashboard never reads a half-written file.
    let tmp = dir.join(format!(".{SNAPSHOT_LABEL}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&snapshot)?)?;
    std::fs::rename(&tmp, dir.join(format!("{SNAPSHOT_LABEL}.json")))?;
    Ok(())
}

/// One poll now, for checking a key. Errors if no key is configured.
#[allow(dead_code)] // Used by the live test; the binary only runs the loop.
pub async fn poll_now() -> Result<()> {
    let key = api_key().context("No Factory API key configured")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    poll_once(&client, &key).await
}

/// Poll Factory every few minutes for as long as the dashboard runs. Does nothing without a key.
pub async fn run() {
    let Some(key) = api_key() else {
        return;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return;
    };
    let mut reported = false;
    loop {
        match poll_once(&client, &key).await {
            Ok(()) => reported = false,
            Err(e) if !reported => {
                crate::utils::report(format!("Droid limits: {e:#}"));
                reported = true; // Once per failure streak, not every poll.
            }
            Err(_) => {}
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"usesTokenRateLimitsBilling":true,"limits":{"standard":{"fiveHour":{"usedPercent":0,"windowEnd":null,"secondsRemaining":null},"weekly":{"usedPercent":1,"windowEnd":"2026-10-20T11:33:48.094Z","secondsRemaining":555494},"monthly":{"usedPercent":3,"windowEnd":"2026-10-31T19:03:25.215Z","secondsRemaining":1532871}},"core":{"fiveHour":{"usedPercent":0,"windowEnd":null,"secondsRemaining":null}}},"extraUsageBalanceCents":0}"#;

    #[test]
    fn response_becomes_a_status_line_style_snapshot() {
        let snapshot = snapshot_from_response(BODY, 1_791_900_000.0).unwrap();
        let limits =
            crate::plan_limits::parse_snapshot(&snapshot.to_string(), 1_791_900_000.0).unwrap();
        let five = limits.five_hour.unwrap();
        assert_eq!(five.used_percentage, 0.0);
        assert_eq!(five.resets_in_secs, None); // No window open.
        let week = limits.seven_day.unwrap();
        assert_eq!(week.used_percentage, 1.0);
        assert_eq!(
            week.resets_in_secs,
            Some(chrono::DateTime::parse_from_rfc3339("2026-10-20T11:33:48.094Z").unwrap().timestamp() - 1_791_900_000)
        );
        assert_eq!(snapshot["monthly"]["used_percentage"], 3.0);
    }

    /// Hits Factory with the configured key: `cargo test droid_limits -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_poll_writes_the_snapshot() {
        poll_now().await.unwrap();
        assert!(crate::plan_limits::limits_for_analyzer("Droid CLI").is_some());
    }

    #[test]
    fn a_response_without_limits_is_ignored() {
        assert!(snapshot_from_response(r#"{"limits":null}"#, 0.0).is_none());
        assert!(snapshot_from_response("not json", 0.0).is_none());
    }
}

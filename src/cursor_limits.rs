//! Cursor plan usage. Cursor bills by the month, with no 5h or weekly windows, and keeps no usage
//! on disk; its `/usage` panel asks `aiserver.v1.DashboardService/GetCurrentPeriodUsage`. With a
//! Cursor user API key in `~/.config/splitrail/cursor_api_key` (or `CURSOR_API_KEY`), this does
//! what `cursor-agent` does with that key, exchanging it for an access token at
//! `/auth/exchange_user_api_key`, then asks for the current period's usage and writes a snapshot in
//! the status line format (`limits/cursor.json`) with a `monthly` window.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

const API: &str = "https://api2.cursor.sh";
pub const SNAPSHOT_LABEL: &str = "cursor";
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// int64 fields come back as JSON strings (`"billingCycleEnd": "1794534757000"`).
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// The snapshot file for a `GetCurrentPeriodUsage` response, or `None` if it has no plan usage.
pub fn snapshot_from_response(body: &str, now: f64) -> Option<Value> {
    let response: Value = serde_json::from_str(body).ok()?;
    let plan = response.get("planUsage")?;
    let used = number(plan.get("totalPercentUsed")?)?;
    let resets_at = response
        .get("billingCycleEnd")
        .and_then(number)
        .map(|ms| ms / 1000.0);
    let cents = |key: &str| plan.get(key).and_then(number);
    Some(json!({
        "account": SNAPSHOT_LABEL,
        "captured_at": now,
        "monthly": { "used_percentage": used, "resets_at": resets_at },
        // Auto and named models draw on separate allowances; the total is what Cursor's own
        // panel leads with.
        "auto_percent_used": plan.get("autoPercentUsed").and_then(number),
        "api_percent_used": plan.get("apiPercentUsed").and_then(number),
        "total_spend_cents": cents("totalSpend"),
        "included_spend_cents": cents("includedSpend"),
        "limit_cents": cents("limit"),
    }))
}

fn api_key() -> Option<String> {
    if let Ok(key) = std::env::var("CURSOR_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(key.trim().to_string());
    }
    let key = std::fs::read_to_string(config_dir()?.join("cursor_api_key")).ok()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

fn config_dir() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config").join("splitrail"))
}

async fn post(client: &reqwest::Client, path: &str, token: &str, body: Value) -> Result<String> {
    Ok(client
        .post(format!("{API}{path}"))
        .bearer_auth(token)
        .header("Connect-Protocol-Version", "1")
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .context("Failed to reach Cursor")?
        .error_for_status()
        .with_context(|| format!("Cursor rejected {path}"))?
        .text()
        .await?)
}

async fn poll_once(client: &reqwest::Client, key: &str) -> Result<()> {
    // Access tokens expire; exchanging every poll is one extra request per five minutes.
    let exchange: Value =
        serde_json::from_str(&post(client, "/auth/exchange_user_api_key", key, json!({})).await?)?;
    let token = exchange
        .get("accessToken")
        .and_then(Value::as_str)
        .context("Cursor's key exchange returned no access token")?;
    let body = post(
        client,
        "/aiserver.v1.DashboardService/GetCurrentPeriodUsage",
        token,
        json!({}),
    )
    .await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    let snapshot =
        snapshot_from_response(&body, now).context("Cursor usage response had no plan usage")?;
    let dir = config_dir().context("No home directory")?.join("limits");
    std::fs::create_dir_all(&dir)?;
    // Write-then-rename so the dashboard never reads a half-written file.
    let tmp = dir.join(format!(".{SNAPSHOT_LABEL}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&snapshot)?)?;
    std::fs::rename(&tmp, dir.join(format!("{SNAPSHOT_LABEL}.json")))?;
    refresh_charges(client, token, now).await
}

/// One billed request from `GetFilteredUsageEvents`, as stored in `cursor/charges.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Charge {
    pub conversation_id: String,
    /// Epoch milliseconds.
    pub ts: i64,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// What Cursor charged against the plan, in cents.
    pub cents: f64,
}

/// The token-billed events of a `GetFilteredUsageEvents` page. Requests without token usage
/// (counted per request) carry nothing to attribute.
pub fn charges_from_response(body: &str) -> Option<(Vec<Charge>, usize)> {
    let response: Value = serde_json::from_str(body).ok()?;
    let events = response
        .get("usageEventsDisplay")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let count = |usage: &Value, key: &str| usage.get(key).and_then(number).unwrap_or(0.0) as u64;
    let charges = events
        .iter()
        .filter_map(|event| {
            let usage = event.get("tokenUsage")?;
            Some(Charge {
                conversation_id: event.get("conversationId")?.as_str()?.to_string(),
                ts: number(event.get("timestamp")?)? as i64,
                model: event
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input_tokens: count(usage, "inputTokens"),
                output_tokens: count(usage, "outputTokens"),
                cache_read_tokens: count(usage, "cacheReadTokens"),
                cache_write_tokens: count(usage, "cacheWriteTokens"),
                cents: event
                    .get("chargedCents")
                    .or_else(|| usage.get("totalCents"))
                    .and_then(number)
                    .unwrap_or(0.0),
            })
        })
        .collect();
    Some((charges, events.len()))
}

pub fn charges_file() -> Option<PathBuf> {
    Some(config_dir()?.join("cursor").join("charges.jsonl"))
}

const PAGE_SIZE: usize = 100;
/// How far back the first fetch goes, and how long charges are kept.
const HISTORY_MS: i64 = 60 * 86_400_000;
/// Events can show up a little after the request; each fetch re-reads this much of the past.
const OVERLAP_MS: i64 = 3_600_000;

/// Fetch the usage events since the last fetch into `cursor/charges.jsonl`, which the Cursor
/// Agent analyzer reads for each turn's real charged cost and full token counts.
async fn refresh_charges(client: &reqwest::Client, token: &str, now: f64) -> Result<()> {
    let path = charges_file().context("No home directory")?;
    let now_ms = (now * 1000.0) as i64;
    let mut charges: Vec<Charge> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let since = charges
        .iter()
        .map(|c| c.ts - OVERLAP_MS)
        .max()
        .unwrap_or(now_ms - HISTORY_MS);
    let mut fresh = Vec::new();
    for page in 1.. {
        let body = post(
            client,
            "/aiserver.v1.DashboardService/GetFilteredUsageEvents",
            token,
            json!({"teamId": 0, "startDate": since.to_string(), "endDate": now_ms.to_string(),
                   "page": page, "pageSize": PAGE_SIZE}),
        )
        .await?;
        let (batch, events) =
            charges_from_response(&body).context("Cursor usage events response was malformed")?;
        fresh.extend(batch);
        if events < PAGE_SIZE || page >= 100 {
            break;
        }
    }
    let seen: std::collections::HashSet<(String, i64)> = fresh
        .iter()
        .map(|c| (c.conversation_id.clone(), c.ts))
        .collect();
    charges.retain(|c| {
        c.ts >= now_ms - HISTORY_MS && !seen.contains(&(c.conversation_id.clone(), c.ts))
    });
    charges.extend(fresh);
    charges.sort_by_key(|c| c.ts);
    let mut out = String::new();
    for charge in &charges {
        out.push_str(&serde_json::to_string(charge)?);
        out.push('\n');
    }
    std::fs::create_dir_all(path.parent().context("charges path has no parent")?)?;
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?)
}

/// One poll now, for checking a key. Errors if no key is configured.
#[allow(dead_code)] // Used by the live test; the binary only runs the loop.
pub async fn poll_now() -> Result<()> {
    let key = api_key().context("No Cursor API key configured")?;
    poll_once(&client()?, &key).await
}

/// Poll Cursor every few minutes for as long as the dashboard runs. Does nothing without a key.
pub async fn run() {
    let Some(key) = api_key() else {
        return;
    };
    let Ok(client) = client() else {
        return;
    };
    let mut reported = false;
    loop {
        match poll_once(&client, &key).await {
            Ok(()) => reported = false,
            Err(e) if !reported => {
                crate::utils::report(format!("Cursor limits: {e:#}"));
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

    const BODY: &str = r#"{"billingCycleStart":"1791856357000","billingCycleEnd":"1794534757000","planUsage":{"totalSpend":4080,"includedSpend":2000,"bonusSpend":2080,"limit":2000,"autoPercentUsed":7.3,"apiPercentUsed":35.333333333333336,"totalPercentUsed":8.634920634920636},"spendLimitUsage":{"limitType":"user"},"enabled":true}"#;

    #[test]
    fn response_becomes_a_monthly_snapshot() {
        let now = 1_791_900_000.0;
        let snapshot = snapshot_from_response(BODY, now).unwrap();
        assert_eq!(snapshot["total_spend_cents"], 4080.0);
        let limits = crate::plan_limits::parse_snapshot(&snapshot.to_string(), now).unwrap();
        assert!(limits.five_hour.is_none() && limits.seven_day.is_none());
        let month = limits.monthly.unwrap();
        assert!((month.used_percentage - 8.6349).abs() < 1e-3);
        assert_eq!(month.resets_in_secs, Some(1_794_534_757 - 1_791_900_000));
    }

    #[test]
    fn usage_events_become_charges() {
        let body = r#"{"totalUsageEventsCount":2,"usageEventsDisplay":[{"timestamp":"1791961120280","model":"default","tokenUsage":{"inputTokens":9763,"outputTokens":352,"cacheReadTokens":3392,"totalCents":3.6265},"chargedCents":3.6265,"conversationId":"c1"},{"timestamp":"1791961120000","model":"grok-bot-default","requestsCosts":1,"conversationId":"c2"}]}"#;
        let (charges, events) = charges_from_response(body).unwrap();
        assert_eq!(events, 2);
        assert_eq!(charges.len(), 1); // The request-counted event has no tokens.
        let c = &charges[0];
        assert_eq!(
            (
                c.ts,
                c.input_tokens,
                c.output_tokens,
                c.cache_read_tokens,
                c.cache_write_tokens
            ),
            (1_791_961_120_280, 9763, 352, 3392, 0)
        );
        assert_eq!(c.cents, 3.6265);
    }

    #[test]
    fn a_response_without_plan_usage_is_ignored() {
        assert!(snapshot_from_response(r#"{"enabled":false}"#, 0.0).is_none());
        assert!(snapshot_from_response("not json", 0.0).is_none());
    }

    /// Hits Cursor with the configured key: `cargo test cursor_limits -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_poll_writes_the_snapshot() {
        poll_now().await.unwrap();
        let limits = crate::plan_limits::limits_for_analyzer("Cursor Agent").unwrap();
        assert!(limits.monthly.is_some());
    }
}

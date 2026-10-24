//! `~/.config/splitrail/status.json`: the live numbers for the menu bar (SwiftBar,
//! `scripts/swiftbar/splitrail.5s.sh`), rewritten about every second while the dashboard or
//! `splitrail-dashboard mqtt` runs. Other machines' columns are included, so the total covers
//! every machine on the broker.
//!
//! ```json
//! {"updated": 1792130000.0, "tks": 213.4, "cost_today": 41.2,
//!  "tools": [{"name": "Claude (main)", "tks": 137.4, "cost_today": 30.1,
//!             "limits": {"five_hour": {"used": 23.0, "resets_in": 5400}, ...}}],
//!  "publishers": [{"host": "devbox-a", "version": "2.0.0-dash-0.53", "online": true, "behind": false}],
//!  "month": {"goal_tks": 150.0, "avg_tks": 132.4, "needed_tks": 171.0, "output_tokens": 171584000,
//!            "remaining_secs": 1340000}}
//! ```
//!
//! `month` is the pace against `[goal] monthly_tks`: output tokens since the 1st (local time)
//! over the seconds since then, and the rate the rest of the month needs to finish on the goal.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::watch;

use crate::plan_limits::WindowStatus;
use crate::types::MultiAnalyzerStats;

/// How often the file is rewritten, unless working it out takes long enough that this would cost
/// more than `MAX_SHARE` of a core; then proportionally less often.
const INTERVAL: Duration = Duration::from_secs(1);
const MAX_SHARE: u32 = 20; // At most 1/20 of the time: 5%.

pub fn path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/status.json"))
}

fn round(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (value * factor).round() / factor
}

fn window(window: Option<WindowStatus>) -> Value {
    window.map_or(Value::Null, |w| {
        json!({ "used": round(w.used_percentage, 1), "resets_in": w.resets_in_secs })
    })
}

/// Local midnight on `date`, or `None` in a gap (a DST change at midnight).
fn local_midnight(date: NaiveDate) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest()
}

/// The month's pace against `goal_tks`: output tokens since the 1st, from the daily totals (which
/// cover other machines too), averaged over the time elapsed, and what the rest needs.
pub fn month_pace(stats: &MultiAnalyzerStats, now: DateTime<Local>, goal_tks: f64) -> Value {
    let first = now.date_naive().with_day(1).unwrap_or(now.date_naive());
    let next = first.checked_add_months(chrono::Months::new(1)).unwrap_or(first);
    let (Some(start), Some(end)) = (local_midnight(first), local_midnight(next)) else {
        return Value::Null;
    };
    let from = first.format("%Y-%m-%d").to_string();
    let output_tokens: u64 = stats
        .analyzer_stats
        .iter()
        .flat_map(|tool| tool.daily_stats.range(from.clone()..))
        .map(|(_, day)| day.stats.output_tokens)
        .sum();
    let elapsed = (now - start).num_seconds().max(1) as f64;
    let remaining = (end - now).num_seconds().max(0) as f64;
    let target = goal_tks * (end - start).num_seconds() as f64;
    let needed = if remaining > 0.0 { ((target - output_tokens as f64) / remaining).max(0.0) } else { 0.0 };
    json!({
        "goal_tks": goal_tks,
        "avg_tks": round(output_tokens as f64 / elapsed, 1),
        "needed_tks": round(needed, 1),
        "output_tokens": output_tokens,
        "remaining_secs": remaining as i64,
    })
}

/// The file's contents for `stats` as of now. Tools idle today with no limits are left out.
pub fn payload(stats: &MultiAnalyzerStats, goal_tks: f64) -> Value {
    let now = Utc::now();
    let mut tools: Vec<_> = crate::mqtt::tool_states(stats, now, crate::plan_limits::limits_for_analyzer)
        .into_iter()
        .filter(|t| t.tks > 0.0 || t.cost_today > 0.0 || t.limits.is_some())
        .collect();
    tools.sort_by(|a, b| b.tks.total_cmp(&a.tks).then(b.cost_today.total_cmp(&a.cost_today)));
    // Other logins on this machine's Claude accounts: each its own allowance, one `/login` away.
    let other_logins: Vec<Value> = tools
        .iter()
        .filter(|t| t.name.starts_with("Claude (") && !t.name.contains('@'))
        .flat_map(|t| {
            let tool_stats = stats.analyzer_stats.iter().find(|s| s.analyzer_name == t.name).cloned();
            crate::plan_limits::other_logins(&t.name).into_iter().map(move |o| {
                // What this login has used in its own week, measured once it has run (BZL-283).
                let spend = match (&tool_stats, o.seven_day.resets_in_secs) {
                    (Some(s), Some(left)) => {
                        let start = now + chrono::Duration::seconds(left) - chrono::Duration::days(7);
                        let spend = crate::advisor::login_spend(s, start, &o.account, false);
                        (spend.cost > 0.0).then(|| spend.to_json())
                    }
                    _ => None,
                };
                json!({
                    "name": format!("{} · {}", t.name, short_login(&o.account)),
                    "account": o.account,
                    "tks": 0.0,
                    "cost_today": 0.0,
                    "switch": true,
                    "limits": {"five_hour": Value::Null, "monthly": Value::Null,
                               "seven_day": {"used": round(o.seven_day.used_percentage, 1),
                                             "resets_in": o.seven_day.resets_in_secs,
                                             "spend": spend}},
                })
            })
        })
        .collect();
    let publishers: Vec<Value> = crate::rollup::publishers()
        .into_iter()
        .map(|p| json!({ "host": p.host, "version": p.version, "online": p.online, "behind": p.behind() }))
        .collect();
    json!({
        "updated": now.timestamp_millis() as f64 / 1000.0,
        "version": env!("CARGO_PKG_VERSION"),
        "tks": round(tools.iter().map(|t| t.tks).sum(), 1),
        "cost_today": round(tools.iter().map(|t| t.cost_today).sum(), 2),
        "tools": tools.iter().map(|t| json!({
            "name": t.name,
            "account": crate::plan_limits::account_for_analyzer(&t.name),
            "tks": round(t.tks, 1),
            "cost_today": round(t.cost_today, 2),
            // With each window's spend so far, for the quota advisor.
            "limits": stats
                .analyzer_stats
                .iter()
                .find(|s| s.analyzer_name == t.name)
                .and_then(|s| crate::advisor::tool_json(s, now))
                .map(|j| j["limits"].clone())
                .unwrap_or_else(|| json!({
                    "five_hour": window(t.limits.and_then(|l| l.five_hour)),
                    "seven_day": window(t.limits.and_then(|l| l.seven_day)),
                    "monthly": window(t.limits.and_then(|l| l.monthly)),
                })),
        })).chain(other_logins).collect::<Vec<_>>(),
        "publishers": publishers,
        "month": month_pace(stats, now.with_timezone(&Local), goal_tks),
    })
}

/// `a@virti.us (Virtius)` -> `a@Virtius`; `a@virti.us (personal)` -> `a`.
fn short_login(account: &str) -> String {
    let (email, org) = account.split_once(" (").map_or((account, ""), |(e, o)| (e, o.trim_end_matches(')')));
    let name = email.split('@').next().unwrap_or(email);
    if org.is_empty() || org == "personal" { name.to_string() } else { format!("{name}@{org}") }
}

fn write(payload: &Value) -> std::io::Result<()> {
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write-then-rename so the menu bar never reads a half-written file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(payload)?)?;
    std::fs::rename(tmp, path)
}

/// Rewrite the file about every second (tk/s decays between messages, so on a timer as well as
/// on change) for as long as the stats channel lives.
pub async fn run(mut stats_rx: watch::Receiver<MultiAnalyzerStats>, goal_tks: f64) {
    let mut wait = INTERVAL;
    loop {
        tokio::select! {
            changed = stats_rx.changed() => if changed.is_err() { break },
            _ = tokio::time::sleep(wait) => {}
        }
        let started = std::time::Instant::now();
        let payload = {
            let stats = stats_rx.borrow_and_update();
            // Keeps records.json current (and announces new records) even with the view closed.
            let tools: Vec<&crate::types::AgenticCodingToolStats> = stats.analyzer_stats.iter().map(|s| s.as_ref()).collect();
            crate::records::board(&tools, goal_tks);
            payload(&stats, goal_tks)
        };
        wait = INTERVAL.max(started.elapsed() * MAX_SHARE);
        if let Err(e) = tokio::task::spawn_blocking(move || write(&payload)).await.unwrap_or(Ok(())) {
            crate::utils::log_error(format!("Status file: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};

    fn tool(name: &str, output: u64, cost: f64, age_secs: i64) -> AgenticCodingToolStats {
        let now = Utc::now();
        // A recent message stays today even when the test runs just after midnight.
        let midnight = local_midnight(now.with_timezone(&Local).date_naive()).map(|m| m.with_timezone(&Utc));
        let date = match midnight {
            Some(midnight) if age_secs < 900 => (now - chrono::Duration::seconds(age_secs)).max(midnight),
            _ => now - chrono::Duration::seconds(age_secs),
        };
        AgenticCodingToolStats {
            daily_stats: Default::default(),
            num_conversations: 1,
            analyzer_name: name.into(),
            messages: vec![ConversationMessage {
                application: Application::ClaudeCode,
                date,
                project_hash: String::new(),
                conversation_hash: "c".into(),
                local_hash: None,
                global_hash: name.to_string(),
                model: Some("m".into()),
                stats: Stats { output_tokens: output, cost, ..Default::default() },
                role: MessageRole::Assistant,
                content: None,
            }],
        }
    }

    #[test]
    fn totals_every_tool_busiest_first_skipping_idle_ones() {
        let stats = MultiAnalyzerStats {
            analyzer_stats: vec![
                tool("Status Test Quiet", 900, 0.5, 60).into(),
                tool("Status Test Busy", 9000, 2.0, 60).into(),
                tool("Status Test Idle", 0, 0.0, 10 * 86_400).into(),
            ],
        };
        let payload = payload(&stats, 150.0);
        let names: Vec<&str> = payload["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Status Test Busy", "Status Test Quiet"]);
        assert_eq!(payload["tks"], 11.0); // 9,900 output tokens over the 15-minute window.
        assert_eq!(payload["cost_today"], 2.5);
    }

    #[test]
    fn other_logins_get_short_names() {
        assert_eq!(short_login("a@virti.us (Virtius)"), "a@Virtius");
        assert_eq!(short_login("a@virti.us (personal)"), "a");
        assert_eq!(short_login("plain"), "plain");
    }

    #[test]
    fn month_pace_averages_since_the_first_and_spreads_the_rest() {
        let mut claude = tool("Status Test Month", 0, 0.0, 60);
        for (date, output) in [("2026-09-30", 999_999_999), ("2026-10-01", 50_000_000), ("2026-10-02", 50_000_000)] {
            let mut day = crate::types::DailyStats::default();
            day.stats.output_tokens = output;
            claude.daily_stats.insert(date.into(), day);
        }
        let stats = MultiAnalyzerStats { analyzer_stats: vec![claude.into()] };
        // Noon on Oct 3: 2.5 days in, 28.5 days left of 31.
        let now = Local.with_ymd_and_hms(2026, 10, 3, 12, 0, 0).unwrap();
        let month = month_pace(&stats, now, 150.0);
        let elapsed = (now - local_midnight(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()).unwrap()).num_seconds() as f64;
        assert_eq!(month["output_tokens"], 100_000_000);
        assert_eq!(month["avg_tks"], round(100_000_000.0 / elapsed, 1));
        let remaining = month["remaining_secs"].as_f64().unwrap();
        let needed = (150.0 * (elapsed + remaining) - 100_000_000.0) / remaining;
        assert!((month["needed_tks"].as_f64().unwrap() - needed).abs() < 0.1);
    }
}

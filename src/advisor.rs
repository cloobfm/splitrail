//! Quota answers for agents (BZL-137): where every plan window stands, when it resets, whether
//! today's pace runs out early or leaves quota unused, and so which tool and account to spend on
//! next. Read from `status.json`, which the running dashboard rewrites every second with every
//! machine's columns (devboxes included), so answers are instant and cover all accounts.

use chrono::{DateTime, Duration, Local, Utc};
use serde_json::{Value, json};

/// Seconds in each window, for working out how far through it we are.
fn window_secs(key: &str) -> f64 {
    match key {
        "five_hour" => 5.0 * 3600.0,
        "seven_day" => 7.0 * 86_400.0,
        _ => 30.44 * 86_400.0, // monthly
    }
}

fn window_secs_label(label: &str) -> f64 {
    match label {
        "5h" => window_secs("five_hour"),
        "week" => window_secs("seven_day"),
        _ => window_secs("monthly"),
    }
}

fn window_label(key: &str) -> &'static str {
    match key {
        "five_hour" => "5h",
        "seven_day" => "week",
        _ => "month",
    }
}

/// A used share is "full" from here: the tool will refuse soon, so it is not somewhere to spend.
const FULL: f64 = 95.0;

/// One plan window of one tool or account.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub tool: String,
    pub window: &'static str,
    pub used: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_in_secs: Option<i64>,
    /// Used share at the reset if use carries on at the window's average rate so far.
    pub projected: Option<f64>,
}

impl Window {
    /// Seconds until the window is used up at its average rate so far, when that comes before
    /// the reset.
    pub fn runs_out_in_secs(&self) -> Option<i64> {
        let left = self.resets_in_secs?;
        let projected = self.projected?;
        if projected <= 100.0 || self.used >= 100.0 {
            return None;
        }
        let length = window_secs_label(self.window);
        let elapsed = length - left as f64;
        let per_sec = self.used / elapsed;
        Some(((100.0 - self.used) / per_sec) as i64)
    }

    /// Quota projected to go unused at the reset, as a share of the window.
    pub fn expiring(&self) -> f64 {
        (100.0 - self.projected.unwrap_or(self.used)).max(0.0)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "tool": self.tool,
            "window": self.window,
            "used_percent": round1(self.used),
            "resets_at": self.resets_at.map(|t| t.with_timezone(&Local).to_rfc3339()),
            "resets_in": self.resets_in_secs.map(human_secs),
            "projected_percent_at_reset": self.projected.map(round1),
            "unused_at_reset_percent": round1(self.expiring()),
            "runs_out_in": self.runs_out_in_secs().map(human_secs),
            "full": self.used >= FULL,
        })
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `3d4h`, `2h05m`, `12m`.
pub fn human_secs(secs: i64) -> String {
    if secs <= 0 {
        return "now".into();
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

/// Every plan window in a status payload, with its reset time and pace, as of `now`.
pub fn windows(status: &Value, now: DateTime<Utc>) -> Vec<Window> {
    let mut out = Vec::new();
    for tool in status.get("tools").and_then(Value::as_array).into_iter().flatten() {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or("?");
        for key in ["five_hour", "seven_day", "monthly"] {
            let Some(w) = tool.get("limits").and_then(|l| l.get(key)).filter(|w| !w.is_null()) else {
                continue;
            };
            let Some(used) = w.get("used").and_then(Value::as_f64) else { continue };
            let resets_in = w.get("resets_in").and_then(Value::as_i64);
            if resets_in.is_none() && used <= 0.0 {
                continue; // An unused window with no reset says nothing.
            }
            let length = window_secs(key);
            let projected = resets_in.and_then(|left| {
                let elapsed = length - left as f64;
                // Too early in the window to say; the average would be noise.
                (elapsed > length * 0.05).then(|| used * length / elapsed)
            });
            out.push(Window {
                tool: name.to_string(),
                window: window_label(key),
                used,
                resets_at: resets_in.map(|s| now + Duration::seconds(s)),
                resets_in_secs: resets_in,
                projected,
            });
        }
    }
    merge_shared_accounts(out)
}

/// Columns signed in to the same account (a devbox using the Mac's Claude login) report the same
/// window: same usage and the same reset to within two minutes. Keep one, named for all of them.
fn merge_shared_accounts(windows: Vec<Window>) -> Vec<Window> {
    // Which columns share every window they report.
    let same = |a: &Window, b: &Window| {
        a.window == b.window
            && (a.used - b.used).abs() < 0.5
            && match (a.resets_at, b.resets_at) {
                (Some(x), Some(y)) => (x - y).num_seconds().abs() <= 120,
                _ => false,
            }
    };
    let mut tools: Vec<String> = windows.iter().map(|w| w.tool.clone()).collect();
    tools.dedup();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for tool in tools.iter().filter(|t| !groups.iter().flatten().any(|g| g == *t)).cloned().collect::<Vec<_>>() {
        if groups.iter().flatten().any(|g| *g == tool) {
            continue;
        }
        let own: Vec<&Window> = windows.iter().filter(|w| w.tool == tool).collect();
        let mut group = vec![tool.clone()];
        for other in tools.iter().filter(|t| **t != tool && !groups.iter().flatten().any(|g| g == *t)) {
            let theirs: Vec<&Window> = windows.iter().filter(|w| &w.tool == other).collect();
            if !own.is_empty()
                && own.len() == theirs.len()
                && own.iter().all(|a| theirs.iter().any(|b| same(a, b)))
            {
                group.push(other.clone());
            }
        }
        groups.push(group);
    }
    groups
        .into_iter()
        .flat_map(|group| {
            let name = group.join(" = ");
            windows
                .iter()
                .filter(|w| w.tool == group[0])
                .map(|w| Window { tool: name.clone(), ..w.clone() })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// One ranked suggestion of where to spend tokens next.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub tool: String,
    pub available: bool,
    pub reason: String,
    /// Higher first.
    pub score: f64,
}

/// Rank tools by quota that would otherwise be lost: the window furthest from being used up
/// by its reset, weighted by how soon that reset comes (unused share per day left). A tool with
/// any window nearly full is not available until that window resets, and goes last.
pub fn where_to_spend(windows: &[Window]) -> Vec<Pick> {
    let mut tools: Vec<&str> = windows.iter().map(|w| w.tool.as_str()).collect();
    tools.sort();
    tools.dedup();
    let mut picks: Vec<Pick> = tools
        .into_iter()
        .map(|tool| {
            let own: Vec<&Window> = windows.iter().filter(|w| w.tool == tool).collect();
            if let Some(full) = own.iter().filter(|w| w.used >= FULL).min_by_key(|w| w.resets_in_secs) {
                let back = full.resets_in_secs.map(human_secs).unwrap_or_else(|| "?".into());
                return Pick {
                    tool: tool.to_string(),
                    available: false,
                    reason: format!("{} window {:.0}% used; back in {back}", full.window, full.used),
                    score: -1.0 - full.resets_in_secs.unwrap_or(0) as f64 / 86_400.0,
                };
            }
            // The plan's real allowance is its longest window (week or month); the 5-hour one
            // only gates when it can be used. Score: share expected to expire, per day left.
            let best = own
                .iter()
                .max_by(|a, b| window_secs_label(a.window).total_cmp(&window_secs_label(b.window)))
                .map(|w| {
                    let days = (w.resets_in_secs.unwrap_or(0) as f64 / 86_400.0).max(1.0 / 24.0);
                    (w, w.expiring() / days)
                });
            match best {
                Some((w, score)) => {
                    let pace = match (w.projected, w.runs_out_in_secs()) {
                        (_, Some(out)) => format!("on pace to run out in {}", human_secs(out)),
                        (Some(p), None) => format!("on pace for {:.0}%", p.min(999.0)),
                        (None, None) => "too early to project".into(),
                    };
                    let resets = w.resets_in_secs.map(human_secs).unwrap_or_else(|| "?".into());
                    Pick {
                        tool: tool.to_string(),
                        available: true,
                        reason: format!(
                            "{} window {:.0}% used, {pace}; about {:.0}% would go unused at its reset in {resets}",
                            w.window,
                            w.used,
                            w.expiring()
                        ),
                        score,
                    }
                }
                None => Pick { tool: tool.to_string(), available: true, reason: "no window data".into(), score: 0.0 },
            }
        })
        .collect();
    picks.sort_by(|a, b| b.score.total_cmp(&a.score));
    picks
}

/// How old a status payload is, in seconds.
pub fn age_secs(status: &Value, now: DateTime<Utc>) -> f64 {
    let updated = status.get("updated").and_then(Value::as_f64).unwrap_or(0.0);
    now.timestamp_millis() as f64 / 1000.0 - updated
}

/// The status file, or an error saying the dashboard is not running.
pub fn read_status() -> anyhow::Result<Value> {
    let path = crate::status_file::path().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let text = std::fs::read_to_string(&path).map_err(|_| {
        anyhow::anyhow!("{} not found: the splitrail dashboard writes it while it runs", path.display())
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// A note when the numbers are old (the dashboard stopped).
fn freshness(status: &Value, now: DateTime<Utc>) -> Value {
    let age = age_secs(status, now);
    if age > 120.0 {
        json!(format!("status is {} old; the dashboard may have stopped", human_secs(age as i64)))
    } else {
        Value::Null
    }
}

/// The `quotas` answer.
pub fn quotas_answer(status: &Value, now: DateTime<Utc>) -> Value {
    let mut all = windows(status, now);
    all.sort_by_key(|w| w.resets_in_secs.unwrap_or(i64::MAX));
    json!({
        "as_of": now.with_timezone(&Local).to_rfc3339(),
        "stale": freshness(status, now),
        "windows": all.iter().map(Window::to_json).collect::<Vec<_>>(),
    })
}

/// The `where_to_spend` answer.
pub fn where_to_spend_answer(status: &Value, now: DateTime<Utc>) -> Value {
    let picks = where_to_spend(&windows(status, now));
    json!({
        "as_of": now.with_timezone(&Local).to_rfc3339(),
        "stale": freshness(status, now),
        "how": "Each tool's plan is judged by its longest window (week or month): the share projected to go unused at that reset at the pace so far, per day until the reset. A 5-hour window only gates availability: any window at 95%+ makes the tool unavailable until it resets. Shares are of each plan's own allowance, so a large share of a small plan can rank above a smaller share of a large one. Tools without plan limits (pay-per-token, local) are not ranked.",
        "ranking": picks.iter().map(|p| json!({
            "tool": p.tool, "available": p.available, "reason": p.reason, "score": round1(p.score),
        })).collect::<Vec<_>>(),
    })
}

/// The `usage` answer: today's numbers per tool and the month's pace.
pub fn usage_answer(status: &Value, now: DateTime<Utc>) -> Value {
    json!({
        "as_of": now.with_timezone(&Local).to_rfc3339(),
        "stale": freshness(status, now),
        "tks_now": status.get("tks"),
        "cost_today": status.get("cost_today"),
        "month": status.get("month"),
        "tools": status.get("tools").and_then(Value::as_array).map(|tools| tools.iter().map(|t| json!({
            "tool": t.get("name"), "tks_now": t.get("tks"), "cost_today": t.get("cost_today"),
        })).collect::<Vec<_>>()),
        "machines": status.get("publishers"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Value {
        json!({"updated": 1000.0, "tools": [
            // Weekly 40% used with 1 day left of 7: pace 46.7%, about 53% would expire.
            {"name": "Claude (v)", "limits": {"five_hour": {"used": 10.0, "resets_in": 9000},
                                             "seven_day": {"used": 40.0, "resets_in": 86_400}}},
            // Weekly 50% used with 6 days left: early, but pace 350%: little expires.
            {"name": "Claude (main)", "limits": {"seven_day": {"used": 50.0, "resets_in": 518_400}}},
            // 5h window full: unavailable for 30 minutes.
            {"name": "Claude@dev-a", "limits": {"five_hour": {"used": 97.0, "resets_in": 1800}}},
            {"name": "Kiro CLI", "limits": {"monthly": {"used": 2.0, "resets_in": 1_296_000}}},
            // The same account as Claude (main), seen from a devbox.
            {"name": "Claude@dev-b", "limits": {"seven_day": {"used": 50.0, "resets_in": 518_430}}},
            // An empty window with no reset: dropped.
            {"name": "Droid CLI", "limits": {"five_hour": {"used": 0.0, "resets_in": null}}},
            {"name": "Ollama", "limits": {"five_hour": null, "seven_day": null, "monthly": null}},
        ]})
    }

    #[test]
    fn pace_projects_the_window_to_its_reset() {
        let now = DateTime::from_timestamp(1000, 0).unwrap();
        let all = windows(&status(), now);
        let week = all.iter().find(|w| w.tool == "Claude (v)" && w.window == "week").unwrap();
        assert!((week.projected.unwrap() - 40.0 * 7.0 / 6.0).abs() < 1e-9);
        assert_eq!(week.resets_at, Some(now + Duration::days(1)));
        assert!(all.iter().all(|w| w.tool != "Ollama"), "no limits, no windows");
        assert!(all.iter().all(|w| w.tool != "Droid CLI"), "empty window with no reset");
        assert!(all.iter().any(|w| w.tool == "Claude (main) = Claude@dev-b"), "{all:?}");
        assert_eq!(all.iter().filter(|w| w.tool.contains("Claude@dev-b")).count(), 1);
    }

    #[test]
    fn spend_where_quota_is_about_to_expire_and_skip_full_ones() {
        let now = DateTime::from_timestamp(1000, 0).unwrap();
        let picks = where_to_spend(&windows(&status(), now));
        assert_eq!(picks[0].tool, "Claude (v)", "{picks:?}");
        let last = picks.last().unwrap();
        assert_eq!((last.tool.as_str(), last.available), ("Claude@dev-a", false));
        assert!(last.reason.contains("back in 30m"));
        let main = picks.iter().find(|p| p.tool == "Claude (main) = Claude@dev-b").unwrap();
        assert!(main.score < picks[0].score, "on pace to use its week: little expires");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human_secs(90_000), "1d1h");
        assert_eq!(human_secs(7_500), "2h05m");
        assert_eq!(human_secs(600), "10m");
        assert_eq!(human_secs(0), "now");
    }

    #[test]
    fn a_fast_pace_says_when_it_runs_out() {
        // 70% of a week used in 3.5 days: 20%/day, so the last 30% lasts 1.5 days.
        let w = Window {
            tool: "t".into(), window: "week", used: 70.0, resets_at: None,
            resets_in_secs: Some(302_400), projected: Some(140.0),
        };
        assert_eq!(w.runs_out_in_secs(), Some(129_600));
    }
}

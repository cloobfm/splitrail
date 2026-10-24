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

/// Tokens and cost spent inside one window, from the daily totals (the first day counted only
/// from the window's start).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    /// API-equivalent dollars, as the dashboard prices them.
    pub cost: f64,
}

impl Spend {
    fn add(&mut self, other: &Spend) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self.cost += other.cost;
    }

    /// What the same tokens would have cost on `model`.
    pub fn cost_on(&self, model: &str) -> f64 {
        crate::models::calculate_total_cost(
            model,
            self.input as u64,
            self.output as u64,
            self.cache_write as u64,
            self.cache_read as u64,
        )
    }

    pub fn to_json(self) -> Value {
        json!({"input": self.input.round(), "output": self.output.round(), "cache_write": self.cache_write.round(),
               "cache_read": self.cache_read.round(), "cost": round2(self.cost)})
    }

    fn from_json(v: &Value) -> Option<Spend> {
        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        v.is_object().then(|| Spend {
            input: f("input"),
            output: f("output"),
            cache_write: f("cache_write"),
            cache_read: f("cache_read"),
            cost: f("cost"),
        })
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// A tool's spend since `start`, from its per-day totals. The first day counts only the share
/// of it after `start` (daily totals have no hours), so this is close but not exact.
pub fn window_spend(stats: &crate::types::AgenticCodingToolStats, start: DateTime<Utc>) -> Spend {
    let start_local = start.with_timezone(&Local);
    let first_day = start_local.date_naive();
    let first_share = first_day
        .succ_opt()
        .and_then(|next| next.and_hms_opt(0, 0, 0))
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map_or(1.0, |midnight| ((midnight - start_local).num_seconds() as f64 / 86_400.0).clamp(0.0, 1.0));
    let mut spend = Spend::default();
    for (day, totals) in stats.daily_stats.range(first_day.format("%Y-%m-%d").to_string()..) {
        let share = if *day == first_day.format("%Y-%m-%d").to_string() { first_share } else { 1.0 };
        let t = &totals.stats;
        spend.add(&Spend {
            input: t.input_tokens as f64 * share,
            output: t.output_tokens as f64 * share,
            cache_write: t.cache_creation_tokens as f64 * share,
            cache_read: t.cache_read_tokens as f64 * share,
            cost: t.cost * share,
        });
    }
    spend
}

/// A login's spend since `start` (BZL-283), from the slices credited to it, to the hour.
/// `with_unknown` also counts usage from before login history was kept, which belongs to
/// whoever was signed in then: the signed-in login's row takes it, other logins' do not.
pub fn login_spend(
    stats: &crate::types::AgenticCodingToolStats,
    start: DateTime<Utc>,
    login: &str,
    with_unknown: bool,
) -> Spend {
    let start_local = start.with_timezone(&Local);
    let first_day = start_local.date_naive().format("%Y-%m-%d").to_string();
    let first_hour = chrono::Timelike::hour(&start_local) as u8;
    let mut spend = Spend::default();
    for (day, totals) in stats.daily_stats.range(first_day.clone()..) {
        for s in &totals.slices {
            if *day == first_day && s.hour < first_hour {
                continue;
            }
            if s.account == login || (with_unknown && s.account.is_empty()) {
                spend.add(&Spend {
                    input: s.input_tokens as f64,
                    output: s.output_tokens as f64,
                    cache_write: s.cache_creation_tokens as f64,
                    cache_read: s.cache_read_tokens as f64,
                    cost: s.cost,
                });
            }
        }
    }
    spend
}

/// The login on a Claude account that has used the most of its week, signed in or not: usage
/// from before login history was kept most likely came out of its allowance.
fn top_login(analyzer_name: &str) -> Option<String> {
    let signed_in = crate::plan_limits::account_for_analyzer(analyzer_name);
    let signed_in_used = crate::plan_limits::limits_for_analyzer(analyzer_name)
        .and_then(|l| l.seven_day)
        .map_or(0.0, |w| w.used_percentage);
    std::iter::once((signed_in, signed_in_used))
        .chain(crate::plan_limits::other_logins(analyzer_name).into_iter().map(|o| (Some(o.account), o.seven_day.used_percentage)))
        .filter(|(_, used)| *used > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .and_then(|(login, _)| login)
}

/// Whether a column's usage is credited to logins (a Claude account's directory).
fn has_logins(stats: &crate::types::AgenticCodingToolStats) -> bool {
    stats.daily_stats.values().rev().take(14).any(|d| d.slices.iter().any(|s| !s.account.is_empty()))
}

/// Output tokens per second over the last 15 minutes, as the rest of the dashboard counts it.
pub fn recent_tks(stats: &crate::types::AgenticCodingToolStats, now: DateTime<Utc>) -> f64 {
    let since = now - Duration::minutes(15);
    let output: u64 = stats
        .messages
        .iter()
        .filter(|m| m.role == crate::types::MessageRole::Assistant && m.date >= since && m.date <= now)
        .map(|m| m.stats.output_tokens)
        .sum();
    output as f64 / 900.0
}

/// One tool's entry in the `status.json` shape, with each window's spend so far.
pub fn tool_json(stats: &crate::types::AgenticCodingToolStats, now: DateTime<Utc>) -> Option<Value> {
    let name = stats.analyzer_name.as_str();
    let limits = crate::plan_limits::limits_for_analyzer(name)?;
    let window = |key: &str, w: Option<crate::plan_limits::WindowStatus>| {
        w.map_or(Value::Null, |w| {
            // Signed-in login only, where usage is credited per login (BZL-283): another login's
            // use this week came out of its own allowance. Usage from before logins were
            // recorded goes to whichever login used its week most.
            let signed_in = has_logins(stats).then(|| crate::plan_limits::account_for_analyzer(name)).flatten();
            let spend = w.resets_in_secs.map(|left| {
                let start = now + Duration::seconds(left) - Duration::seconds(window_secs(key) as i64);
                match &signed_in {
                    Some(login) => {
                        let unknown_is_mine = top_login(name).is_none_or(|top| top == *login);
                        login_spend(stats, start, login, unknown_is_mine)
                    }
                    None => window_spend(stats, start),
                }
            });
            json!({"used": w.used_percentage, "resets_in": w.resets_in_secs, "spend": spend.map(Spend::to_json)})
        })
    };
    Some(json!({
        "name": name,
        "account": crate::plan_limits::account_for_analyzer(name),
        "plan": crate::plan_limits::plan_for_analyzer(name).or_else(|| codex_plan(name)),
        "tks": round1(recent_tks(stats, now)),
        "limits": {
            "five_hour": window("five_hour", limits.five_hour),
            "seven_day": window("seven_day", limits.seven_day),
            "monthly": window("monthly", limits.monthly),
        },
    }))
}

/// Models to express a Claude plan's remaining allowance in, as (label, model).
const CLAUDE_MODELS: [(&str, &str); 3] =
    [("Opus 5.5", "claude-opus-5-5"), ("Fable 5.1", "claude-fable-5-1"), ("Sonnet 5.5", "claude-sonnet-5-5")];

/// `1.6M`, `650k`, `900`.
pub fn human_tokens(tokens: f64) -> String {
    if tokens >= 1e6 {
        format!("{:.1}M", tokens / 1e6)
    } else if tokens >= 1e3 {
        format!("{:.0}k", tokens / 1e3)
    } else {
        format!("{tokens:.0}")
    }
}

/// A used share is "full" from here: the tool will refuse soon, so it is not somewhere to spend.
const FULL: f64 = 95.0;

/// One plan window of one tool or account.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub tool: String,
    /// The signed-in account (email and organisation, or plan), when known.
    pub account: Option<String>,
    pub window: &'static str,
    pub used: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_in_secs: Option<i64>,
    /// Used share at the reset if use carries on at the window's average rate so far.
    pub projected: Option<f64>,
    /// Tokens and cost spent in the window so far, when the dashboard knows them.
    pub spend: Option<Spend>,
    /// The tool's output tk/s over the last 15 minutes.
    pub now_tks: f64,
    /// Another login on the account, not signed in: reached by `/login`.
    pub switch_login: bool,
    /// The plan (`Max 20x`, `Team Std`), where known.
    pub plan: Option<String>,
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

    /// Seconds between running out and the reset: time without this account at this pace.
    pub fn locked_out_secs(&self) -> Option<i64> {
        Some(self.resets_in_secs? - self.runs_out_in_secs()?)
    }

    /// API-equivalent dollars per 1% of the window, measured from what this window has used.
    pub fn dollars_per_percent(&self) -> Option<f64> {
        let spend = self.spend?;
        (self.used >= 1.0 && spend.cost > 0.0).then(|| spend.cost / self.used)
    }

    /// Dollars of allowance left until the reset.
    pub fn left_dollars(&self) -> Option<f64> {
        Some(self.dollars_per_percent()? * (100.0 - self.used).max(0.0))
    }

    /// Output tokens the rest of the allowance buys, at this window's mix of models and cache.
    pub fn left_output(&self) -> Option<f64> {
        let spend = self.spend?;
        (spend.output > 0.0).then_some(())?;
        Some(spend.output * self.left_dollars()? / spend.cost)
    }

    /// Output tk/s needed from now until the reset to use the whole allowance.
    pub fn need_tks(&self) -> Option<f64> {
        let left = self.resets_in_secs.filter(|s| *s > 0)?;
        Some(self.left_output()? / left as f64)
    }

    /// For a Claude plan: the remaining allowance as output tokens on each model, if this
    /// window's work (its input, cache and output mix) had run on that model.
    pub fn left_output_by_model(&self) -> Vec<(&'static str, f64)> {
        let (Some(spend), Some(left)) = (self.spend, self.left_dollars()) else { return Vec::new() };
        if !self.tool.contains("Claude") || spend.output <= 0.0 {
            return Vec::new();
        }
        CLAUDE_MODELS
            .iter()
            .filter_map(|(label, model)| {
                let cost = spend.cost_on(model);
                (cost > 0.0).then(|| (*label, spend.output * left / cost))
            })
            .collect()
    }

    /// Quota projected to go unused at the reset, as a share of the window.
    pub fn expiring(&self) -> f64 {
        (100.0 - self.projected.unwrap_or(self.used)).max(0.0)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "tool": self.tool,
            "account": self.account,
            "window": self.window,
            "used_percent": round1(self.used),
            "resets_at": self.resets_at.map(|t| t.with_timezone(&Local).to_rfc3339()),
            "resets_in": self.resets_in_secs.map(human_secs),
            "projected_percent_at_reset": self.projected.map(round1),
            "unused_at_reset_percent": round1(self.expiring()),
            "runs_out_in": self.runs_out_in_secs().map(human_secs),
            "locked_out_for": self.locked_out_secs().map(human_secs),
            "spend_so_far": self.spend.map(Spend::to_json),
            "dollars_per_percent": self.dollars_per_percent().map(round2),
            "left_dollars": self.left_dollars().map(round2),
            "left_output_tokens": self.left_output().map(f64::round),
            "left_output_tokens_by_model": self.left_output_by_model().into_iter()
                .map(|(label, tokens)| (label.to_string(), json!(tokens.round()))).collect::<serde_json::Map<_, _>>(),
            "need_tks": self.need_tks().map(round1),
            "now_tks": round1(self.now_tks),
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
        let account = tool.get("account").and_then(Value::as_str).map(str::to_string);
        let now_tks = tool.get("tks").and_then(Value::as_f64).unwrap_or(0.0);
        let switch_login = tool.get("switch").and_then(Value::as_bool).unwrap_or(false);
        let plan = tool.get("plan").and_then(Value::as_str).map(str::to_string);
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
                account: account.clone(),
                window: window_label(key),
                used,
                resets_at: resets_in.map(|s| now + Duration::seconds(s)),
                resets_in_secs: resets_in,
                projected,
                spend: w.get("spend").and_then(Spend::from_json),
                now_tks,
                switch_login,
                plan: plan.clone(),
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
            // `Claude (main), dev-a`: the others by host when they are remote columns.
            let others: Vec<&str> = group[1..].iter().map(|t| t.rsplit('@').next().unwrap_or(t)).collect();
            let name = if others.is_empty() { group[0].clone() } else { format!("{}, {}", group[0], others.join(", ")) };
            // The account's use is every column's: sum their spend and rate.
            windows
                .iter()
                .filter(|w| w.tool == group[0])
                .map(|w| {
                    let mut merged = Window { tool: name.clone(), ..w.clone() };
                    for other in windows.iter().filter(|o| group[1..].contains(&o.tool) && o.window == w.window) {
                        merged.now_tks += other.now_tks;
                        if let Some(more) = other.spend {
                            let mut sum = merged.spend.unwrap_or_default();
                            sum.add(&more);
                            merged.spend = Some(sum);
                        }
                    }
                    merged
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// One account's plan as the Quotas view shows it: its allowance (the longest window, week or
/// month) and, beside it, the current 5-hour window, which only says when to switch.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRow {
    pub plan: Window,
    pub five_hour: Option<Window>,
}

/// A 5-hour window from here means switching soon; at `FULL` it means switching now.
pub const SWITCH_SOON: f64 = 80.0;

/// One row per tool or account, soonest allowance reset first.
pub fn plan_rows(windows: &[Window]) -> Vec<PlanRow> {
    let mut tools: Vec<&str> = windows.iter().map(|w| w.tool.as_str()).collect();
    tools.sort();
    tools.dedup();
    let mut rows: Vec<PlanRow> = tools
        .into_iter()
        .filter_map(|tool| {
            let own: Vec<&Window> = windows.iter().filter(|w| w.tool == tool).collect();
            let plan = own
                .iter()
                .max_by(|a, b| window_secs_label(a.window).total_cmp(&window_secs_label(b.window)))?;
            let five_hour = own.iter().find(|w| w.window == "5h" && w.window != plan.window);
            Some(PlanRow { plan: (*plan).clone(), five_hour: five_hour.map(|w| (*w).clone()) })
        })
        .collect();
    rows.sort_by_key(|r| r.plan.resets_in_secs.unwrap_or(i64::MAX));
    rows
}

/// `plan_rows` in ranking order: most quota about to be wasted first.
pub fn plans_by_waste(windows: &[Window]) -> Vec<PlanRow> {
    let picks = where_to_spend(windows);
    let mut rows = plan_rows(windows);
    rows.sort_by_key(|r| picks.iter().position(|p| p.tool == r.plan.tool).unwrap_or(usize::MAX));
    rows
}

/// One ranked suggestion of where to spend tokens next.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub tool: String,
    pub account: Option<String>,
    pub available: bool,
    pub reason: String,
    /// Higher first.
    pub score: f64,
    /// The window the pick is about: the full one when unavailable, else the plan's longest.
    pub window: Option<Window>,
}

impl Pick {
    /// What to do, in a word or two, for the dashboard's at-a-glance list.
    pub fn action(&self) -> &'static str {
        let Some(w) = &self.window else { return "-" };
        if !self.available {
            "WAIT"
        } else if w.runs_out_in_secs().is_some() {
            "EASE OFF"
        } else if w.projected.is_none() {
            "USE"
        } else if w.expiring() >= 25.0 || (w.expiring() >= 5.0 && w.resets_in_secs.is_some_and(|s| s < 86_400)) {
            // A lot unused, or some with under a day to use it.
            "USE NOW"
        } else if w.expiring() >= 5.0 {
            "USE"
        } else {
            "ON TRACK"
        }
    }

    /// Why, as a short phrase.
    pub fn why(&self) -> String {
        let Some(w) = &self.window else { return "no data".into() };
        let secs = |s: Option<i64>| s.map(human_secs).unwrap_or_else(|| "?".into());
        if !self.available {
            return format!("{} full · back in {}", w.window, secs(w.resets_in_secs));
        }
        if w.switch_login {
            // Not signed in: it sits unused unless you switch to it.
            return format!("switch with /login · {:.0}% free", (100.0 - w.used).max(0.0));
        }
        match (w.projected, w.runs_out_in_secs()) {
            (_, Some(out)) => format!("runs out in {}", human_secs(out)),
            (Some(p), None) => format!("on pace for {:.0}%", p.min(999.0)),
            (None, None) => "too early to tell".into(),
        }
    }
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
                    account: own.iter().find_map(|w| w.account.clone()),
                    available: false,
                    reason: format!("{} window {:.0}% used; back in {back}", full.window, full.used),
                    score: -1.0 - full.resets_in_secs.unwrap_or(0) as f64 / 86_400.0,
                    window: Some((*full).clone()),
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
                    let mut reason = format!(
                        "{} window {:.0}% used, {pace}; about {:.0}% would go unused at its reset in {resets}",
                        w.window,
                        w.used,
                        w.expiring()
                    );
                    if let (Some(left), Some(need)) = (w.left_output(), w.need_tks()) {
                        reason.push_str(&format!(
                            ". Left: ~{} output tokens (${:.0}); to use it all: {need:.0} tk/s until the reset, now {:.0}",
                            human_tokens(left),
                            w.left_dollars().unwrap_or(0.0),
                            w.now_tks
                        ));
                        let models = w.left_output_by_model();
                        if !models.is_empty() {
                            let list: Vec<String> =
                                models.iter().map(|(m, t)| format!("{} {m}", human_tokens(*t))).collect();
                            reason.push_str(&format!(" (≈ {} if all on one model)", list.join(" / ")));
                        }
                    }
                    Pick {
                        tool: tool.to_string(),
                        account: w.account.clone(),
                        available: true,
                        reason,
                        score,
                        window: Some((*w).clone()),
                    }
                }
                None => Pick { tool: tool.to_string(), account: None, available: true, reason: "no window data".into(), score: 0.0, window: None },
            }
        })
        .collect();
    picks.sort_by(|a, b| b.score.total_cmp(&a.score));
    picks
}

/// The same shape as `status.json`'s `tools`, built straight from each column's limits, for the
/// dashboard's own Quotas view (no file needed).
pub fn status_for_tools(stats: &[&crate::types::AgenticCodingToolStats]) -> Value {
    let now = Utc::now();
    let mut tools: Vec<Value> = stats.iter().filter_map(|s| tool_json(s, now)).collect();
    tools.extend(other_login_tools(stats, now));
    json!({"updated": now.timestamp_millis() as f64 / 1000.0, "tools": tools})
}

/// Codex's ChatGPT plan from its logs (`chatgpt·plus` -> `Plus`).
fn codex_plan(name: &str) -> Option<String> {
    let account = crate::plan_limits::account_for_analyzer(name).filter(|_| name == "Codex CLI")?;
    let plan = account.rsplit('·').next()?;
    let mut chars = plan.chars();
    Some(chars.next()?.to_uppercase().chain(chars).collect())
}

/// `a@virti.us (Virtius)` -> `a@Virtius`; `a@virti.us (personal)` -> `a@virti.us`.
pub fn short_login(account: &str) -> String {
    let (email, org) = account.split_once(" (").map_or((account, ""), |(e, o)| (e, o.trim_end_matches(')')));
    let name = email.split('@').next().unwrap_or(email);
    // A personal login by its whole email (`a` alone is too short to recognise); a team seat by
    // the name and the team, which is what tells seats on one email apart.
    if org == "personal" { email.to_string() } else if org.is_empty() { name.to_string() } else { format!("{name}@{org}") }
}

/// The other logins on this machine's Claude accounts, as tools in the status shape: each its
/// own allowance, one `/login` away, with its own week's spend once it has run (BZL-283).
pub fn other_login_tools(stats: &[&crate::types::AgenticCodingToolStats], now: DateTime<Utc>) -> Vec<Value> {
    let mut out = Vec::new();
    for tool in stats.iter().filter(|t| t.analyzer_name.starts_with("Claude (") && !t.analyzer_name.contains('@')) {
        for other in crate::plan_limits::other_logins(&tool.analyzer_name) {
            let unknown_is_mine = top_login(&tool.analyzer_name).as_deref() == Some(other.account.as_str());
            let spend = other.seven_day.resets_in_secs.and_then(|left| {
                let start = now + Duration::seconds(left) - Duration::days(7);
                let spend = login_spend(tool, start, &other.account, unknown_is_mine);
                (spend.cost > 0.0).then(|| spend.to_json())
            });
            out.push(json!({
                "name": format!("{} · {}", tool.analyzer_name, short_login(&other.account)),
                "account": other.account,
                "plan": other.plan,
                "tks": 0.0,
                "cost_today": 0.0,
                "switch": true,
                "limits": {"five_hour": Value::Null, "monthly": Value::Null,
                           "seven_day": {"used": (other.seven_day.used_percentage * 10.0).round() / 10.0,
                                         "resets_in": other.seven_day.resets_in_secs, "spend": spend}},
            }));
        }
    }
    out
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
        "plans": plans_by_waste(&all).iter().map(|r| json!({
            "plan": r.plan.to_json(),
            "five_hour_now": r.five_hour.as_ref().map(Window::to_json),
        })).collect::<Vec<_>>(),
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
            "tool": p.tool, "account": p.account, "available": p.available, "reason": p.reason, "score": round1(p.score),
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

    #[test]
    fn a_logins_spend_is_its_own_slices_from_the_windows_hour() {
        let slice = |hour: u8, account: &str, cost: f64| crate::types::Slice {
            hour, account: account.into(), model: "m".into(), replies: 1, output_tokens: 10, cost, ..Default::default()
        };
        let mut day = crate::types::DailyStats::default();
        day.slices = vec![slice(8, "a (personal)", 1.0), slice(12, "a (personal)", 2.0), slice(12, "s (Virtius)", 4.0), slice(13, "", 8.0)];
        let mut stats = crate::types::AgenticCodingToolStats {
            daily_stats: Default::default(), num_conversations: 0, messages: Vec::new(), analyzer_name: "Claude (v)".into(),
        };
        stats.daily_stats.insert("2026-10-23".into(), day);
        let start = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap().and_hms_opt(10, 0, 0).unwrap()
            .and_local_timezone(Local).unwrap().with_timezone(&Utc);
        // From 10:00: the 08:00 slice is before the window.
        assert_eq!(login_spend(&stats, start, "a (personal)", false).cost, 2.0);
        // The signed-in login also takes usage from before logins were recorded.
        assert_eq!(login_spend(&stats, start, "a (personal)", true).cost, 10.0);
        assert_eq!(login_spend(&stats, start, "s (Virtius)", false).cost, 4.0);
        assert!(has_logins(&stats));
    }

    #[test]
    fn a_fresh_signed_out_login_ranks_as_a_whole_week_to_use() {
        // Two days into a fresh week nobody has touched: the whole allowance would go unused.
        let status = serde_json::json!({"tools": [{
            "name": "Claude (v) · a@Virtius", "account": "a@virti.us (Virtius)", "tks": 0.0, "switch": true,
            "limits": {"five_hour": null, "monthly": null, "seven_day": {"used": 0.0, "resets_in": 5 * 86_400}}
        }]});
        let windows = windows(&status, Utc::now());
        assert!(windows[0].switch_login);
        let picks = where_to_spend(&windows);
        assert_eq!(picks[0].action(), "USE NOW");
        assert_eq!(picks[0].why(), "switch with /login · 100% free");
    }

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
        assert!(all.iter().any(|w| w.tool == "Claude (main), dev-b"), "{all:?}");
        assert_eq!(all.iter().filter(|w| w.tool.contains("dev-b")).count(), 1, "one row for the shared account");
    }

    #[test]
    fn spend_where_quota_is_about_to_expire_and_skip_full_ones() {
        let now = DateTime::from_timestamp(1000, 0).unwrap();
        let picks = where_to_spend(&windows(&status(), now));
        assert_eq!(picks[0].tool, "Claude (v)", "{picks:?}");
        let last = picks.last().unwrap();
        assert_eq!((last.tool.as_str(), last.available), ("Claude@dev-a", false));
        assert!(last.reason.contains("back in 30m"));
        let main = picks.iter().find(|p| p.tool == "Claude (main), dev-b").unwrap();
        assert!(main.score < picks[0].score, "on pace to use its week: little expires");
    }

    #[test]
    fn one_row_per_plan_with_the_five_hour_window_beside_it() {
        let now = DateTime::from_timestamp(1000, 0).unwrap();
        let rows = plan_rows(&windows(&status(), now));
        let v = rows.iter().find(|r| r.plan.tool == "Claude (v)").unwrap();
        assert_eq!(v.plan.window, "week");
        assert_eq!(v.five_hour.as_ref().map(|w| w.used), Some(10.0));
        // Only a 5-hour window: that is the plan.
        let dev_a = rows.iter().find(|r| r.plan.tool == "Claude@dev-a").unwrap();
        assert_eq!((dev_a.plan.window, dev_a.five_hour.is_none()), ("5h", true));
        assert_eq!(rows.iter().filter(|r| r.plan.tool == "Claude (v)").count(), 1);
    }

    #[test]
    fn measured_spend_turns_the_remaining_share_into_tokens_and_a_pace() {
        // 70% of the week cost $381 for 3.77M output tokens; 9.2 hours left.
        let w = Window {
            tool: "Claude (main)".into(), account: None, window: "week", used: 70.0, resets_at: None,
            resets_in_secs: Some(33_120), projected: Some(74.0), now_tks: 140.0, switch_login: false, plan: None,
            spend: Some(Spend { input: 9_764.0, output: 3_770_000.0, cache_write: 17_259_201.0,
                                cache_read: 1_094_254_105.0, cost: 381.0 }),
        };
        assert!((w.dollars_per_percent().unwrap() - 381.0 / 70.0).abs() < 1e-9);
        assert!((w.left_dollars().unwrap() - 381.0 * 30.0 / 70.0).abs() < 1e-6);
        let left = w.left_output().unwrap();
        assert!((left - 3_770_000.0 * 30.0 / 70.0).abs() < 1.0, "{left}");
        assert!((w.need_tks().unwrap() - left / 33_120.0).abs() < 1e-9);
        assert_eq!(w.locked_out_secs(), None, "on pace for 74%: it does not run out");
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
            tool: "t".into(), account: None, window: "week", used: 70.0, resets_at: None,
            resets_in_secs: Some(302_400), projected: Some(140.0), spend: None, now_tks: 0.0, switch_login: false, plan: None,
        };
        assert_eq!(w.runs_out_in_secs(), Some(129_600));
    }
}

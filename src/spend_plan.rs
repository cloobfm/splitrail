//! The token stack: every plan's allowance as a pile with an expiry, drained earliest first.
//!
//! `where_to_spend` ranks plans; this turns the ranking into a plan with times in it:
//! - **Driver**: the Claude account a lead agent should work in now, the earliest-expiring one
//!   that is usable and not on pace to run out, and **when to switch** off it (its 5-hour window
//!   fills at the current rate, its week runs out, or the next account's last moment to start).
//! - **Drains**: the other piles a lead agent should hand subagent work to, earliest expiry
//!   first, each with the headless command that runs it and the work it suits.
//! - **Avoid**: piles on pace to run out before their reset; **lost**: what cannot be used by the
//!   reset whatever happens.
//!
//! [`brief`] renders it as a short prompt for a lead agent (`splitrail-dashboard brief`, the MCP
//! tool `spend_brief`, and the dispatcher, which hands each lane its own).

use chrono::{DateTime, Duration, Local, Utc};
use serde_json::{Value, json};

use crate::advisor::{Pick, Reach, Window, human_secs, human_tokens, reach, where_to_spend};

/// Below this percent left, a pile is spent.
const SPENT: f64 = 1.0;

/// What one pile is for in the plan.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    /// Work in it now.
    Driver,
    /// Work in it when the driver is switched off.
    Next,
    /// Hand subagent work to it.
    Drain,
    /// On pace to run out before its reset: send no more to it.
    Avoid,
    /// Full until its window resets.
    Full,
    /// Nothing worth planning: spent, or unknown.
    Idle,
}

#[derive(Debug, Clone)]
pub struct Pile {
    pub tool: String,
    pub account: Option<String>,
    pub role: Role,
    /// When what is left stops being usable in full: the last moment to start, for a plan
    /// gated by 5-hour windows; the reset otherwise.
    pub deadline: Option<DateTime<Utc>>,
    /// The deadline is the last moment to start (5-hour caps), not the reset.
    pub start_by: bool,
    pub resets_at: Option<DateTime<Utc>>,
    /// Percent of the plan left.
    pub left: f64,
    pub left_output: Option<f64>,
    pub left_dollars: Option<f64>,
    /// Percent that will go unused even at full use (5-hour caps).
    pub lost: f64,
    /// For a Full pile: when it is usable again.
    pub back_at: Option<DateTime<Utc>>,
    pub switch_login: bool,
    pub now_tks: f64,
}

#[derive(Debug, Clone)]
pub struct Switch {
    pub at: DateTime<Utc>,
    pub why: String,
}

#[derive(Debug, Clone)]
pub struct SpendPlan {
    pub now: DateTime<Utc>,
    /// Earliest deadline first.
    pub piles: Vec<Pile>,
    pub switch: Option<Switch>,
}

impl SpendPlan {
    pub fn driver(&self) -> Option<&Pile> {
        self.piles.iter().find(|p| p.role == Role::Driver)
    }
    pub fn next(&self) -> Option<&Pile> {
        self.piles.iter().find(|p| p.role == Role::Next)
    }
    /// Piles a subagent can run on now: not another login waiting for a `/login`.
    pub fn drains(&self) -> impl Iterator<Item = &Pile> {
        self.piles.iter().filter(|p| p.role == Role::Drain && !p.switch_login)
    }
    /// Other logins with quota, usable only by switching to them.
    pub fn waiting_for_login(&self) -> impl Iterator<Item = &Pile> {
        self.piles.iter().filter(|p| p.role == Role::Drain && p.switch_login)
    }
    pub fn avoid(&self) -> impl Iterator<Item = &Pile> {
        self.piles.iter().filter(|p| p.role == Role::Avoid)
    }
}

/// A Claude account a lead agent can work in. Columns on other machines (`Claude@dev-d`) are
/// those machines' own lanes.
fn can_drive(tool: &str) -> bool {
    tool.starts_with("Claude") && !tool.starts_with("Claude@")
}

/// The headless command that runs a subagent on this pile, and the work it suits.
pub fn drain_command(tool: &str) -> Option<(String, &'static str)> {
    let t = tool;
    Some(if t.starts_with("Antigravity") {
        ("agy-run \"<task>\"".into(), "reading, search, summaries")
    } else if t.starts_with("Codex") {
        ("codex exec \"<task>\"".into(), "tests, refactors, reviews")
    } else if t.starts_with("Cursor") {
        ("cursor-run --trust --workspace <dir> \"<task>\"".into(), "edits and tests")
    } else if t.starts_with("Droid") {
        ("droid exec \"<task>\"".into(), "edits and tests")
    } else if t.starts_with("Warp") {
        (
            "warp-run --cwd <dir> --name <name> --output-format text --prompt \"<task>\"".into(),
            "shell and ops tasks",
        )
    } else if t.starts_with("Kiro") {
        ("kiro-cli chat --no-interactive \"<task>\"".into(), "small, self-contained tasks")
    } else if t.starts_with("Claude (v)") {
        ("CLAUDE_CONFIG_DIR=~/.claude-v claude -p \"<task>\"".into(), "anything; same models as the driver")
    } else if t.starts_with("Claude (main)") || t.starts_with("Claude Code") {
        ("claude -p \"<task>\"".into(), "anything; same models as the driver")
    } else {
        return None;
    })
}

/// The pile one pick stands for.
fn pile(pick: &Pick) -> Pile {
    let w = pick.window.as_ref();
    let reach: Option<Reach> = pick.reach;
    let left = w.map_or(0.0, |w| (100.0 - w.used).max(0.0));
    let (deadline, start_by) = match reach {
        Some(r) if r.lost < SPENT && r.left_to_use() >= SPENT => {
            (Some(r.start_by.min(w.and_then(|w| w.resets_at).unwrap_or(r.start_by))), true)
        }
        _ => (w.and_then(|w| w.resets_at), false),
    };
    let runs_out = w.is_some_and(|w| w.runs_out_in_secs().is_some());
    let role = if !pick.available {
        Role::Full
    } else if w.is_none() || left < SPENT {
        Role::Idle
    } else if runs_out {
        Role::Avoid
    } else {
        Role::Drain
    };
    let back_at = (!pick.available).then(|| w.and_then(|w| w.resets_at)).flatten();
    Pile {
        tool: pick.tool.clone(),
        account: pick.account.clone(),
        role,
        deadline,
        start_by,
        resets_at: w.and_then(|w| w.resets_at),
        left,
        left_output: w.and_then(Window::left_output),
        left_dollars: w.and_then(Window::left_dollars),
        lost: reach.map_or(0.0, |r| r.lost),
        back_at,
        switch_login: w.is_some_and(|w| w.switch_login),
        now_tks: w.map_or(0.0, |w| w.now_tks),
    }
}

/// When the 5-hour window fills at the current rate, from how much of the week a token is.
fn five_hour_fills_at(week: &Window, five: &Window, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let r = reach(week, Some(five))?;
    let spend = week.spend?;
    if week.now_tks <= 0.0 || spend.output <= 0.0 || week.used <= 0.0 {
        return None;
    }
    let week_pct_per_token = week.used / spend.output;
    let pct_per_sec = week.now_tks * week_pct_per_token * 100.0 / r.per_window;
    let secs = (100.0 - five.used).max(0.0) / pct_per_sec;
    let ends = five.resets_in_secs?;
    (secs < ends as f64).then(|| now + Duration::seconds(secs as i64))
}

pub fn plan(windows: &[Window], now: DateTime<Utc>) -> SpendPlan {
    let picks = where_to_spend(windows);
    let five_of = |tool: &str| windows.iter().find(|w| w.tool == tool && w.window == "5h");
    let mut piles: Vec<Pile> = picks.iter().map(pile).collect();
    piles.sort_by_key(|p| p.deadline.unwrap_or(DateTime::<Utc>::MAX_UTC));

    // Driver: the earliest-expiring Claude account usable now; a signed-in one before one that
    // needs a /login at the same deadline.
    let driver = piles
        .iter()
        .enumerate()
        .filter(|(_, p)| p.role == Role::Drain && can_drive(&p.tool))
        .min_by_key(|(_, p)| (p.deadline.unwrap_or(DateTime::<Utc>::MAX_UTC), p.switch_login))
        .map(|(i, _)| i);
    let mut switch = None;
    if let Some(d) = driver {
        piles[d].role = Role::Driver;
        let next = piles
            .iter()
            .enumerate()
            .find(|(i, p)| *i != d && p.role == Role::Drain && can_drive(&p.tool))
            .map(|(i, _)| i);
        if let Some(n) = next {
            piles[n].role = Role::Next;
        }
        // Switch at the first of: the driver's 5-hour window filling, its plan running out at
        // this rate, the next account's last moment to start, the driver's reset.
        let tool = piles[d].tool.clone();
        let week = windows.iter().filter(|w| w.tool == tool).max_by(|a, b| {
            crate::advisor::window_secs_label(a.window).total_cmp(&crate::advisor::window_secs_label(b.window))
        });
        let mut candidates: Vec<(DateTime<Utc>, String)> = Vec::new();
        if let (Some(week), Some(five)) = (week, five_of(&tool))
            && let Some(at) = five_hour_fills_at(week, five, now)
        {
            candidates.push((at, format!("its 5-hour window fills at {:.0} tk/s", week.now_tks)));
        }
        if let Some(week) = week
            && let (Some(left), true) = (week.left_output(), week.now_tks > 0.0)
        {
            let at = now + Duration::seconds((left / week.now_tks) as i64);
            candidates.push((at, "its plan runs out at this rate".into()));
        }
        if let Some(n) = next
            && let Some(at) = piles[n].deadline
        {
            let why = if piles[n].start_by {
                format!("{} must start by then to be used in full", piles[n].tool)
            } else {
                format!("{} resets then", piles[n].tool)
            };
            candidates.push((at, why));
        }
        if let Some(at) = piles[d].resets_at {
            candidates.push((at, "it resets".into()));
        }
        switch = candidates
            .into_iter()
            .filter(|(at, _)| *at > now)
            .min_by_key(|(at, _)| *at)
            .map(|(at, why)| Switch { at, why });
    }
    SpendPlan { now, piles, switch }
}

fn local(t: DateTime<Utc>) -> String {
    let t = t.with_timezone(&Local);
    if t.date_naive() == Local::now().date_naive() {
        t.format("%H:%M").to_string()
    } else {
        t.format("%a %H:%M").to_string()
    }
}

fn amount(p: &Pile) -> String {
    match (p.left_output, p.left_dollars) {
        (Some(t), Some(d)) => format!("{} left (${d:.0})", human_tokens(t)),
        _ => format!("{:.0}% left", p.left),
    }
}

fn until(p: &Pile, now: DateTime<Utc>) -> String {
    let what = if p.start_by { "start by" } else { "gone" };
    match p.deadline {
        Some(d) => format!("{what} {} (in {})", local(d), human_secs((d - now).num_seconds())),
        None => "no reset known".into(),
    }
}

/// The plan as a short prompt for a lead agent.
pub fn brief(plan: &SpendPlan) -> String {
    let now = plan.now;
    let mut out = String::new();
    let horizon = plan.switch.as_ref().map(|s| local(s.at)).unwrap_or_else(|| "further notice".into());
    out.push_str(&format!("Token plan until {horizon} (Splitrail, {}):\n", local(now)));
    match plan.driver() {
        Some(d) => {
            let login = if d.switch_login { " (switch to it with /login)" } else { "" };
            out.push_str(&format!("- Work in {}{login}: {}, {}.\n", d.tool, amount(d), until(d, now)));
            if let Some(s) = &plan.switch {
                let then = plan.next().map(|n| format!("; then move to {}", n.tool)).unwrap_or_default();
                out.push_str(&format!("- Switch at {}: {}{then}.\n", local(s.at), s.why));
            }
        }
        None => out.push_str("- No Claude account has quota to plan around right now.\n"),
    }
    let drains: Vec<&Pile> = plan.drains().filter(|p| drain_command(&p.tool).is_some()).collect();
    if !drains.is_empty() {
        out.push_str("- Hand subagent work out, earliest-expiring first:\n");
        for (i, p) in drains.iter().enumerate() {
            let (command, suits) = drain_command(&p.tool).unwrap_or_default();
            out.push_str(&format!(
                "  {}. {}: {}, {}. Good for {suits}: {command}\n",
                i + 1,
                p.tool,
                amount(p),
                until(p, now)
            ));
        }
    }
    let waiting: Vec<String> = plan.waiting_for_login().map(|p| format!("{} ({})", p.tool, until(p, now))).collect();
    if !waiting.is_empty() {
        out.push_str(&format!("- Also unused, but only after a /login to it: {}.\n", waiting.join(", ")));
    }
    let avoid: Vec<String> = plan.avoid().map(|p| p.tool.clone()).collect();
    if !avoid.is_empty() {
        out.push_str(&format!("- Don't send work to {}: on pace to run out before its reset.\n", avoid.join(", ")));
    }
    let lost: Vec<String> = plan
        .piles
        .iter()
        .filter(|p| p.lost >= SPENT)
        .map(|p| format!("{} ~{:.0}%", p.tool, p.lost))
        .collect();
    if !lost.is_empty() {
        out.push_str(&format!("- Can't all be used by the reset whatever happens: {}.\n", lost.join(", ")));
    }
    out.push_str("- Keep design and hard calls in your own context; hand out work a weaker model can check.\n");
    out.push_str(&format!(
        "Re-check with the splitrail spend_brief tool at {horizon}, or when a handoff fails.\n"
    ));
    out
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::Driver => "driver",
        Role::Next => "next",
        Role::Drain => "drain",
        Role::Avoid => "avoid",
        Role::Full => "full",
        Role::Idle => "idle",
    }
}

pub fn to_json(plan: &SpendPlan) -> Value {
    let pile_json = |p: &Pile| {
        let cmd = drain_command(&p.tool);
        json!({
            "tool": p.tool,
            "account": p.account,
            "role": role_name(&p.role),
            "deadline": p.deadline.map(|t| t.with_timezone(&Local).to_rfc3339()),
            "deadline_in": p.deadline.map(|t| human_secs((t - plan.now).num_seconds())),
            "deadline_is_start_by": p.start_by,
            "left_percent": (p.left * 10.0).round() / 10.0,
            "left_output_tokens": p.left_output.map(f64::round),
            "left_dollars": p.left_dollars.map(|d| (d * 100.0).round() / 100.0),
            "lost_percent": (p.lost * 10.0).round() / 10.0,
            "back_at": p.back_at.map(|t| t.with_timezone(&Local).to_rfc3339()),
            "needs_login_switch": p.switch_login,
            "now_tks": (p.now_tks * 10.0).round() / 10.0,
            "command": cmd.as_ref().map(|c| c.0.clone()),
            "suits": cmd.map(|c| c.1),
        })
    };
    json!({
        "driver": plan.driver().map(pile_json),
        "switch_at": plan.switch.as_ref().map(|s| s.at.with_timezone(&Local).to_rfc3339()),
        "switch_in": plan.switch.as_ref().map(|s| human_secs((s.at - plan.now).num_seconds())),
        "switch_why": plan.switch.as_ref().map(|s| s.why.clone()),
        "next_driver": plan.next().map(pile_json),
        "delegate_to": plan.drains().filter(|p| drain_command(&p.tool).is_some()).map(pile_json).collect::<Vec<_>>(),
        "avoid": plan.avoid().map(pile_json).collect::<Vec<_>>(),
        "stack": plan.piles.iter().map(pile_json).collect::<Vec<_>>(),
        "brief": brief(plan),
    })
}

/// `spend_brief`: the plan from the dashboard's status file.
pub fn answer(status: &Value, now: DateTime<Utc>) -> Value {
    to_json(&plan(&crate::advisor::windows(status, now), now))
}

/// Announce a new driver, and a switch coming within ten minutes, as macOS notifications.
/// Called from the status-file loop; checks at most once a minute.
pub fn watch(status: &Value) {
    use std::sync::Mutex;
    struct Seen {
        checked: Option<std::time::Instant>,
        driver: Option<String>,
        warned: Option<DateTime<Utc>>,
    }
    static SEEN: Mutex<Seen> = Mutex::new(Seen { checked: None, driver: None, warned: None });
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.checked.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(60)) {
        return;
    }
    seen.checked = Some(std::time::Instant::now());
    let now = Utc::now();
    let plan = plan(&crate::advisor::windows(status, now), now);
    let driver = plan.driver().map(|d| d.tool.clone());
    if let (Some(old), Some(new)) = (&seen.driver, &driver)
        && old != new
    {
        crate::records::notify_text("🔀 Switch accounts", &format!("Work in {new} now (was {old})."));
    }
    if let (Some(s), Some(next)) = (&plan.switch, plan.next())
        && s.at - now <= Duration::minutes(10)
        && seen.warned.is_none_or(|w| (w - s.at).num_minutes().abs() > 30)
    {
        crate::records::notify_text(
            "⏭ Switch soon",
            &format!("At {}: move to {} ({}).", local(s.at), next.tool, s.why),
        );
        seen.warned = Some(s.at);
    }
    seen.driver = driver;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(tool: &str, window: &'static str, used: f64, resets_in_h: i64, now: DateTime<Utc>) -> Window {
        let secs = resets_in_h * 3600;
        let length = crate::advisor::window_secs_label(window);
        Window {
            tool: tool.into(),
            account: None,
            window,
            used,
            resets_at: Some(now + Duration::seconds(secs)),
            resets_in_secs: Some(secs),
            projected: Some(used * length / (length - secs as f64)),
            spend: None,
            now_tks: 0.0,
            switch_login: false,
            plan: Some("Max 20x".into()),
            week_per_5h: Some(20.0),
        }
    }

    #[test]
    fn the_earliest_expiring_claude_account_drives_and_the_next_follows() {
        let now = Utc::now();
        let windows = vec![
            window("Claude (main)", "week", 60.0, 20, now),
            window("Claude (v)", "week", 30.0, 40, now),
            window("Antigravity", "week", 50.0, 4, now),
            window("Codex CLI", "week", 5.0, 120, now),
        ];
        let plan = plan(&windows, now);
        assert_eq!(plan.driver().unwrap().tool, "Claude (main)");
        assert_eq!(plan.next().unwrap().tool, "Claude (v)");
        let drains: Vec<&str> = plan.drains().map(|p| p.tool.as_str()).collect();
        assert_eq!(drains, ["Antigravity", "Codex CLI"], "earliest expiry first");
        let brief = brief(&plan);
        assert!(brief.contains("Work in Claude (main)"), "{brief}");
        assert!(brief.contains("agy-run"), "{brief}");
        assert!(plan.switch.is_some(), "at the latest when the next must start, or the reset");
    }

    #[test]
    fn a_plan_on_pace_to_run_out_is_avoided_and_never_drives() {
        let now = Utc::now();
        // 90% used two days into the week: runs out well before the reset.
        let windows = vec![
            window("Claude (main)", "week", 90.0, 120, now),
            window("Claude (v)", "week", 10.0, 100, now),
        ];
        let plan = plan(&windows, now);
        assert_eq!(plan.driver().unwrap().tool, "Claude (v)");
        assert_eq!(plan.avoid().next().unwrap().tool, "Claude (main)");
    }

    #[test]
    fn other_machines_claude_columns_are_not_drivers() {
        assert!(!can_drive("Claude@dev-d, dev-b"));
        assert!(can_drive("Claude (main), dev-c, dev-a"));
    }
}

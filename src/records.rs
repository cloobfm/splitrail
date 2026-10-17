//! Personal bests across every tool and machine: peak tk/s, the biggest day, week and month, the
//! longest run of days on the monthly goal, and the most sessions running at once. Kept in
//! `~/.config/splitrail/records.json`, which only ever goes up, so a record outlives the
//! transcripts it came from (Claude deletes old ones). A record beaten while the dashboard runs
//! raises a macOS notification.

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::types::{AgenticCodingToolStats, MessageRole};

/// Sessions with a reply in the same slot count as running at once (Ollama requests aside).
const PARALLEL_SLOT_SECS: i64 = 5 * 60;
/// Recomputing sorts every message, so at most this often.
const MIN_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    PeakTks15m,
    PeakTks1h,
    DayOutput,
    DayCost,
    GoalStreak,
    WeekOutput,
    MonthOutput,
    ParallelSessions,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::PeakTks15m,
        Kind::PeakTks1h,
        Kind::DayOutput,
        Kind::DayCost,
        Kind::GoalStreak,
        Kind::WeekOutput,
        Kind::MonthOutput,
        Kind::ParallelSessions,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::PeakTks15m => "Peak tk/s (15 min)",
            Kind::PeakTks1h => "Peak tk/s (1 hour)",
            Kind::DayOutput => "Biggest day (output)",
            Kind::DayCost => "Biggest day ($)",
            Kind::GoalStreak => "Goal streak (days)",
            Kind::WeekOutput => "Best week (output)",
            Kind::MonthOutput => "Best month (output)",
            Kind::ParallelSessions => "Most sessions at once",
        }
    }

    /// `value` in this record's unit, short.
    pub fn format(self, value: f64) -> String {
        match self {
            Kind::PeakTks15m | Kind::PeakTks1h => format!("{value:.0} tk/s"),
            Kind::DayCost => format!("${value:.0}"),
            Kind::GoalStreak => format!("{value:.0} d"),
            Kind::ParallelSessions => format!("{value:.0}"),
            Kind::DayOutput | Kind::WeekOutput | Kind::MonthOutput => crate::advisor::human_tokens(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub value: f64,
    /// When it was set: a date, week, month or time, as fits the record.
    pub when: String,
}

/// The records, and where each stands now (today, this week, the last 15 minutes...).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    pub best: BTreeMap<Kind, Record>,
    pub now: BTreeMap<Kind, f64>,
}

fn week_label(date: NaiveDate) -> String {
    let monday = date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
    format!("week of {}", monday.format("%b %-d %Y"))
}

/// The best sum of `values` over any `window_secs`, as a rate, and when that window ended.
fn peak_rate(points: &[(i64, f64)], window_secs: i64) -> Option<(f64, i64)> {
    let (mut start, mut sum, mut best) = (0, 0.0, None::<(f64, i64)>);
    for &(at, value) in points {
        sum += value;
        while points[start].0 <= at - window_secs {
            sum -= points[start].1;
            start += 1;
        }
        if best.is_none_or(|(b, _)| sum > b) {
            best = Some((sum, at));
        }
    }
    best.map(|(sum, at)| (sum / window_secs as f64, at))
}

/// Records as the data stands, with nothing saved: `goal_tks` is the monthly goal a streak day
/// must average.
pub fn compute(stats: &[&AgenticCodingToolStats], goal_tks: f64, now: DateTime<Utc>) -> Board {
    let mut board = Board::default();
    let set = |board: &mut Board, kind: Kind, value: f64, when: String| {
        if value > 0.0 && board.best.get(&kind).is_none_or(|r| value > r.value) {
            board.best.insert(kind, Record { value, when });
        }
    };

    // Days, from the daily totals (which remote columns have too).
    let mut days: BTreeMap<NaiveDate, (f64, f64)> = BTreeMap::new();
    for tool in stats {
        for (date, day) in &tool.daily_stats {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                let entry = days.entry(date).or_default();
                entry.0 += day.stats.output_tokens as f64;
                entry.1 += day.stats.cost;
            }
        }
    }
    let today = now.with_timezone(&Local).date_naive();
    let mut weeks: BTreeMap<String, f64> = BTreeMap::new();
    let mut months: BTreeMap<String, f64> = BTreeMap::new();
    let (mut run, mut prev) = (0u32, None::<NaiveDate>);
    let mut current_streak = 0u32;
    for (&date, &(output, cost)) in &days {
        let pretty = date.format("%a %b %-d %Y").to_string();
        set(&mut board, Kind::DayOutput, output, pretty.clone());
        set(&mut board, Kind::DayCost, cost, pretty);
        *weeks.entry(week_label(date)).or_default() += output;
        *months.entry(date.format("%B %Y").to_string()).or_default() += output;
        if output >= goal_tks * 86_400.0 {
            run = if prev.is_some_and(|p| date - p == chrono::Duration::days(1)) { run + 1 } else { 1 };
            prev = Some(date);
            set(&mut board, Kind::GoalStreak, run as f64, format!("to {}", date.format("%b %-d %Y")));
        } else {
            run = 0;
            prev = None;
        }
    }
    // A streak is still alive if it reached yesterday, whether or not today has made it yet.
    if let Some(last) = prev
        && (last == today || last + chrono::Duration::days(1) == today)
    {
        current_streak = run;
    }
    for (week, output) in weeks {
        set(&mut board, Kind::WeekOutput, output, week);
    }
    for (month, output) in months {
        set(&mut board, Kind::MonthOutput, output, month);
    }

    // Replies, for rates and parallel sessions.
    let mut points: Vec<(i64, f64)> = Vec::new();
    let mut slots: HashMap<i64, HashSet<(usize, &str)>> = HashMap::new();
    for (i, tool) in stats.iter().enumerate() {
        for m in tool.messages.iter().filter(|m| m.role == MessageRole::Assistant) {
            let at = m.date.timestamp();
            if m.stats.output_tokens > 0 {
                points.push((at, m.stats.output_tokens as f64));
            }
            // Each Ollama request is its own "conversation" (a decision-model eval fires hundreds),
            // so it is not a session running alongside the others.
            if m.application != crate::types::Application::Ollama {
                slots.entry(at.div_euclid(PARALLEL_SLOT_SECS)).or_default().insert((i, m.conversation_hash.as_str()));
            }
        }
    }
    points.sort_by_key(|p| p.0);
    let time = |at: i64| {
        DateTime::from_timestamp(at, 0)
            .map(|t| t.with_timezone(&Local).format("%a %b %-d %Y %H:%M").to_string())
            .unwrap_or_default()
    };
    for (kind, window) in [(Kind::PeakTks15m, 15 * 60), (Kind::PeakTks1h, 3600)] {
        if let Some((rate, at)) = peak_rate(&points, window) {
            set(&mut board, kind, rate, time(at));
        }
        let since = now.timestamp() - window;
        let recent: f64 = points.iter().rev().take_while(|p| p.0 > since).map(|p| p.1).sum();
        board.now.insert(kind, recent / window as f64);
    }
    if let Some((&slot, sessions)) = slots.iter().max_by_key(|(slot, s)| (s.len(), std::cmp::Reverse(**slot))) {
        // Which tools they were, busiest first: "Sat Oct 17 2026 08:05 · Claude (main) 9, Codex CLI 3".
        let mut by_tool: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, _) in sessions {
            *by_tool.entry(stats[*i].analyzer_name.as_str()).or_default() += 1;
        }
        let mut by_tool: Vec<_> = by_tool.into_iter().collect();
        by_tool.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), *name));
        let tools: Vec<String> = by_tool.iter().take(3).map(|(name, n)| format!("{name} {n}")).collect();
        let when = format!("{} · {}", time(slot * PARALLEL_SLOT_SECS), tools.join(", "));
        set(&mut board, Kind::ParallelSessions, sessions.len() as f64, when);
    }
    let slot_now = now.timestamp().div_euclid(PARALLEL_SLOT_SECS);
    let parallel_now = [slot_now, slot_now - 1]
        .iter()
        .filter_map(|s| slots.get(s))
        .flatten()
        .collect::<HashSet<_>>()
        .len();

    let (today_output, today_cost) = days.get(&today).copied().unwrap_or_default();
    board.now.insert(Kind::DayOutput, today_output);
    board.now.insert(Kind::DayCost, today_cost);
    board.now.insert(Kind::GoalStreak, current_streak as f64);
    let this_week = week_label(today);
    board.now.insert(
        Kind::WeekOutput,
        days.iter().filter(|(d, _)| week_label(**d) == this_week).map(|(_, v)| v.0).sum(),
    );
    board.now.insert(
        Kind::MonthOutput,
        days.iter().filter(|(d, _)| d.year() == today.year() && d.month() == today.month()).map(|(_, v)| v.0).sum(),
    );
    board.now.insert(Kind::ParallelSessions, parallel_now as f64);
    board
}

/// `computed` merged into `saved`, keeping the higher of each, with the records newly beaten
/// (as `(kind, new, old)`). A record never seen before is not "beaten".
pub fn ratchet(
    saved: &BTreeMap<Kind, Record>,
    computed: &BTreeMap<Kind, Record>,
) -> (BTreeMap<Kind, Record>, Vec<(Kind, Record, Record)>) {
    let mut merged = saved.clone();
    let mut beaten = Vec::new();
    for (&kind, record) in computed {
        match saved.get(&kind) {
            Some(old) if record.value > old.value + 1e-9 => {
                beaten.push((kind, record.clone(), old.clone()));
                merged.insert(kind, record.clone());
            }
            Some(_) => {}
            None => {
                merged.insert(kind, record.clone());
            }
        }
    }
    (merged, beaten)
}

pub fn path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/records.json"))
}

fn load() -> Option<BTreeMap<Kind, Record>> {
    serde_json::from_str(&std::fs::read_to_string(path()?).ok()?).ok()
}

fn save(records: &BTreeMap<Kind, Record>) -> std::io::Result<()> {
    let Some(path) = path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(records)?)?;
    std::fs::rename(tmp, path)
}

/// How a new record reaches the desktop.
///
/// Implementations *build* the command rather than running it, so the platform logic is unit
/// testable without spawning anything (BZL-12).
trait Notifier {
    /// The command and arguments that post this notification, or `None` where the platform has
    /// no way to (or its notifier is not installed), in which case nothing is posted.
    fn command(&self, title: &str, body: &str) -> Option<(String, Vec<String>)>;
}

/// macOS posts through AppleScript, which is always present.
struct MacNotifier;

impl Notifier for MacNotifier {
    fn command(&self, title: &str, body: &str) -> Option<(String, Vec<String>)> {
        let script = format!(
            "display notification {} with title {} sound name \"Glass\"",
            serde_json::to_string(body).unwrap_or_default(),
            serde_json::to_string(title).unwrap_or_default(),
        );
        Some(("osascript".to_string(), vec!["-e".to_string(), script]))
    }
}

/// Linux posts through `notify-send` from libnotify, which is not always installed. `installed`
/// is passed in rather than probed here so the decision is testable.
struct LinuxNotifier {
    installed: bool,
}

impl Notifier for LinuxNotifier {
    fn command(&self, title: &str, body: &str) -> Option<(String, Vec<String>)> {
        self.installed.then(|| {
            (
                "notify-send".to_string(),
                vec![
                    // Records are not urgent and should not stay on screen.
                    "--urgency=low".to_string(),
                    "--app-name=splitrail".to_string(),
                    title.to_string(),
                    body.to_string(),
                ],
            )
        })
    }
}

/// Anywhere else: no desktop notification, and no attempt to find one.
struct NullNotifier;

impl Notifier for NullNotifier {
    fn command(&self, _title: &str, _body: &str) -> Option<(String, Vec<String>)> {
        None
    }
}

/// Whether a command exists on `PATH`, for deciding if `notify-send` is available.
fn on_path(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(command).is_file())
    })
}

fn platform_notifier() -> Box<dyn Notifier> {
    if cfg!(target_os = "macos") {
        Box::new(MacNotifier)
    } else if cfg!(target_os = "linux") {
        Box::new(LinuxNotifier {
            installed: on_path("notify-send"),
        })
    } else {
        Box::new(NullNotifier)
    }
}

fn notify(kind: Kind, new: &Record, old: &Record) {
    let body = format!(
        "{}: {} (was {})",
        kind.label(),
        kind.format(new.value),
        kind.format(old.value)
    );
    notify_text("🏆 New record", &body);
}

fn notify_text(title: &str, body: &str) {
    let Some((program, args)) = platform_notifier().command(title, body) else {
        return;
    };
    // Best effort: a missing or unhappy notifier must never disturb the dashboard.
    let _ = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod notifier_tests {
    use super::{LinuxNotifier, MacNotifier, NotifierTestExt, NullNotifier};

    #[test]
    fn macos_posts_through_osascript_with_escaped_text() {
        let (program, args) = MacNotifier.must_command("🏆 New record", "Cost: $1 (was $2)");
        assert_eq!(program, "osascript");
        assert_eq!(args[0], "-e");
        // Both strings go through serde_json, so a quote in the text cannot break the script.
        let (_, tricky) = MacNotifier.must_command("t", "has \"quotes\" in it");
        assert!(
            tricky[1].contains("\\\"quotes\\\""),
            "quotes must be escaped, not interpolated raw: {}",
            tricky[1]
        );
    }

    #[test]
    fn linux_posts_through_notify_send_when_installed() {
        let (program, args) = LinuxNotifier { installed: true }
            .must_command("🏆 New record", "Cost: $1 (was $2)");
        assert_eq!(program, "notify-send");
        assert!(args.contains(&"--urgency=low".to_string()));
        assert_eq!(args[args.len() - 2], "🏆 New record", "title before body");
        assert_eq!(args[args.len() - 1], "Cost: $1 (was $2)");
    }

    /// Without libnotify there is nothing to call, and nothing should be spawned.
    #[test]
    fn linux_without_notify_send_posts_nothing() {
        use super::Notifier;
        assert!(
            LinuxNotifier { installed: false }
                .command("t", "b")
                .is_none()
        );
    }

    #[test]
    fn other_platforms_post_nothing() {
        use super::Notifier;
        assert!(NullNotifier.command("t", "b").is_none());
    }
}

#[cfg(test)]
/// Test convenience: unwrap the command a notifier builds.
trait NotifierTestExt {
    fn must_command(&self, title: &str, body: &str) -> (String, Vec<String>);
}

#[cfg(test)]
impl<T: Notifier> NotifierTestExt for T {
    fn must_command(&self, title: &str, body: &str) -> (String, Vec<String>) {
        self.command(title, body)
            .expect("this notifier should produce a command")
    }
}

struct Cache {
    at: Instant,
    key: (usize, u64),
    board: Board,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// The board for `stats`, recomputed at most every `MIN_INTERVAL` and only when the data
/// changed, with the saved records merged in. New records are saved and announced.
pub fn board(stats: &[&AgenticCodingToolStats], goal_tks: f64) -> Board {
    let key = (
        stats.iter().map(|s| s.messages.len()).sum(),
        stats.iter().flat_map(|s| s.daily_stats.values()).map(|d| d.stats.output_tokens).sum(),
    );
    observe_plans(stats);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = cache.as_ref()
        && (c.key == key || c.at.elapsed() < MIN_INTERVAL)
    {
        return c.board.clone();
    }
    let mut board = compute(stats, goal_tks, Utc::now());
    let saved = load();
    let (merged, beaten) = ratchet(saved.as_ref().unwrap_or(&BTreeMap::new()), &board.best);
    if saved.as_ref() != Some(&merged) {
        if let Err(e) = save(&merged) {
            crate::utils::log_error(format!("Records: {e}"));
        }
    }
    // Only a record that stood before this run started counts as beaten; the first pass after
    // a start (or with no file yet) just fills the board in.
    if cache.is_some() {
        for (kind, new, old) in &beaten {
            notify(*kind, new, old);
        }
    }
    board.best = merged;
    *cache = Some(Cache { at: Instant::now(), key, board: board.clone() });
    board
}

/// How full each plan window got before it reset: the highest figure seen for it, per account.
/// Sessions keep re-reporting the figure from their last API call, so the latest report can be
/// lower than the real end; the highest one is the honest close.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanWindow {
    /// The signed-in account (`ambertyeung@gmail.com (personal)`), else the column name.
    pub account: String,
    /// A column it was seen in, for display.
    pub tool: String,
    /// `week` or `month`.
    pub window: String,
    pub resets_at: i64,
    pub peak: f64,
    /// Set once the close has been announced.
    #[serde(default)]
    pub announced: bool,
}

/// Windows kept per account and plan.
const PLAN_HISTORY: usize = 26;
/// Reset times drift by seconds between reports; they are compared at this granularity.
const RESET_ROUNDING_SECS: i64 = 15 * 60;

pub fn plans_path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/plan_closes.json"))
}

fn load_plans() -> Vec<PlanWindow> {
    plans_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

/// `seen` (account, tool, window, used, resets_at) folded into `plans`: the peak per window
/// kept, a new window added, old ones trimmed. Returns the windows that have now reset and were
/// not announced yet (marking them announced).
pub fn fold_plans(plans: &mut Vec<PlanWindow>, seen: &[(String, String, &str, f64, i64)], now: i64) -> Vec<PlanWindow> {
    let round = |t: i64| (t + RESET_ROUNDING_SECS / 2).div_euclid(RESET_ROUNDING_SECS);
    for (account, tool, window, used, resets_at) in seen {
        match plans
            .iter_mut()
            .find(|p| &p.account == account && p.window == *window && round(p.resets_at) == round(*resets_at))
        {
            Some(p) => p.peak = p.peak.max(*used),
            None => plans.push(PlanWindow {
                account: account.clone(),
                tool: tool.clone(),
                window: window.to_string(),
                resets_at: *resets_at,
                peak: *used,
                announced: false,
            }),
        }
    }
    plans.sort_by(|a, b| (&a.account, &a.window, a.resets_at).cmp(&(&b.account, &b.window, b.resets_at)));
    let mut kept: Vec<PlanWindow> = Vec::new();
    for p in plans.drain(..).rev() {
        if kept.iter().filter(|k| k.account == p.account && k.window == p.window).count() < PLAN_HISTORY {
            kept.push(p);
        }
    }
    kept.reverse();
    *plans = kept;
    let mut closed = Vec::new();
    for p in plans.iter_mut().filter(|p| p.resets_at <= now && !p.announced) {
        p.announced = true;
        closed.push(p.clone());
    }
    closed
}

static PLANS_CHECKED: Mutex<Option<Instant>> = Mutex::new(None);

/// Note every plan window's current figure (Claude, Codex, Cursor...), at most every 30 s, and
/// announce windows that have just reset with how full they closed.
fn observe_plans(stats: &[&AgenticCodingToolStats]) {
    {
        let mut checked = PLANS_CHECKED.lock().unwrap_or_else(|e| e.into_inner());
        if checked.is_some_and(|at| at.elapsed() < MIN_INTERVAL) {
            return;
        }
        *checked = Some(Instant::now());
    }
    let now = Utc::now().timestamp();
    let mut seen = Vec::new();
    for tool in stats {
        let Some(limits) = crate::plan_limits::limits_for_analyzer(&tool.analyzer_name) else { continue };
        let account = crate::plan_limits::account_for_analyzer(&tool.analyzer_name)
            .unwrap_or_else(|| tool.analyzer_name.clone());
        for (window, status) in [("week", limits.seven_day), ("month", limits.monthly)] {
            if let Some(w) = status
                && let Some(secs) = w.resets_in_secs.filter(|s| *s > 0)
            {
                seen.push((account.clone(), tool.analyzer_name.clone(), window, w.used_percentage, now + secs));
            }
        }
    }
    let first = !plans_path().is_some_and(|p| p.exists());
    let mut plans = load_plans();
    let before = plans.clone();
    let closed = fold_plans(&mut plans, &seen, now);
    if plans != before
        && let Some(path) = plans_path()
    {
        let tmp = path.with_extension("json.tmp");
        let written = serde_json::to_vec_pretty(&plans)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&tmp, bytes))
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            crate::utils::log_error(format!("Plan closes: {e}"));
        }
    }
    if !first {
        for p in closed {
            notify_text(
                &format!("📅 {} {} closed", p.tool, p.window),
                &format!("{:.0}% used, {:.0}% left unused", p.peak, (100.0 - p.peak).max(0.0)),
            );
        }
    }
}

/// Every plan window seen, closed and still open, oldest first, for the Records view.
pub fn plan_windows() -> Vec<PlanWindow> {
    load_plans()
}

/// The last board `board` worked out, if any, without recomputing.
pub fn cached() -> Option<Board> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|c| c.board.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Application, ConversationMessage, DailyStats, Stats};

    fn reply(conversation: &str, at: i64, output: u64) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date: DateTime::from_timestamp(at, 0).unwrap(),
            project_hash: String::new(),
            conversation_hash: conversation.into(),
            local_hash: None,
            global_hash: format!("{conversation}-{at}"),
            model: Some("m".into()),
            stats: Stats { output_tokens: output, ..Default::default() },
            role: MessageRole::Assistant,
            content: None,
        }
    }

    fn tool(days: &[(&str, u64, f64)], messages: Vec<ConversationMessage>) -> AgenticCodingToolStats {
        let mut daily_stats = BTreeMap::new();
        for &(date, output, cost) in days {
            let mut day = DailyStats::default();
            day.stats.output_tokens = output;
            day.stats.cost = cost;
            daily_stats.insert(date.to_string(), day);
        }
        AgenticCodingToolStats { daily_stats, num_conversations: 1, messages, analyzer_name: "t".into() }
    }

    #[test]
    fn peak_rate_finds_the_busiest_window() {
        let points = [(0, 900.0), (100, 900.0), (2000, 9000.0), (2500, 4500.0), (5000, 1.0)];
        let (rate, at) = peak_rate(&points, 900).unwrap();
        assert_eq!((rate, at), (15.0, 2500)); // 13,500 over 15 minutes.
    }

    #[test]
    fn days_weeks_months_and_streaks_add_up_across_tools() {
        let goal_day: f64 = 150.0 * 86_400.0;
        let a = tool(&[("2026-10-12", goal_day as u64, 10.0), ("2026-10-13", goal_day as u64, 5.0)], vec![]);
        let b = tool(&[("2026-10-13", 1_000, 50.0), ("2026-10-14", 1, 1.0), ("2026-10-15", goal_day as u64, 1.0)], vec![]);
        let now = Local.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap().with_timezone(&Utc);
        let board = compute(&[&a, &b], 150.0, now);
        assert_eq!(board.best[&Kind::DayCost].value, 55.0);
        assert!(board.best[&Kind::DayCost].when.contains("Oct 13"));
        assert_eq!(board.best[&Kind::GoalStreak].value, 2.0, "Oct 12-13; Oct 14 broke it");
        assert_eq!(board.now[&Kind::GoalStreak], 1.0, "Oct 15 reaches yesterday");
        assert_eq!(board.best[&Kind::WeekOutput].value, 3.0 * goal_day.floor() + 1_001.0);
        assert_eq!(board.best[&Kind::WeekOutput].when, "week of Oct 12 2026");
    }

    #[test]
    fn parallel_sessions_count_distinct_conversations_in_one_slot() {
        let t = tool(&[], vec![reply("a", 600, 1), reply("b", 650, 1), reply("c", 899, 1), reply("a", 700, 1), reply("d", 1300, 1)]);
        let board = compute(&[&t], 150.0, DateTime::from_timestamp(10_000, 0).unwrap());
        assert_eq!(board.best[&Kind::ParallelSessions].value, 3.0);
    }

    #[test]
    fn saved_records_only_go_up_and_a_first_record_is_not_beaten() {
        let rec = |value: f64| Record { value, when: "x".into() };
        let saved = BTreeMap::from([(Kind::DayOutput, rec(10.0)), (Kind::DayCost, rec(99.0))]);
        let computed = BTreeMap::from([(Kind::DayOutput, rec(12.0)), (Kind::DayCost, rec(50.0)), (Kind::GoalStreak, rec(3.0))]);
        let (merged, beaten) = ratchet(&saved, &computed);
        assert_eq!(merged[&Kind::DayOutput].value, 12.0);
        assert_eq!(merged[&Kind::DayCost].value, 99.0, "pruned transcripts never lower a record");
        assert_eq!(merged[&Kind::GoalStreak].value, 3.0);
        assert_eq!(beaten.len(), 1);
        assert_eq!(beaten[0].0, Kind::DayOutput);
    }

    #[test]
    fn a_plan_window_keeps_its_peak_and_is_announced_once_after_it_resets() {
        let mut plans = Vec::new();
        let seen = |used: f64, resets_at: i64| vec![("me@x.com".to_string(), "Claude (main)".to_string(), "week", used, resets_at)];
        assert!(fold_plans(&mut plans, &seen(90.0, 10_000), 1_000).is_empty());
        // An idle session re-reports an older, lower figure with a reset a few seconds off.
        fold_plans(&mut plans, &seen(93.0, 10_005), 2_000);
        fold_plans(&mut plans, &seen(79.0, 9_990), 3_000);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].peak, 93.0);
        // The next week starts; the old one is now closed and announced, once.
        let closed = fold_plans(&mut plans, &seen(2.0, 10_000 + 7 * 86_400), 10_100);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].peak, 93.0);
        assert!(fold_plans(&mut plans, &[], 10_200).is_empty());
        assert_eq!(plans.len(), 2);
    }

    use chrono::TimeZone;
}

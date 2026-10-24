use crate::types::{AgenticCodingToolStats, MultiAnalyzerStats};
use crate::utils::{
    NumberFormatOptions, clear_old_warnings, format_date_for_display, format_number, log_error,
    read_log_tail, warn_once,
};
use crate::watcher::{FileWatcher, RealtimeStatsManager};
use anyhow::{bail, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::style::{Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{ExecutableCommand, execute};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Widget, Borders};
use ratatui::{Frame, Terminal};
use std::io::{stdout, Write, IsTerminal};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

// Import the summary dashboard module
mod summary_dashboard;
use summary_dashboard::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct UiLayout {
    pub today_by_cli_rect: Option<Rect>,
    pub live_activity_rect: Option<Rect>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PerfSnapshot {
    pub fps: f32,
    pub render_ms: f32,
    pub cpu_pct: Option<f32>,
}

#[derive(Debug)]
pub struct PerfOverlayState {
    pub enabled: bool,
    pub last_snapshot: Option<PerfSnapshot>,
    window_start: std::time::Instant,
    render_count: u64,
    render_total: std::time::Duration,
    cpu_last_wall: Option<std::time::Instant>,
    cpu_last_proc: Option<std::time::Duration>,
}

impl PerfOverlayState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            window_start: std::time::Instant::now(),
            last_snapshot: None,
            render_count: 0,
            render_total: std::time::Duration::ZERO,
            cpu_last_wall: None,
            cpu_last_proc: None,
        }
    }

    pub fn record_render(&mut self, start: Option<std::time::Instant>) {
        if self.enabled {
            if let Some(start) = start {
                self.render_total += start.elapsed();
                self.render_count += 1;
            }
        }
    }

    pub fn maybe_snapshot(&mut self) {
        if !self.enabled {
            return;
        }
        let now = std::time::Instant::now();
        let window = now.duration_since(self.window_start);
        if window >= std::time::Duration::from_secs(2) {
            let secs = window.as_secs_f32().max(0.001);
            let fps = self.render_count as f32 / secs;
            let render_ms = if self.render_count > 0 {
                (self.render_total.as_secs_f32() / self.render_count as f32) * 1000.0
            } else {
                0.0
            };
            let cpu_pct = self.compute_cpu_pct(now);
            self.last_snapshot = Some(PerfSnapshot {
                fps,
                render_ms,
                cpu_pct,
            });
            self.window_start = now;
            self.render_count = 0;
            self.render_total = std::time::Duration::ZERO;
        }
    }

    fn compute_cpu_pct(&mut self, now: std::time::Instant) -> Option<f32> {
        #[cfg(unix)]
        {
            if let Some(proc_time) = read_process_cpu_time() {
                if let (Some(prev_wall), Some(prev_proc)) =
                    (self.cpu_last_wall, self.cpu_last_proc)
                {
                    let wall_delta = now.saturating_duration_since(prev_wall);
                    let proc_delta = proc_time.checked_sub(prev_proc)?;
                    if wall_delta.as_secs_f32() > 0.0 {
                        let pct =
                            (proc_delta.as_secs_f32() / wall_delta.as_secs_f32()) * 100.0;
                        self.cpu_last_wall = Some(now);
                        self.cpu_last_proc = Some(proc_time);
                        return Some(pct);
                    }
                }
                self.cpu_last_wall = Some(now);
                self.cpu_last_proc = Some(proc_time);
            }
            None
        }
        #[cfg(not(unix))]
        {
            let _ = now;
            None
        }
    }
}

impl Default for PerfOverlayState {
    fn default() -> Self {
        Self::new(false)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DebugOverlayState {
    pub enabled: bool,
    pub lines: Vec<String>,
    pub last_refresh: Option<std::time::Instant>,
}

#[cfg(unix)]
fn read_process_cpu_time() -> Option<std::time::Duration> {
    use std::time::Duration;
    unsafe {
        let mut usage = std::mem::zeroed::<libc::rusage>();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
            let user = Duration::new(
                usage.ru_utime.tv_sec as u64,
                (usage.ru_utime.tv_usec * 1000) as u32,
            );
            let sys = Duration::new(
                usage.ru_stime.tv_sec as u64,
                (usage.ru_stime.tv_usec * 1000) as u32,
            );
            return Some(user + sys);
        }
    }
    None
}

fn refresh_debug_overlay(tui_state: &mut TuiState) {
    if !tui_state.debug_overlay.enabled {
        tui_state.debug_overlay.lines.clear();
        tui_state.debug_overlay.last_refresh = None;
        return;
    }

    tui_state.debug_overlay.last_refresh = Some(Instant::now());
    tui_state.debug_overlay.lines = match read_log_tail(200) {
        Some(lines) if lines.is_empty() => {
            vec!["Log file is empty (written to ~/.splitrail.log)".to_string()]
        }
        Some(lines) => lines,
        None => vec!["No log file yet (written to ~/.splitrail.log)".to_string()],
    };
}

#[derive(Debug, Clone)]
pub struct ClockTitleCache {
    pub area: Rect,
    pub spacer: String,
    pub countdown_section: String,
    pub countdown_color: Color,
    pub last_section: String,
    pub slack_icon: String,
    pub slack_color: Color,
    pub tks_display: String,
}

/// How a tool tab groups its rows (`g` cycles them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Period {
    #[default]
    Day,
    Week,
    Month,
    Hour,
}

impl Period {
    fn next(self) -> Self {
        match self {
            Period::Day => Period::Week,
            Period::Week => Period::Month,
            Period::Month => Period::Hour,
            Period::Hour => Period::Day,
        }
    }

    /// Short unit for the totals row: `Total (30d)`.
    fn unit(self) -> &'static str {
        match self {
            Period::Day => "d",
            Period::Week => "w",
            Period::Month => "mo",
            Period::Hour => "h",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Week => "week",
            Period::Month => "month",
            Period::Hour => "hour",
        }
    }

    /// The row a `YYYY-MM-DD` day belongs to, as a key that sorts in time order:
    /// the day itself, `2026-W42` (ISO week) or `2026-10`.
    fn key(self, day: &str) -> String {
        let Ok(date) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") else {
            return day.to_string();
        };
        match self {
            Period::Day => day.to_string(),
            Period::Week => {
                let week = chrono::Datelike::iso_week(&date);
                format!("{}-W{:02}", week.year(), week.week())
            }
            Period::Month => date.format("%Y-%m").to_string(),
            // Hours come from the day's slices, not from here.
            Period::Hour => day.to_string(),
        }
    }
}

/// Esc: undo the innermost thing open, one level at a time. On the Summary tab: records or
/// quotas, then verbose; on a tool tab: a filter, then week/month/hour rows, then back to the
/// Summary tab. False at the top, where there is nothing to go back from.
fn go_back(state: &mut TuiState) -> bool {
    if state.help_open {
        state.help_open = false;
    } else if state.selected_tab == 0 {
        if state.records_view || state.quotas_view {
            state.records_view = false;
            state.quotas_view = false;
        } else if state.summary_verbose_mode {
            state.summary_verbose_mode = false;
            state.perf_overlay.enabled = false;
        } else {
            return false;
        }
    } else if !state.filter.is_empty() {
        state.filter = SliceFilter::default();
    } else if state.period != Period::Day {
        state.period = Period::Day;
    } else {
        state.selected_tab = 0;
    }
    true
}

/// Which slices the tool tabs count: one model and/or one project (BZL-138).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SliceFilter {
    pub model: Option<String>,
    pub project: Option<String>,
}

impl SliceFilter {
    fn is_empty(&self) -> bool {
        self.model.is_none() && self.project.is_none()
    }

    fn admits(&self, slice: &crate::types::Slice) -> bool {
        self.model.as_ref().is_none_or(|m| *m == slice.model) && self.project.as_ref().is_none_or(|p| *p == slice.project)
    }

    /// `· model claude-opus-5-5 · project splitrail`, or empty.
    fn describe(&self) -> String {
        let mut parts = String::new();
        if let Some(model) = &self.model {
            parts.push_str(&format!(" · model {model}"));
        }
        if let Some(project) = &self.project {
            parts.push_str(&format!(" · project {project}"));
        }
        parts
    }
}

/// The next value after `current` among `options` (busiest first), wrapping to none.
fn cycle(current: &Option<String>, options: &[String]) -> Option<String> {
    match current.as_ref().and_then(|c| options.iter().position(|o| o == c)) {
        None => options.first().cloned(),
        Some(i) => options.get(i + 1).cloned(),
    }
}

/// A tool's models or projects, by output tokens across its history, busiest first.
fn slice_options(stats: &AgenticCodingToolStats, by_model: bool) -> Vec<String> {
    let mut output: std::collections::HashMap<&str, u64> = Default::default();
    for slice in stats.daily_stats.values().flat_map(|d| &d.slices) {
        let key = if by_model { slice.model.as_str() } else { slice.project.as_str() };
        *output.entry(key).or_default() += slice.output_tokens.max(1);
    }
    let mut options: Vec<(&str, u64)> = output.into_iter().collect();
    options.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    options.into_iter().map(|(k, _)| k.to_string()).collect()
}

/// Rows built from the days' slices: by hour, or by day/week/month under a filter. Slices carry
/// tokens, cost and replies only, so the other columns stay empty.
fn group_slices(stats: &AgenticCodingToolStats, period: Period, filter: &SliceFilter) -> AgenticCodingToolStats {
    let mut grouped: std::collections::BTreeMap<String, crate::types::DailyStats> = Default::default();
    for (day, day_stats) in &stats.daily_stats {
        for slice in day_stats.slices.iter().filter(|s| filter.admits(s)) {
            let key = match period {
                Period::Hour => format!("{day} {:02}", slice.hour),
                _ => period.key(day),
            };
            let row = grouped.entry(key.clone()).or_insert_with(|| crate::types::DailyStats {
                date: key,
                ..Default::default()
            });
            row.ai_messages += slice.replies;
            *row.models.entry(slice.model.clone()).or_default() += slice.replies;
            row.stats.input_tokens += slice.input_tokens;
            row.stats.output_tokens += slice.output_tokens;
            row.stats.cache_creation_tokens += slice.cache_creation_tokens;
            row.stats.cache_read_tokens += slice.cache_read_tokens;
            row.stats.cached_tokens += slice.cache_read_tokens;
            row.stats.cost += slice.cost;
        }
    }
    AgenticCodingToolStats {
        daily_stats: grouped,
        num_conversations: stats.num_conversations,
        messages: Vec::new(),
        analyzer_name: stats.analyzer_name.clone(),
    }
}

/// `stats` with its days merged into weeks or months. Only the daily totals are carried over,
/// which is all the table reads.
fn group_by_period(stats: &AgenticCodingToolStats, period: Period) -> AgenticCodingToolStats {
    let mut grouped: std::collections::BTreeMap<String, crate::types::DailyStats> = Default::default();
    for (day, day_stats) in &stats.daily_stats {
        let key = period.key(day);
        let row = grouped.entry(key.clone()).or_insert_with(|| crate::types::DailyStats {
            date: key,
            ..Default::default()
        });
        row.user_messages += day_stats.user_messages;
        row.ai_messages += day_stats.ai_messages;
        row.conversations += day_stats.conversations;
        row.active_seconds += day_stats.active_seconds;
        for (model, count) in &day_stats.models {
            *row.models.entry(model.clone()).or_default() += count;
        }
        crate::utils::add_stats_delta_into_day(row, &day_stats.stats, day_stats.stats.cost);
    }
    AgenticCodingToolStats {
        daily_stats: grouped,
        num_conversations: stats.num_conversations,
        messages: Vec::new(),
        analyzer_name: stats.analyzer_name.clone(),
    }
}

#[derive(Debug, Default)]
pub struct TuiState {
    pub selected_tab: usize,
    #[allow(dead_code)]
    pub scroll_offset: usize,
    pub summary_day_offset: usize,
    pub cli_scroll_offset: usize,
    pub cli_table_scroll_offset: usize, // For horizontal scroll
    pub layout: UiLayout,
    pub mouse_mode_enabled: bool, // Toggle between mouse scroll and text selection
    pub summary_verbose_mode: bool,
    /// `p` on the Summary tab: plan quotas instead of the summary.
    pub quotas_view: bool,
    /// `r` on the Summary tab: personal bests instead of the summary.
    pub records_view: bool,
    /// `?`: every keyboard shortcut, over whatever is showing.
    pub help_open: bool,
    /// Rows on the tool tabs: days, weeks, months or hours.
    pub period: Period,
    /// `f` / `o` on the tool tabs: only one model / project; `x` clears.
    pub filter: SliceFilter,
    pub daily_stats_view_start: usize, // For daily stats pagination (0 = most recent 30 days)
    pub status_message: Option<String>, // Temporary status messages (Slack toggle, etc.)
    pub status_message_timer: Option<std::time::Instant>, // Auto-clear timer for status messages
    pub clock_title_cache: Option<ClockTitleCache>, // Cached geometry + static text for clock-only redraws
    pub perf_overlay: PerfOverlayState,
    pub debug_overlay: DebugOverlayState,
}

#[derive(Debug, Clone)]
pub enum UploadStatus {
    None,
    Uploading {
        current: usize,
        total: usize,
        dots: usize,
    },
    Uploaded,
    Failed(String), // Include error message
    MissingApiToken,
    MissingServerUrl,
    MissingConfig,
}

fn has_data(stats: &AgenticCodingToolStats) -> bool {
    stats.num_conversations > 0
        || stats.daily_stats.values().any(|day| {
            day.stats.cost > 0.0
                || day.stats.input_tokens > 0
                || day.stats.output_tokens > 0
                || day.stats.reasoning_tokens > 0
                || day.stats.tool_calls > 0
        })
}

pub fn run_tui(
    stats_receiver: watch::Receiver<MultiAnalyzerStats>,
    format_options: &NumberFormatOptions,
    upload_status: Arc<Mutex<UploadStatus>>,
    file_watcher: FileWatcher,
    mut stats_manager: RealtimeStatsManager,
) -> Result<()> {
    let mut stdout = stdout();

    // Crossterm raw mode fails if stdout isn't a TTY (e.g., CI, log capture). Bail early with a clear message.
    if !stdout.is_terminal() {
        bail!("Interactive TUI requires a terminal (stdout is not a TTY). Run from a terminal or use subcommands like `splitrail config`/`splitrail upload`.");
    }

    // Restore the terminal on a panic too, before the default hook prints the message.
    // Otherwise the panic output lands on the alternate screen and the user's shell is
    // left with raw mode and mouse tracking still enabled.
    // The message is also written to ~/.splitrail.log: once the TUI is gone, the terminal's
    // copy is easily lost (a restart in the same window clears it).
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        let thread = std::thread::current();
        crate::utils::log_error(format!(
            "panic in thread '{}': {info}\n{}",
            thread.name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        ));
        default_panic_hook(info);
    }));

    enable_raw_mode()?;
    let _guard = TerminalGuard;
    stdout.execute(EnterAlternateScreen)?;
    crate::utils::set_tui_active(true);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut tui_state = TuiState::default();
    // Enable mouse mode by default for consistent scrolling
    tui_state.mouse_mode_enabled = true;
    set_mouse(true)?;

    // Start with perf overlay disabled; user can toggle via verbose mode
    tui_state.perf_overlay = PerfOverlayState::new(false);

    let shutdown = Arc::new(AtomicBool::new(false));
    spawn_signal_listener(shutdown.clone());

    // `_guard` is dropped after this returns, on success, `?` error, or unwind, and puts the
    // terminal back in every case.
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(run_app(
            &mut terminal,
            stats_receiver,
            format_options,
            &mut tui_state,
            upload_status,
            file_watcher,
            &mut stats_manager,
            shutdown,
        ))
    })
}

/// Puts the terminal back the way we found it. Held for the lifetime of the TUI so that every
/// exit path (normal quit, `?` error, panic, signal) restores raw mode, mouse capture, the
/// alternate screen, and the cursor. The escape sequences are idempotent, so running this more
/// than once is harmless.
struct TerminalGuard;

/// Mouse reporting for clicks and the wheel only (1000, in SGR form 1006). Crossterm's
/// `EnableMouseCapture` also turns on 1003, every movement, which the TUI never uses: it floods
/// input with `35;x;yM` reports while the pointer moves, and any still in flight when the
/// dashboard exits are typed into the shell.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
/// Every mode either this or crossterm's capture may have turned on.
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

fn set_mouse(on: bool) -> std::io::Result<()> {
    let mut out = stdout();
    out.write_all(if on { MOUSE_ON } else { MOUSE_OFF }.as_bytes())?;
    out.flush()
}

impl TerminalGuard {
    /// Mouse reporting goes off while still in raw mode, and reports already on their way are
    /// discarded before the shell gets the terminal back, so none of them reach the prompt.
    fn restore() {
        crate::utils::set_tui_active(false);
        let _ = set_mouse(false);
        std::thread::sleep(Duration::from_millis(30));
        // Discard mouse reports already in flight so none reach the shell prompt. POSIX only;
        // Windows has no tcflush, and the console is left as crossterm restores it.
        #[cfg(unix)]
        // SAFETY: tcflush only discards the terminal's pending input; stdin is fd 0.
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        }
        let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

/// Wait for SIGTERM, SIGHUP or SIGINT; false if they cannot be listened for.
#[cfg(unix)]
async fn wait_for_quit_signal() -> bool {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut hup), Ok(mut int)) =
        (signal(SignalKind::terminate()), signal(SignalKind::hangup()), signal(SignalKind::interrupt()))
    else {
        return false;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = hup.recv() => {}
        _ = int.recv() => {}
    }
    true
}

/// Windows has no such signals: Ctrl-C (and Ctrl-Break, closing the console) is the one there is.
#[cfg(windows)]
async fn wait_for_quit_signal() -> bool {
    tokio::signal::ctrl_c().await.is_ok()
}

/// Turn SIGTERM / SIGHUP / SIGINT into a clean quit. The event loop checks `shutdown` on every
/// iteration; if it is stuck in a long blocking reload and has not exited within a few seconds,
/// restore the terminal directly and exit so a `kill` never leaves the shell broken.
fn spawn_signal_listener(shutdown: Arc<AtomicBool>) {
    tokio::spawn(async move {
        if !wait_for_quit_signal().await {
            return;
        }
        shutdown.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(3)).await;
        // Still running: the loop is blocked. Restore and bail out.
        TerminalGuard::restore();
        std::process::exit(130);
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut stats_receiver: watch::Receiver<MultiAnalyzerStats>,
    format_options: &NumberFormatOptions,
    tui_state: &mut TuiState,
    upload_status: Arc<Mutex<UploadStatus>>,
    file_watcher: FileWatcher,
    stats_manager: &mut RealtimeStatsManager,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let mut table_states: Vec<TableState> = Vec::new();
    let mut current_stats = stats_receiver.borrow().clone();

    // Initialize table states for current stats
    update_table_states(
        &mut table_states,
        &current_stats,
        &mut tui_state.selected_tab,
    );

    let mut needs_redraw = true;
    let mut last_upload_status = {
        let status = upload_status.lock().unwrap();
        format!("{:?}", *status)
    };
    let mut dots_counter = 0; // Counter for dots animation (advance every 5 frames = 500ms)

    // Filter analyzer stats to only include those with data - calculate once and update when stats change
    let mut filtered_stats: Vec<&AgenticCodingToolStats> = current_stats
        .analyzer_stats
        .iter()
        .map(std::sync::Arc::as_ref)
        .filter(|stats| has_data(stats))
        .collect();

    // Cache summary data to avoid recalculating on every redraw
    let mut cached_summary_data: Option<SummaryData> = Some(calculate_summary_data(
        &filtered_stats,
        tui_state.summary_day_offset,
    ));

    // Adaptive polling: adjust poll rate based on activity
    let base_poll_ms: u64 = std::env::var("SPLITRAIL_POLL_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let mut last_activity = Instant::now();
    let idle_threshold = Duration::from_secs(3); // After 3s of no activity, slow down

    // Rate limit warning cleanup to once per minute
    let mut last_warning_cleanup = std::time::Instant::now();
    let mut last_sparkline_refresh = Instant::now();
    let mut last_clock_tick = Instant::now();
    
    // PERFORMANCE OPTIMIZATIONS APPLIED:
    // 1. Clock updates no longer trigger full UI redraws (was 38% CPU reduction)
    // 2. Status message clears no longer trigger full UI redraws  
    // 3. Sparkline refresh reduced from 5s to 10s (50% reduction)
    // 4. More aggressive idle polling (2000ms option for background monitoring)
    // 5. Intelligent redraw system (prevents unnecessary renders)

    loop {
        // A signal (SIGTERM/SIGHUP/SIGINT) asked us to stop.
        // A finished update: quit so main can restart into the new binary.
        if crate::self_update::restart_pending() {
            break;
        }
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Update perf snapshot at the start of each loop (uses data from previous renders)
        tui_state.perf_overlay.maybe_snapshot();

        // Check for stats updates
        if stats_receiver.has_changed()? {
            current_stats = stats_receiver.borrow_and_update().clone();
            update_table_states(
                &mut table_states,
                &current_stats,
                &mut tui_state.selected_tab,
            );
            // Recalculate filtered stats only when stats change
            filtered_stats = current_stats
                .analyzer_stats
                .iter()
                .map(std::sync::Arc::as_ref)
                .filter(|stats| has_data(stats))
                .collect();
            // Recalculate summary data when stats change
            cached_summary_data = Some(calculate_summary_data(
                &filtered_stats,
                tui_state.summary_day_offset,
            ));
            last_sparkline_refresh = Instant::now();
            needs_redraw = true;
            // Data changed - reset to active polling
            last_activity = Instant::now();
        }

        // Check for file watcher events (batched)
        while let Some(watcher_event) = file_watcher.try_recv() {
            if let Err(e) = stats_manager.handle_watcher_event(watcher_event).await {
                log_error(format!("Error handling watcher event: {e:#}"));
                warn_once("Watcher event failed; see ~/.splitrail.log");
            }
        }

        // Process batched file events if window has expired
        if let Err(e) = stats_manager.process_pending_batches().await {
            log_error(format!("Error processing batched events: {e:#}"));
            warn_once("Processing batched events failed; see ~/.splitrail.log");
        }

        // Poll Codex CLI periodically (every 5 seconds)
        if let Err(e) = stats_manager.poll_codex_if_needed().await {
            log_error(format!("Error polling Codex CLI: {e:#}"));
            warn_once("Codex poll failed; see ~/.splitrail.log");
        }

        // Check if upload status has changed or advance dots animation
        let current_upload_status = {
            let mut status = upload_status.lock().unwrap();
            // Advance dots animation for uploading status every 500ms (5 frames at 100ms)
            if let UploadStatus::Uploading {
                current: _,
                total: _,
                dots,
            } = &mut *status
            {
                // Always animate dots during upload
                dots_counter += 1;
                if dots_counter >= 5 {
                    *dots = (*dots + 1) % 4;
                    dots_counter = 0;
                    needs_redraw = true;
                }
            } else {
                // Reset counter when not uploading
                dots_counter = 0;
            }
            format!("{:?}", *status)
        };
        if current_upload_status != last_upload_status {
            last_upload_status = current_upload_status;
            needs_redraw = true;
        }

        // Clean up old warnings (older than 1 minute) - rate limited to once per minute
        if last_warning_cleanup.elapsed() >= Duration::from_secs(60) {
            clear_old_warnings();
            last_warning_cleanup = std::time::Instant::now();
        }

        // Refresh debug overlay (tail of ~/.splitrail.log)
        if tui_state.debug_overlay.enabled {
            let refresh_due = tui_state
                .debug_overlay
                .last_refresh
                .map_or(true, |t| t.elapsed() >= Duration::from_secs(1));
            if refresh_due {
                refresh_debug_overlay(tui_state);
                needs_redraw = true;
            }
        }

        // OPTIMIZATION: Reduce sparkline refresh frequency from 5s to 10s
        // This cuts expensive sparkline recomputation in half
        let sparkline_interval_secs = if std::env::var("SPLITRAIL_FAST_SPARKLINES").is_ok() {
            5  // Allow fast refresh for debugging
        } else {
            10  // Default: slower refresh for CPU savings
        };
        
        if last_sparkline_refresh.elapsed() >= Duration::from_secs(sparkline_interval_secs) {
            if let Some(summary) = cached_summary_data.as_mut() {
                let now_utc = chrono::Utc::now();
                let (cache, max_cache, buffers) = build_activity_sparkline_cache_with_prev(
                    &filtered_stats,
                    now_utc,
                    Some(&summary.sparkline_max_cache),
                    Some(&summary.sparkline_buffers),
                );
                summary.sparkline_cache = cache;
                summary.sparkline_max_cache = max_cache;
                summary.sparkline_buffers = buffers;
                // Also refresh tokens per second (same cadence)
                summary.tokens_per_second = calculate_tokens_per_second(&filtered_stats, now_utc);
                needs_redraw = true;
            }
            last_sparkline_refresh = Instant::now();
        }

        // OPTIMIZATION: Intelligent redraw system
        // Only redraw if something has actually changed
        if needs_redraw {
            // Cache time values ONCE for the entire render cycle to avoid syscall spam
            let render_time_utc = chrono::Utc::now();
            let render_time_system = SystemTime::now();

            let render_start = if tui_state.perf_overlay.enabled {
                Some(Instant::now())
            } else {
                None
            };
            terminal.draw(|frame| {
                draw_ui(
                    frame,
                    &filtered_stats,
                    format_options,
                    &mut table_states,
                    tui_state,
                    upload_status.clone(),
                    &cached_summary_data,
                    render_time_utc,
                    render_time_system,
                );
            })?;
            tui_state.perf_overlay.record_render(render_start);
            needs_redraw = false;
        }

        // OPTIMIZATION: Clock updates work but don't trigger full UI redraw
        // Clock updates should update the clock display without full UI redraw
        let clock_enabled = std::env::var("SPLITRAIL_DISABLE_CLOCK").is_err();
        if clock_enabled && last_clock_tick.elapsed() >= Duration::from_secs(1) {
            last_clock_tick = Instant::now();
            match redraw_clock_only(terminal, tui_state) {
                Ok(true) => {}
                Ok(false) => {
                    // No cache yet; fall back to a full redraw on summary tab
                    if tui_state.selected_tab == 0 {
                        needs_redraw = true;
                    }
                }
                Err(err) => {
                    crate::utils::report(format!(
                        "Clock-only redraw failed, falling back to full redraw: {err}"
                    ));
                    needs_redraw = true;
                }
            }
        }
        
        // OPTIMIZATION: Auto-clear status messages after 3 seconds
        // This also triggers unnecessary full UI redraws
        if let (Some(_), Some(timer)) = (&tui_state.status_message, &tui_state.status_message_timer) {
            if timer.elapsed() >= Duration::from_secs(3) {
                tui_state.status_message = None;
                tui_state.status_message_timer = None;
                // REMOVED: needs_redraw = true;  // Don't redraw entire UI for status clear
            }
        }

        // OPTIMIZATION: More aggressive adaptive polling
        // Use environment variable to allow even slower polling
        let time_since_activity = Instant::now().duration_since(last_activity);
        let poll_ms = if time_since_activity > idle_threshold {
            // Idle mode: very slow polling for maximum CPU savings
            if std::env::var("SPLITRAIL_ULTRA_SLOW").is_ok() {
                2000  // Ultra-slow: 0.5 FPS for background monitoring
            } else {
                1000  // Normal idle: 1 FPS
            }
        } else {
            // Active mode: use base rate
            base_poll_ms
        };

        if let Ok(event_available) = event::poll(Duration::from_millis(poll_ms)) {
            if !event_available {
                continue;
            }

            // Handle different event types
            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    // Reset activity timer on any key press
                    last_activity = Instant::now();

                    // Handle Shift+Enter for upload
                    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
                        // Trigger upload
                        if let Err(e) = stats_manager.trigger_manual_upload().await {
                            log_error(format!("Error triggering upload: {e:#}"));
                            warn_once("Manual upload failed; see ~/.splitrail.log");
                        }
                        needs_redraw = true;
                        continue;
                    }

                    // Handle quitting. Raw mode delivers Ctrl-C as a key event rather than
                    // SIGINT, so treat it as quit too; otherwise users force-kill the process
                    // and the terminal is left with mouse tracking enabled.
                    let ctrl_c = matches!(key.code, KeyCode::Char('c'))
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_c || key.code == KeyCode::Char('q') {
                        break;
                    }

                    // ?: every shortcut, in a panel; ? again (or Esc) closes it.
                    if key.code == KeyCode::Char('?') {
                        tui_state.help_open = !tui_state.help_open;
                        needs_redraw = true;
                        continue;
                    }

                    // Esc goes back one level; it never quits.
                    if key.code == KeyCode::Esc {
                        if go_back(tui_state) {
                            tui_state.daily_stats_view_start = 0;
                            for state in table_states.iter_mut() {
                                state.select(Some(0));
                            }
                        } else {
                            tui_state.status_message = Some("Press q to quit".to_string());
                            tui_state.status_message_timer = Some(std::time::Instant::now());
                        }
                        needs_redraw = true;
                        continue;
                    }

                    // U: build the newer version in the source tree, then restart into it.
                    if matches!(key.code, KeyCode::Char('U') | KeyCode::Char('u')) {
                        crate::self_update::start();
                        needs_redraw = true;
                        continue;
                    }

                    // Ctrl-L repaints every cell, the usual terminal convention, for anything that
                    // still manages to draw behind ratatui's back.
                    if matches!(key.code, KeyCode::Char('l'))
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        terminal.clear()?;
                        needs_redraw = true;
                        continue;
                    }

                    // Handle mouse mode toggle
                    if matches!(key.code, KeyCode::Char('m')) {
                        tui_state.mouse_mode_enabled = !tui_state.mouse_mode_enabled;
                        if tui_state.mouse_mode_enabled {
                            set_mouse(true)?;
                        } else {
                            // Disable mouse capture and flush any pending events
                            set_mouse(false)?;
                            // Consume any queued mouse events
                            while event::poll(Duration::from_millis(0))? {
                                if let Event::Mouse(_) = event::read()? {
                                    // Discard mouse event
                                } else {
                                    break;
                                }
                            }
                        }
                        needs_redraw = true;
                        continue;
                    }

                    // Toggle debug log overlay
                    if matches!(key.code, KeyCode::Char('d')) {
                        tui_state.debug_overlay.enabled = !tui_state.debug_overlay.enabled;
                        refresh_debug_overlay(tui_state);
                        needs_redraw = true;
                        continue;
                    }

                    // Only handle navigation keys if we have data (`filtered_stats` is non-empty).
                    if filtered_stats.is_empty() {
                        continue;
                    }

                    if key.code == KeyCode::Char('p') && tui_state.selected_tab == 0 {
                        tui_state.quotas_view = !tui_state.quotas_view;
                        tui_state.records_view = false;
                        needs_redraw = true;
                        continue;
                    }

                    if key.code == KeyCode::Char('g') && tui_state.selected_tab > 0 {
                        tui_state.period = tui_state.period.next();
                        tui_state.daily_stats_view_start = 0;
                        for state in table_states.iter_mut() {
                            state.select(Some(0));
                        }
                        needs_redraw = true;
                        continue;
                    }

                    if matches!(key.code, KeyCode::Char('f') | KeyCode::Char('o') | KeyCode::Char('x'))
                        && tui_state.selected_tab > 0
                    {
                        if let Some(stats) = filtered_stats.get(tui_state.selected_tab - 1) {
                            match key.code {
                                KeyCode::Char('f') => {
                                    tui_state.filter.model = cycle(&tui_state.filter.model, &slice_options(stats, true))
                                }
                                KeyCode::Char('o') => {
                                    tui_state.filter.project =
                                        cycle(&tui_state.filter.project, &slice_options(stats, false))
                                }
                                _ => tui_state.filter = SliceFilter::default(),
                            }
                        }
                        tui_state.daily_stats_view_start = 0;
                        for state in table_states.iter_mut() {
                            state.select(Some(0));
                        }
                        needs_redraw = true;
                        continue;
                    }

                    if key.code == KeyCode::Char('r') && tui_state.selected_tab == 0 {
                        tui_state.records_view = !tui_state.records_view;
                        tui_state.quotas_view = false;
                        needs_redraw = true;
                        continue;
                    }

                    if key.code == KeyCode::Char('v') && tui_state.selected_tab == 0 {
                        tui_state.summary_verbose_mode = !tui_state.summary_verbose_mode;
                        // Toggle perf overlay with verbose mode; reset stats when enabling
                        if tui_state.summary_verbose_mode {
                            tui_state.perf_overlay = PerfOverlayState::new(true);
                        } else {
                            tui_state.perf_overlay.enabled = false;
                        }
                        needs_redraw = true;
                        continue;
                    }

                        // Handle Slack notifications toggle
                    if key.code == KeyCode::Char('s') {
                        match crate::config::Config::load() {
                            Ok(Some(mut config)) => {
                                let _was_enabled = config.notifications.slack.enabled;
                                config.notifications.slack.enabled = !config.notifications.slack.enabled;
                                
                                // Save the updated config
                                if let Err(e) = config.save(false) {
                                    crate::utils::report(format!("Failed to save config: {e}"));
                                } else {
                                    // Takes effect now, not at the next start.
                                    crate::notifications::set_slack_enabled(config.notifications.slack.enabled);
                                    // Set status message with auto-clear timer
                                    let status = if config.notifications.slack.enabled {
                                        "✅ Slack notifications enabled"
                                    } else {
                                        "❌ Slack notifications disabled"
                                    };
                                    tui_state.status_message = Some(status.to_string());
                                    tui_state.status_message_timer = Some(std::time::Instant::now());
                                }
                            }
                            Ok(None) => {
                                tui_state.status_message = Some("❌ No configuration found. Run 'splitrail config init' to create one.".to_string());
                                tui_state.status_message_timer = Some(std::time::Instant::now());
                            }
                            Err(e) => {
                                tui_state.status_message = Some(format!("❌ Failed to load configuration: {}", e));
                                tui_state.status_message_timer = Some(std::time::Instant::now());
                            }
                        }
                        needs_redraw = true;
                        continue;
                    }

                    match key.code {
                        KeyCode::Left | KeyCode::Char('h') => {
                            if tui_state.selected_tab > 0 {
                                tui_state.selected_tab -= 1;
                                needs_redraw = true;
                            }
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            if tui_state.selected_tab < filtered_stats.len() {
                                // +1 for Summary tab
                                tui_state.selected_tab += 1;
                                needs_redraw = true;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            // On summary tab, navigate through days (towards today)
                            if tui_state.selected_tab == 0 {
                                if tui_state.summary_day_offset > 0 {
                                    tui_state.summary_day_offset -= 1;
                                    cached_summary_data = Some(calculate_summary_data(
                                        &filtered_stats,
                                        tui_state.summary_day_offset,
                                    ));
                                    needs_redraw = true;
                                }
                            }
                            // Only handle table navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            else if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(current_stats) = filtered_stats.get(analyzer_index)
                                {
                                    // Calculate visible rows (only showing last 30 days by default)
                                    let total_days = current_stats.daily_stats.len();
                                    let view_limit = 30;
                                    let visible_days = std::cmp::min(view_limit, total_days);
                                    let total_rows = visible_days + 2; // data rows + separator + totals

                                    if let Some(table_state) = table_states.get_mut(analyzer_index)
                                        && let Some(selected) = table_state.selected()
                                        && selected < total_rows.saturating_sub(1)
                                    {
                                        table_state.select(Some(selected + 1));
                                        needs_redraw = true;
                                    }
                                }
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            // On summary tab, navigate through days (back in time)
                            if tui_state.selected_tab == 0 {
                                // Limit to 30 days back
                                if tui_state.summary_day_offset < 30 {
                                    tui_state.summary_day_offset += 1;
                                    cached_summary_data = Some(calculate_summary_data(
                                        &filtered_stats,
                                        tui_state.summary_day_offset,
                                    ));
                                    needs_redraw = true;
                                }
                            }
                            // Only handle table navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            else if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(_current_stats) = filtered_stats.get(analyzer_index)
                                    && let Some(table_state) = table_states.get_mut(analyzer_index)
                                    && let Some(selected) = table_state.selected()
                                    && selected > 0
                                {
                                    table_state.select(Some(selected.saturating_sub(1)));
                                    needs_redraw = true;
                                }
                            }
                        }
                        KeyCode::Home => {
                            // Only handle navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(table_state) = table_states.get_mut(analyzer_index)
                                {
                                    table_state.select(Some(0));
                                    needs_redraw = true;
                                }
                            }
                        }
                        KeyCode::End => {
                            // Only handle navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(current_stats) = filtered_stats.get(analyzer_index)
                                {
                                    // Calculate visible rows (only showing last 30 days by default)
                                    let total_days = current_stats.daily_stats.len();
                                    let view_limit = 30;
                                    let visible_days = std::cmp::min(view_limit, total_days);
                                    let total_rows = visible_days + 2; // data rows + separator + totals
                                    if let Some(table_state) = table_states.get_mut(analyzer_index)
                                    {
                                        table_state.select(Some(total_rows.saturating_sub(1)));
                                        needs_redraw = true;
                                    }
                                }
                            }
                        }
                        KeyCode::PageDown => {
                            // Only handle navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(current_stats) = filtered_stats.get(analyzer_index)
                                {
                                    // Calculate visible rows (only showing last 30 days by default)
                                    let total_days = current_stats.daily_stats.len();
                                    let view_limit = 30;
                                    let visible_days = std::cmp::min(view_limit, total_days);
                                    let total_rows = visible_days + 2; // data rows + separator + totals
                                    if let Some(table_state) = table_states.get_mut(analyzer_index)
                                        && let Some(selected) = table_state.selected()
                                    {
                                        let new_selected =
                                            (selected + 10).min(total_rows.saturating_sub(1));
                                        table_state.select(Some(new_selected));
                                        needs_redraw = true;
                                    }
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            // Only handle navigation on individual analyzer tabs (selected_tab > 0)
                            // Summary tab is at index 0, so only process for tabs 1+
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                let analyzer_index = tui_state.selected_tab - 1;
                                if analyzer_index < table_states.len()
                                    && let Some(table_state) = table_states.get_mut(analyzer_index)
                                    && let Some(selected) = table_state.selected()
                                {
                                    let new_selected = selected.saturating_sub(10);
                                    table_state.select(Some(new_selected));
                                    needs_redraw = true;
                                }
                            }
                        }
                        // Add pagination controls for loading more historical data
                        KeyCode::Char('+') => {
                            // Load more historical data (go back in time)
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                tui_state.daily_stats_view_start =
                                    tui_state.daily_stats_view_start.saturating_add(30);
                                needs_redraw = true;
                            }
                        }
                        KeyCode::Char('-') => {
                            // Load more recent data (go forward in time)
                            if tui_state.selected_tab > 0
                                && tui_state.selected_tab <= filtered_stats.len()
                            {
                                tui_state.daily_stats_view_start =
                                    tui_state.daily_stats_view_start.saturating_sub(30);
                                needs_redraw = true;
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse_event) => {
                    // Reset activity timer on mouse events
                    last_activity = Instant::now();

                    // Only handle mouse events if mouse mode is enabled
                    if !tui_state.mouse_mode_enabled {
                        continue;
                    }

                    if tui_state.selected_tab == 0 {
                        let mouse_y = mouse_event.row;

                        let today_by_cli_rect =
                            tui_state.layout.today_by_cli_rect.unwrap_or_default();
                        let live_activity_rect =
                            tui_state.layout.live_activity_rect.unwrap_or_default();

                        match mouse_event.kind {
                            crossterm::event::MouseEventKind::ScrollUp => {
                                if mouse_y >= live_activity_rect.y
                                    && mouse_y < live_activity_rect.y + live_activity_rect.height
                                {
                                    if tui_state.cli_scroll_offset > 0 {
                                        tui_state.cli_scroll_offset -= 1;
                                        needs_redraw = true;
                                    }
                                } else if mouse_y >= today_by_cli_rect.y
                                    && mouse_y < today_by_cli_rect.y + today_by_cli_rect.height
                                {
                                    if tui_state.cli_table_scroll_offset > 0 {
                                        tui_state.cli_table_scroll_offset -= 1;
                                        needs_redraw = true;
                                    }
                                }
                            }
                            crossterm::event::MouseEventKind::ScrollDown => {
                                if mouse_y >= live_activity_rect.y
                                    && mouse_y < live_activity_rect.y + live_activity_rect.height
                                {
                                    if tui_state.cli_scroll_offset
                                        < filtered_stats.len().saturating_sub(1)
                                    {
                                        tui_state.cli_scroll_offset += 1;
                                        needs_redraw = true;
                                    }
                                } else if mouse_y >= today_by_cli_rect.y
                                    && mouse_y < today_by_cli_rect.y + today_by_cli_rect.height
                                {
                                    // Arbitrary limit, can be improved
                                    if tui_state.cli_table_scroll_offset
                                        < filtered_stats.len().saturating_sub(1)
                                    {
                                        tui_state.cli_table_scroll_offset += 1;
                                        needs_redraw = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Resize(_, _) => {
                    // Terminal was resized, trigger redraw
                    needs_redraw = true;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Redraw just the clock/title line without re-rendering the entire UI.
/// Returns true if a partial redraw was performed.
fn redraw_clock_only(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    tui_state: &TuiState,
) -> anyhow::Result<bool> {
    // Only available on the summary tab where the clock is displayed
    if tui_state.selected_tab != 0 {
        return Ok(false);
    }

    let Some(cache) = &tui_state.clock_title_cache else {
        // Cache not initialized yet (likely before first full render)
        return Ok(false);
    };

    let clock_text = chrono::Local::now().format("%H:%M:%S").to_string();

    let title_spans = vec![
        Span::raw(format!("📊 Activity: {clock_text} | {}", cache.tks_display)),
        Span::raw(cache.spacer.clone()),
        Span::styled(
            cache.countdown_section.clone(),
            Style::default().fg(cache.countdown_color),
        ),
        Span::raw(" | "),
        Span::raw(cache.last_section.clone()),
        Span::raw("     "),
        Span::styled(
            cache.slack_icon.clone(),
            Style::default().fg(cache.slack_color),
        ),
        Span::raw(" Slack"),
    ];

    let mut buf = Buffer::empty(cache.area);
    let paragraph = Paragraph::new(Line::from(title_spans))
        .style(Style::default().bold().fg(Color::Cyan));

    paragraph.render(cache.area, &mut buf);
    let area = cache.area;
    let cells = buf
        .content()
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let x = area.x + (i as u16 % area.width);
            let y = area.y + (i as u16 / area.width);
            (x, y, cell)
        });
    terminal.backend_mut().draw(cells)?;
    Ok(true)
}

/// `p` on the Summary tab: each account's week or month allowance, ordered by how much is about
/// to go unused: what is left (tokens and dollars, measured from this window's own spend), the
/// output tk/s needed to use it all by the reset against the current rate, when a fast pace runs
/// out and for how long the account would then be locked out, and the 5-hour window as a switch
/// signal. The same answers as `splitrail quotas` and the MCP server.
/// Personal bests (`records`), with where each stands now and the date it was set.
fn draw_records_view(frame: &mut Frame, area: Rect, filtered_stats: &[&AgenticCodingToolStats]) {
    use crate::records::Kind;
    let goal = crate::config::Config::load().ok().flatten().map(|c| c.goal.monthly_tks).unwrap_or(150.0);
    let board = crate::records::cached().unwrap_or_else(|| crate::records::board(filtered_stats, goal));
    let dim = Style::default().fg(Color::DarkGray);
    let rows: Vec<Row> = Kind::ALL
        .iter()
        .map(|&kind| {
            let best = board.best.get(&kind);
            let now = board.now.get(&kind).copied().unwrap_or(0.0);
            // How close now is to the record: gold when it is the record (or ties it).
            let share = best.map(|b| now / b.value).unwrap_or(0.0);
            let now_style = if share >= 0.999 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if share >= 0.75 {
                Style::default().fg(Color::Green)
            } else {
                dim
            };
            let now_text = if now > 0.0 { format!("{} ({:.0}%)", kind.format(now), share * 100.0) } else { "-".into() };
            Row::new(vec![
                Cell::from(kind.label()).style(Style::default().add_modifier(Modifier::BOLD)),
                Cell::from(best.map(|b| kind.format(b.value)).unwrap_or_else(|| "-".into()))
                    .style(Style::default().fg(Color::Yellow)),
                Cell::from(best.map(|b| b.when.clone()).unwrap_or_default()),
                Cell::from(now_text).style(now_style),
            ])
        })
        .collect();
    let [_, table_area, plans_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(13), Constraint::Min(4)]).areas(area);
    let table = Table::new(
        rows,
        [Constraint::Length(23), Constraint::Length(12), Constraint::Length(62), Constraint::Min(16)],
    )
    .header(
        Row::new(vec!["Record", "Best", "Set", "Now (of best)"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(1),
    )
    .column_spacing(2)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " 🏆 Records · every tool and machine · streak = days averaging ≥{goal:.0} tk/s · kept in records.json · r: back "
    )));
    frame.render_widget(table, table_area);
    draw_plan_closes(frame, plans_area);
}

/// How full each plan's past weeks and months closed, per account (`records::plan_windows`).
fn draw_plan_closes(frame: &mut Frame, area: Rect) {
    let now = chrono::Utc::now().timestamp();
    let windows = crate::records::plan_windows();
    let mut groups: Vec<(String, String)> = Vec::new();
    for w in &windows {
        let key = (w.account.clone(), w.window.clone());
        if !groups.contains(&key) {
            groups.push(key);
        }
    }
    let dim = Style::default().fg(Color::DarkGray);
    let fullness = |peak: f64| {
        Style::default().fg(if peak >= 95.0 {
            Color::Green
        } else if peak >= 80.0 {
            Color::Yellow
        } else {
            Color::Red
        })
    };
    let date = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%b %-d").to_string())
            .unwrap_or_default()
    };
    let rows: Vec<Row> = groups
        .iter()
        .map(|(account, window)| {
            let own: Vec<&crate::records::PlanWindow> =
                windows.iter().filter(|w| &w.account == account && &w.window == window).collect();
            let closed: Vec<&&crate::records::PlanWindow> = own.iter().filter(|w| w.resets_at <= now).collect();
            let open = own.iter().find(|w| w.resets_at > now);
            let tool = own.last().map(|w| w.tool.clone()).unwrap_or_default();
            let last = closed.last().map(|w| {
                Cell::from(format!("{:.0}% ({})", w.peak, date(w.resets_at))).style(fullness(w.peak))
            });
            let best = closed.iter().max_by(|a, b| a.peak.total_cmp(&b.peak)).map(|w| {
                Cell::from(format!("{:.0}% ({})", w.peak, date(w.resets_at))).style(fullness(w.peak))
            });
            let recent: Vec<f64> = closed.iter().rev().take(4).map(|w| w.peak).collect();
            let average = (!recent.is_empty()).then(|| {
                let avg = recent.iter().sum::<f64>() / recent.len() as f64;
                Cell::from(format!("{avg:.0}% (last {})", recent.len())).style(fullness(avg))
            });
            let now_cell = open.map(|w| Cell::from(format!("{:.0}% → {}", w.peak, date(w.resets_at))));
            Row::new(vec![
                Cell::from(tool).style(Style::default().add_modifier(Modifier::BOLD)),
                Cell::from(account.replace(" (personal)", "")).style(Style::default().fg(Color::Cyan)),
                Cell::from(window.clone()),
                last.unwrap_or_else(|| Cell::from("-").style(dim)),
                best.unwrap_or_else(|| Cell::from("-").style(dim)),
                average.unwrap_or_else(|| Cell::from("-").style(dim)),
                now_cell.unwrap_or_else(|| Cell::from("-").style(dim)),
            ])
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📅 Plan closes · how full each week/month was when it reset (highest figure seen) · kept in plan_closes.json ");
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("No plan windows seen yet; the first close is recorded at its reset.").dim().block(block), area);
        return;
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(24),
            Constraint::Length(6),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(14),
        ],
    )
    .header(
        Row::new(vec!["Tool", "Account", "Plan", "Last closed", "Best", "Average", "This one so far"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(1),
    )
    .column_spacing(1)
    .block(block);
    frame.render_widget(table, area);
}

fn draw_quotas_view(frame: &mut Frame, area: Rect, filtered_stats: &[&AgenticCodingToolStats]) {
    use crate::advisor::{human_secs, human_tokens};
    let status = crate::advisor::status_for_tools(filtered_stats);
    let now = chrono::Utc::now();
    let windows = crate::advisor::windows(&status, now);
    let picks = crate::advisor::where_to_spend(&windows);
    // Most quota about to be wasted first, as the ranking below.
    let plans = crate::advisor::plans_by_waste(&windows);

    let dim = Style::default().fg(Color::DarkGray);
    let used_color = |used: f64| {
        if used >= 90.0 {
            Color::Red
        } else if used >= 70.0 {
            Color::Yellow
        } else {
            Color::Green
        }
    };
    let yardstick = max_week_value(windows.iter());
    let rows: Vec<Row> = plans
        .iter()
        .map(|row| {
            let w = &row.plan;
            // The whole window's worth, `~` where estimated from the plan rather than measured.
            let value = plan_value(w, yardstick);
            // Tokens and dollars in aligned columns, `~` where estimated.
            let amount = |dollars: f64| {
                format!(
                    "{}{:>6} {:>6}",
                    if value.estimated { "~" } else { " " },
                    value.tokens_per_dollar.map_or("?".into(), |t| human_tokens(dollars * t)),
                    format!("${dollars:.0}")
                )
            };
            let allowance = amount(value.allowance);
            // `ambertyeung@gmail.com (personal)` -> the email; a team name stays.
            let account = w.account.clone().map(|a| a.replace(" (personal)", "")).unwrap_or_else(|| "-".into());
            let left = amount(value.left);
            // Needed rate against the current one: green when keeping up, yellow when behind.
            let need = w.need_tks().map(|need| {
                let keeping_up = w.now_tks >= need;
                Cell::from(format!("{need:.0} tk/s"))
                    .style(Style::default().fg(if keeping_up { Color::Green } else { Color::Yellow }))
            });
            let runs_out = w.runs_out_in_secs().map(human_secs);
            let locked = w.locked_out_secs().map(human_secs);
            let resets = match (w.resets_in_secs, w.resets_at) {
                (Some(secs), Some(at)) => format!(
                    "{} ({})",
                    human_secs(secs),
                    at.with_timezone(&chrono::Local)
                        .format(if secs < 6 * 86_400 { "%a %H:%M" } else { "%b %-d" })
                ),
                _ => "?".into(),
            };
            // The 5-hour window only says when to switch: yellow from 80%, red when full.
            let five = row.five_hour.as_ref().map(|f| {
                let refill = f.resets_in_secs.map(human_secs).unwrap_or_else(|| "?".into());
                let color = if f.used >= 95.0 {
                    Color::Red
                } else if f.used >= crate::advisor::SWITCH_SOON {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };
                Cell::from(format!("{:>3.0}% · {refill}", f.used)).style(Style::default().fg(color))
            });
            Row::new(vec![
                Cell::from(w.tool.clone()),
                Cell::from(account).style(Style::default().fg(if w.account.is_some() { Color::Cyan } else { Color::DarkGray })),
                Cell::from(w.window),
                Cell::from(format!("{:>3.0}%", w.used)).style(Style::default().fg(used_color(w.used))),
                Cell::from(allowance).style(if value.estimated { dim } else { Style::default() }),
                Cell::from(left),
                need.unwrap_or_else(|| Cell::from("-").style(dim)),
                Cell::from(if w.now_tks > 0.0 { format!("{:.0} tk/s", w.now_tks) } else { "-".into() }),
                Cell::from(runs_out.clone().unwrap_or_else(|| "-".into()))
                    .style(if runs_out.is_some() { Style::default().fg(Color::Red) } else { dim }),
                Cell::from(locked.clone().unwrap_or_else(|| "-".into()))
                    .style(if locked.is_some() { Style::default().fg(Color::Red) } else { dim }),
                Cell::from(resets),
                five.unwrap_or_else(|| Cell::from("-").style(dim)),
            ])
        })
        .collect();
    let table_height = (rows.len() as u16 + 4).min(area.height.saturating_sub(5).max(3));
    // A blank line under the tabs, as the other views have.
    let [_, table_area, picks_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(table_height), Constraint::Min(3)]).areas(area);
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(19),
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Min(11),
        ],
    )
    .header(
        Row::new(vec![
            "Tool", "Account", "Plan", "Used", "Allowance", "Left", "Need", "Now", "Runs out", "Locked out", "Resets", "5h now",
        ])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .bottom_margin(1),
    )
    .column_spacing(1)
    .block(Block::default().borders(Borders::ALL).title(
        " ⏱ Plan quotas: week/month allowance, most about to be wasted first · Need = tk/s to use it all by the reset · 5h now = when to switch · p: back ",
    ));
    frame.render_widget(table, table_area);

    // One line per tool, a word for what to do and phrases for why, most to gain first.
    let pick_rows: Vec<Row> = picks
        .iter()
        .enumerate()
        .map(|(i, pick)| {
            let action = pick.action();
            let action_color = match action {
                "USE NOW" => Color::Green,
                "USE" => Color::LightGreen,
                "EASE OFF" => Color::Yellow,
                "WAIT" => Color::Red,
                _ => Color::DarkGray,
            };
            let w = pick.window.as_ref();
            let wasted = w.filter(|w| w.expiring() >= 1.0).map(|w| match w.left_dollars() {
                Some(d) if w.projected.is_some() => format!("{:.0}% · ${:.0}", w.expiring(), d * w.expiring() / (100.0 - w.used).max(1.0)),
                _ => format!("{:.0}%", w.expiring()),
            });
            let need = w.and_then(|w| w.need_tks()).filter(|_| pick.available);
            let now_tks = w.map(|w| w.now_tks).unwrap_or(0.0);
            let need_cell = match need {
                Some(n) => Cell::from(format!("{n:.0} tk/s"))
                    .style(Style::default().fg(if now_tks >= n { Color::Green } else { Color::Yellow })),
                None => Cell::from("-").style(dim),
            };
            let resets = w.and_then(|w| w.resets_in_secs).map(human_secs).unwrap_or_else(|| "?".into());
            let style = if pick.available { Style::default() } else { dim };
            Row::new(vec![
                Cell::from(format!("{}", i + 1)).style(dim),
                Cell::from(action).style(Style::default().fg(action_color).add_modifier(Modifier::BOLD)),
                Cell::from(pick.tool.clone()).style(style.add_modifier(Modifier::BOLD)),
                Cell::from(wasted.unwrap_or_else(|| "-".into())).style(style),
                need_cell,
                Cell::from(if now_tks > 0.0 { format!("{now_tks:.0} tk/s") } else { "-".into() }).style(style),
                Cell::from(resets).style(style),
                Cell::from(pick.why()).style(dim),
            ])
        })
        .collect();
    let block = Block::default().borders(Borders::ALL).title(" Where to spend next · top first ");
    if pick_rows.is_empty() {
        frame.render_widget(Paragraph::new("No plan limits known yet.").dim().block(block), picks_area);
        return;
    }
    let picks_table = Table::new(
        pick_rows,
        [
            Constraint::Length(2),
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec!["#", "Do", "Tool", "Wasted", "Need", "Now", "Resets", "Why"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(1),
    )
    .column_spacing(1)
    .block(block);
    // The list, then the timeline of the same plans below it when there is room.
    let list_height = (picks.len() as u16 + 4).min(picks_area.height);
    let [list_area, timeline_area] =
        Layout::vertical([Constraint::Length(list_height), Constraint::Min(0)]).areas(picks_area);
    frame.render_widget(picks_table, list_area);
    let yardstick = max_week_value(windows.iter());
    // The pool chart under the timeline, when there is room for both.
    let timeline_rows = picks.iter().filter(|p| p.window.as_ref().is_some_and(|w| w.resets_in_secs.is_some())).count() as u16;
    // A row per plan, a second for the few big ones, the axis, marks, legend and borders; the
    // pool chart takes the rest.
    let timeline_height = (timeline_rows + 7).min(timeline_area.height);
    let [timeline_area, pool_area] =
        Layout::vertical([Constraint::Length(timeline_height), Constraint::Min(0)]).areas(timeline_area);
    if timeline_area.height >= 5 {
        draw_plan_timeline(frame, timeline_area, &picks, filtered_stats, yardstick, now);
    }
    if pool_area.height >= 8 {
        draw_pool_chart(frame, pool_area, &picks, filtered_stats, yardstick, now);
    }
}

/// Every plan's allowance as one pool, like a stock chart: the API dollars left across all plans
/// over the last day (rebuilt from the hourly usage) and the next week. Ahead, two lines: the
/// pool if left unused (each plan refills at its reset), and at the pace that uses each plan by
/// its reset (the burn-down the timeline asks for). Local GPUs have no allowance to run out, so
/// they are not in it.
fn draw_pool_chart(
    frame: &mut Frame,
    area: Rect,
    picks: &[crate::advisor::Pick],
    stats: &[&AgenticCodingToolStats],
    yardstick: (f64, Option<f64>),
    now: chrono::DateTime<chrono::Utc>,
) {
    use chrono::Duration;
    use ratatui::symbols::Marker;
    use ratatui::widgets::{Axis, Chart, Dataset, GraphType};
    const PAST_H: i64 = 24;
    const AHEAD_H: i64 = 7 * 24;
    struct Plan {
        value: PlanValue,
        resets_in_h: f64,
        period_h: f64,
        spent: Vec<(f64, f64)>, // (hours before now, dollars)
    }
    let since = now - Duration::hours(PAST_H);
    let plans: Vec<Plan> = picks
        .iter()
        .filter_map(|p| {
            let w = p.window.as_ref()?;
            Some(Plan {
                value: plan_value(w, yardstick),
                resets_in_h: w.resets_in_secs? as f64 / 3600.0,
                period_h: crate::advisor::window_secs_label(w.window) / 3600.0,
                spent: plan_spend_events(p, stats, since, now)
                    .into_iter()
                    .map(|(at, cost)| ((now - at).num_seconds() as f64 / 3600.0, cost))
                    .collect(),
            })
        })
        .collect();
    // Past: what is left now plus what has been spent since, capped at the plan's whole value.
    let past: Vec<(f64, f64)> = (0..=PAST_H * 2)
        .map(|half| {
            let ago = (PAST_H * 2 - half) as f64 / 2.0;
            let pool = plans
                .iter()
                .map(|p| (p.value.left + p.spent.iter().filter(|(h, _)| *h <= ago).map(|(_, c)| c).sum::<f64>()).min(p.value.allowance))
                .sum::<f64>();
            (-ago, pool)
        })
        .collect();
    let ahead = |paced: bool| -> Vec<(f64, f64)> {
        (0..=AHEAD_H)
            .map(|h| {
                let h = h as f64;
                let pool = plans
                    .iter()
                    .map(|p| {
                        if h < p.resets_in_h {
                            if paced { p.value.left * (1.0 - h / p.resets_in_h) } else { p.value.left }
                        } else if paced {
                            let into = (h - p.resets_in_h) % p.period_h;
                            p.value.allowance * (1.0 - into / p.period_h)
                        } else {
                            p.value.allowance
                        }
                    })
                    .sum::<f64>();
                (h, pool)
            })
            .collect()
    };
    let idle = ahead(false);
    let paced = ahead(true);
    let top = past.iter().chain(&idle).map(|(_, v)| *v).fold(1.0f64, f64::max) * 1.05;
    let left_now: f64 = plans.iter().map(|p| p.value.left).sum();
    let tokens_now: f64 = plans.iter().filter_map(|p| Some(p.value.left * p.value.tokens_per_dollar?)).sum();
    // The cap: every plan full at once, the sum of the Allowance column.
    let cap: f64 = plans.iter().map(|p| p.value.allowance).sum();
    let cap_tokens: f64 = plans.iter().filter_map(|p| Some(p.value.allowance * p.value.tokens_per_dollar?)).sum();
    let day_ago = past.first().map_or(left_now, |(_, v)| *v);
    let change = left_now - day_ago;
    let datasets = vec![
        Dataset::default()
            .name("last 24h")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&past),
        Dataset::default()
            .name("if unused (refills at each reset)")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::DarkGray))
            .data(&idle),
        Dataset::default()
            .name("using every plan by its reset")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Green))
            .data(&paced),
    ];
    let money = |v: f64| if v >= 1000.0 { format!("${:.1}k", v / 1000.0) } else { format!("${v:.0}") };
    let title = Line::from(vec![
        Span::styled(" 📈 Pool · every plan's allowance, API $ · ", Style::default().bold()),
        Span::styled(format!("{} left", money(left_now)), Style::default().fg(Color::Cyan).bold()),
        Span::styled(
            format!(" of {} cap ({:.0}%) ", money(cap), left_now / cap.max(1.0) * 100.0),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("{}{} 24h ", if change >= 0.0 { "▲" } else { "▼" }, money(change.abs())),
            Style::default().fg(if change >= 0.0 { Color::Green } else { Color::Red }),
        ),
        Span::styled(
            format!(
                "· ≈{} of {} output tokens · local GPUs uncapped, not in it ",
                crate::advisor::human_tokens(tokens_now),
                crate::advisor::human_tokens(cap_tokens)
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let chart = Chart::new(datasets)
        .block(
            Block::default().borders(Borders::ALL).title(title).title_bottom(Line::from(vec![
                Span::styled(" ── last 24h ", Style::default().fg(Color::Cyan)),
                Span::styled(" ── if unused (refills at each reset) ", Style::default().fg(Color::DarkGray)),
                Span::styled(" ── using every plan by its reset ", Style::default().fg(Color::Green)),
            ])),
        )
        .x_axis(
            Axis::default()
                .bounds([-(PAST_H as f64), AHEAD_H as f64])
                // Labels are spread evenly over the bounds: one a day, "now" the second.
                .labels(
                    std::iter::once("-24h".to_string())
                        .chain(std::iter::once("now".to_string()))
                        .chain((1..=7).map(|d| format!("+{d}d")))
                        .map(Span::raw)
                        .collect::<Vec<_>>(),
                )
                .style(Style::default().fg(Color::DarkGray)),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, top])
                .labels(vec![Span::raw("$0"), Span::raw(money(top / 2.0)), Span::raw(money(top))])
                .style(Style::default().fg(Color::DarkGray)),
        )
        .legend_position(None);
    frame.render_widget(chart, area);
}

/// Colour for the share of a plan left: plenty to use (green), some (yellow), nearly out (red).
fn left_color(left: f64) -> Color {
    if left >= 50.0 {
        Color::Green
    } else if left >= 20.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// What a plan's whole window is worth and what is left of it, in API dollars, so plans of every
/// kind compare. Measured from this window's spend where there is some; else estimated from the
/// plan (a Claude plan as a share of the measured Max 20x week, a credit plan's monthly
/// allowance). A Claude plan is always sized by its plan: usage per login has only been credited
/// since 2026-10-23, so a login's measured week can be a sliver of it.
#[derive(Clone, Copy)]
struct PlanValue {
    allowance: f64,
    left: f64,
    /// Output tokens a dollar buys at this plan's mix of models and cache.
    tokens_per_dollar: Option<f64>,
    estimated: bool,
}

/// The measured value of a Max 20x week, the yardstick for estimating other Claude plans.
fn max_week_value<'a>(windows: impl Iterator<Item = &'a crate::advisor::Window>) -> (f64, Option<f64>) {
    windows
        .filter(|w| w.plan.as_deref() == Some("Max 20x") && w.window == "week" && w.used >= 1.0)
        .filter_map(|w| {
            let spend = w.spend?;
            let value = w.dollars_per_percent()? * 100.0;
            Some((value, (spend.cost > 0.0).then(|| spend.output / spend.cost)))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap_or((2_000.0, None))
}

fn plan_value(w: &crate::advisor::Window, (max_week, max_tpd): (f64, Option<f64>)) -> PlanValue {
    let left_share = (100.0 - w.used).clamp(0.0, 100.0) / 100.0;
    let measured_tpd = w.spend.filter(|s| s.cost > 0.0 && s.output > 0.0).map(|s| s.output / s.cost);
    let claude_plan = w.plan.as_deref().is_some_and(|p| p.starts_with("Max") || p.starts_with("Team") || p == "Pro");
    if !claude_plan && let Some(per) = w.dollars_per_percent().filter(|_| w.used >= 1.0) {
        return PlanValue { allowance: per * 100.0, left: per * (100.0 - w.used).max(0.0), tokens_per_dollar: measured_tpd, estimated: false };
    }
    let allowance = match w.plan.as_deref() {
        Some("Max 20x") => max_week,
        Some("Max 5x") | Some("Max") => max_week * 0.25,
        Some("Team Prem") => max_week * 6.25 / 20.0,
        Some("Team Std") | Some("Team") => max_week * 1.25 / 20.0,
        Some("Pro") => max_week / 20.0,
        _ => crate::plan_limits::plan_value_dollars(tool_base(&w.tool)).unwrap_or(20.0),
    };
    let estimated = !(w.plan.as_deref() == Some("Max 20x") && (allowance - w.dollars_per_percent().unwrap_or(0.0) * 100.0).abs() < 1.0);
    PlanValue { allowance, left: allowance * left_share, tokens_per_dollar: measured_tpd.or(max_tpd), estimated }
}

/// API dollars spent on a plan's own usage (a Claude login's slices) between `since` and `now`,
/// by hour.
fn plan_spend_events(
    pick: &crate::advisor::Pick,
    stats: &[&AgenticCodingToolStats],
    since: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<(chrono::DateTime<chrono::Utc>, f64)> {
    use chrono::Local;
    let base = tool_base(&pick.tool);
    let Some(tool) = stats.iter().find(|s| s.analyzer_name == base) else { return Vec::new() };
    let login = pick.account.as_deref();
    let signed_in = !pick.window.as_ref().is_some_and(|w| w.switch_login);
    let local = |day: &str, hour: u8| {
        chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(hour as u32, 30, 0)?
            .and_local_timezone(Local)
            .earliest()
            .map(|t| t.with_timezone(&chrono::Utc))
    };
    let first_day = since.with_timezone(&Local).format("%Y-%m-%d").to_string();
    let mut events = Vec::new();
    for (day, totals) in tool.daily_stats.range(first_day..) {
        for s in &totals.slices {
            let mine = match login {
                Some(l) if !s.account.is_empty() || !signed_in => s.account == l,
                _ => true,
            };
            let Some(at) = local(day, s.hour) else { continue };
            if mine && at >= since && at <= now && s.cost > 0.0 {
                events.push((at, s.cost));
            }
        }
    }
    events
}

/// The plans on one shared axis, the last day and the next week: each bar's height is what is
/// left of the plan, in dollars at API prices so plans of every kind compare (a Max 20x is worth
/// thousands a week, Kiro Free a dollar a month). Left of "now", what was left each hour of the
/// last day, rebuilt from the hourly usage; right of it, the taper to the reset at the pace that
/// uses it all, so the aim is to drive every bar down to a single dot by its deadline. Coloured
/// by the share left; grey while a plan is full and waiting for its reset. Braille dots, like
/// the Summary's sparklines, soonest reset first.
fn draw_plan_timeline(
    frame: &mut Frame,
    area: Rect,
    picks: &[crate::advisor::Pick],
    stats: &[&AgenticCodingToolStats],
    yardstick: (f64, Option<f64>),
    now: chrono::DateTime<chrono::Utc>,
) {
    use chrono::{Datelike, Duration, Local, Timelike};
    const PAST_SECS: f64 = 86_400.0;
    const AHEAD_SECS: f64 = 7.0 * 86_400.0;
    const NAME_W: usize = 26;
    const PLAN_W: usize = 9;
    // Braille rows from the bottom: one to four dots high.
    const LEVELS: [char; 5] = [' ', '⣀', '⣤', '⣶', '⣿'];
    let block = Block::default()
        .title(" 🗓  Plans · last day and next week · height = what is left ")
        .title_style(Style::default().bold())
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let label_w = 36usize;
    let width = (inner.width as usize).saturating_sub(NAME_W - 2 + PLAN_W + label_w + 2).max(16);
    let past_w = (width as f64 * PAST_SECS / (PAST_SECS + AHEAD_SECS)).round().max(3.0) as usize;
    let ahead_w = width - past_w - 1; // One column for "now".
    let col_secs_past = PAST_SECS / past_w as f64;

    let mut rows: Vec<(&crate::advisor::Pick, &crate::advisor::Window, i64)> = picks
        .iter()
        .filter_map(|p| {
            let w = p.window.as_ref()?;
            Some((p, w, w.resets_in_secs?))
        })
        .collect();
    rows.sort_by_key(|(_, _, secs)| *secs);

    // Spend in each column of the last day, for the plan's own usage.
    let past_spend = |pick: &crate::advisor::Pick| -> Vec<f64> {
        let mut spent = vec![0.0; past_w];
        let since = now - Duration::seconds(PAST_SECS as i64);
        for (at, cost) in plan_spend_events(pick, stats, since, now) {
            let col = (((at - since).num_seconds() as f64) / col_secs_past) as usize;
            if col < past_w {
                spent[col] += cost;
            }
        }
        spent
    };
    // Height in dot rows: 1-4 in one text row, 1-8 for a big plan drawn two rows tall.
    let dots = |dollars: f64, biggest: f64, rows: usize| ((dollars / biggest * (4 * rows) as f64).ceil() as usize).clamp(1, 4 * rows);

    let measured: Vec<(f64, Vec<f64>)> = rows.iter().map(|(p, w, _)| (plan_value(w, yardstick).left, past_spend(p))).collect();
    // The tallest over the whole picture, the past included, sets the scale.
    let biggest = measured
        .iter()
        .map(|(now_left, spent)| now_left + spent.iter().sum::<f64>())
        .fold(1.0f64, f64::max);

    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();
    // Axis: "-24h", "now", then each local midnight ahead by weekday.
    let mut axis = vec![' '; width];
    let mut marks = vec![' '; width];
    for (i, ch) in "-24h".chars().enumerate() {
        axis[i] = ch;
    }
    marks[past_w] = '┃';
    let local_now = now.with_timezone(&Local);
    let mut day = local_now.date_naive().succ_opt();
    while let Some(d) = day {
        let Some(midnight) = d.and_hms_opt(0, 0, 0).and_then(|m| m.and_local_timezone(Local).earliest()) else { break };
        let offset = (midnight.with_timezone(&chrono::Utc) - now).num_seconds() as f64;
        if offset >= AHEAD_SECS {
            break;
        }
        let col = past_w + 1 + (offset / AHEAD_SECS * ahead_w as f64) as usize;
        if col < width {
            marks[col] = '┊';
            for (i, ch) in d.weekday().to_string().chars().take(3).enumerate() {
                if col + 1 + i < width {
                    axis[col + 1 + i] = ch;
                }
            }
        }
        day = d.succ_opt();
    }
    // "now hh:mm" ends just before the now line, clear of the day names after it.
    let now_label = format!("now {:02}:{:02}", local_now.hour(), local_now.minute());
    let label_start = past_w.saturating_sub(now_label.len());
    if label_start > 4 {
        for (i, ch) in now_label.chars().enumerate() {
            axis[label_start + i] = ch;
        }
    }
    lines.push(Line::from(vec![
        Span::styled(format!("{:<w$}", "", w = NAME_W - 2 + PLAN_W), dim),
        Span::styled(axis.iter().collect::<String>(), Style::default().fg(Color::Cyan)),
    ]));
    for ((pick, w, secs), (left_now, spent)) in rows.iter().zip(&measured).take(inner.height.saturating_sub(2) as usize) {
        let left_pct = (100.0 - w.used).clamp(0.0, 100.0);
        let color = if pick.available { left_color(left_pct) } else { Color::DarkGray };
        // A plan with a quarter or more of the biggest is drawn two rows tall, for finer steps.
        let tall = left_now + spent.iter().sum::<f64>() >= biggest / 4.0;
        let text_rows = if tall { 2 } else { 1 };
        // The past: what was left at each column, from what has been spent since.
        let mut after: f64 = spent.iter().sum();
        let past_dots: Vec<usize> = spent
            .iter()
            .map(|cost| {
                after -= cost;
                dots(left_now + after + cost, biggest, text_rows)
            })
            .collect();
        // Ahead: tapering to the reset along the pace that uses it all.
        let beyond = (*secs as f64) > AHEAD_SECS;
        let filled = ((*secs as f64).min(AHEAD_SECS) / AHEAD_SECS * ahead_w as f64).round().max(1.0) as usize;
        let filled = if beyond { ahead_w.saturating_sub(1) } else { filled.min(ahead_w) };
        let ahead_dots: Vec<usize> = (0..filled)
            .map(|col| {
                if !pick.available {
                    return 1;
                }
                dots(left_now * (1.0 - col as f64 / filled.max(1) as f64), biggest, text_rows)
            })
            .collect();
        // Split dot heights into text rows: the lower row fills first.
        let row_of = |heights: &[usize], row: usize| -> String {
            heights.iter().map(|h| LEVELS[h.saturating_sub(4 * row).min(4)]).collect()
        };
        let past = row_of(&past_dots, 0);
        let ahead = row_of(&ahead_dots, 0);
        let rest: String = (past_w + 1 + filled + 1..past_w + 1 + ahead_w + 1)
            .map(|i| if marks.get(i) == Some(&'┊') { '┊' } else { ' ' })
            .collect();
        let reset_local = (now + Duration::seconds(*secs)).with_timezone(&Local);
        let action = pick.action();
        let action_color = match action {
            "USE NOW" => Color::Green,
            "USE" => Color::LightGreen,
            "EASE OFF" => Color::Yellow,
            "WAIT" => Color::Red,
            _ => Color::DarkGray,
        };
        let tool = match pick.account.as_deref() {
            Some(account) if pick.tool.starts_with("Claude (") && !pick.tool.contains('·') && !pick.tool.contains(',') => {
                format!("{} · {}", pick.tool, crate::advisor::short_login(account))
            }
            _ => pick.tool.clone(),
        };
        let name: String = tool.chars().take(NAME_W - 3).collect();
        if tall {
            // The upper row of a two-row bar, above the labelled one.
            let blank_rest: String = (past_w + 1 + filled..past_w + 1 + ahead_w + 1)
                .map(|i| if marks.get(i) == Some(&'┊') { '┊' } else { ' ' })
                .collect();
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(NAME_W - 2 + PLAN_W)),
                Span::styled(row_of(&past_dots, 1), Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)),
                Span::styled("┃", Style::default().fg(Color::Cyan)),
                Span::styled(row_of(&ahead_dots, 1), Style::default().fg(color)),
                Span::styled(blank_rest, dim),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<w$}", w = NAME_W - 2), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<PLAN_W$}", w.plan.as_deref().unwrap_or("")), Style::default().fg(Color::Magenta)),
            Span::styled(past, Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)),
            Span::styled("┃", Style::default().fg(Color::Cyan)),
            Span::styled(ahead, Style::default().fg(color)),
            // Box drawing and » are one cell in every terminal font; ▏ was two in some.
            Span::styled(if beyond { "»" } else { "│" }, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(rest, dim),
            Span::styled(format!(" {left_pct:>3.0}% left"), Style::default().fg(color)),
            Span::styled(
                format!(
                    " · {:<9} · ",
                    if beyond {
                        reset_local.format("%b %-d").to_string()
                    } else {
                        format!("{} {:02}:{:02}", &reset_local.weekday().to_string()[..3], reset_local.hour(), reset_local.minute())
                    }
                ),
                dim,
            ),
            Span::styled(action, Style::default().fg(action_color).add_modifier(Modifier::BOLD)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(format!("{:<w$}", "", w = NAME_W - 2 + PLAN_W), dim),
        Span::styled(
            "height = $ left at API prices (tallest = most) · last day ┃ now · tapers to the reset at the pace that uses it all · │ reset · » after this week",
            dim,
        ),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The overall output rate from the status file (written every second), read at most once a
/// second.
fn overall_tks() -> Option<f64> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(std::time::Instant, Option<f64>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().ok()?;
    if let Some((at, tks)) = *cache
        && at.elapsed().as_secs_f64() < 1.0
    {
        return tks;
    }
    let tks = crate::status_file::path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.get("tks")?.as_f64());
    *cache = Some((std::time::Instant::now(), tks));
    tks
}

/// The analyzer behind a timeline row: `Claude (v) · a@virti.us` and `Claude (main), dev-a`
/// are `Claude (v)` and `Claude (main)`.
fn tool_base(tool: &str) -> &str {
    tool.split(" · ").next().unwrap_or(tool).split(", ").next().unwrap_or(tool)
}

fn draw_ui(
    frame: &mut Frame,
    filtered_stats: &[&AgenticCodingToolStats],
    format_options: &NumberFormatOptions,
    table_states: &mut [TableState],
    tui_state: &mut TuiState,
    upload_status: Arc<Mutex<UploadStatus>>,
    cached_summary_data: &Option<SummaryData>,
    render_time_utc: chrono::DateTime<chrono::Utc>,
    render_time_system: SystemTime,
) {
    // Since we're already working with filtered stats, has_data is simply whether we have any stats
    let has_data = !filtered_stats.is_empty();

    // Check if we have an error to determine help area height
    let has_error = if let Ok(status) = upload_status.lock() {
        matches!(*status, UploadStatus::Failed(_))
    } else {
        false
    };

    // Adjust layout based on whether we have data or not
    let chunks = if has_data {
        // For Summary view (tui_state.selected_tab == 0), give more space to main table since no totals shown
        let summary_stats_height = if tui_state.selected_tab == 0 { 0 } else { 9 };
        let has_status_message = tui_state.status_message.is_some();
        Layout::vertical([
            Constraint::Length(3),                             // Header
            Constraint::Length(1),                             // Tabs
            Constraint::Min(3),                                // Main table
            Constraint::Length(summary_stats_height),          // Summary stats (0 for Summary tab)
            Constraint::Length(if has_error { 3 } else { 1 }), // Help text
            Constraint::Length(if has_status_message { 1 } else { 0 }), // Status bar
        ])
        .split(frame.area())
    } else {
        Layout::vertical([
            Constraint::Length(3), // Header
            Constraint::Min(3),    // No-data message
            Constraint::Length(1), // Help text
        ])
        .split(frame.area())
    };

    // Header
    let version = env!("CARGO_PKG_VERSION");
    let perf_text = if tui_state.perf_overlay.enabled {
        if let Some(snapshot) = tui_state.perf_overlay.last_snapshot {
            // Fixed-width fields so the header doesn't jump around
            let fps = format!("{:>4.1}", snapshot.fps.min(999.9));
            let render_ms = format!("{:>5.1}", snapshot.render_ms.min(9999.9));
            let cpu = snapshot
                .cpu_pct
                .map(|c| format!("{:>5.1}", c.min(999.9)))
                .unwrap_or_else(|| " --.-".to_string());
            Some(format!("perf {}fps {}ms cpu:{}%", fps, render_ms, cpu))
        } else {
            Some("perf ----".to_string())
        }
    } else {
        None
    };

    let mut header_line = version.to_string();
    if let Some(perf_text) = perf_text {
        let width = chunks[0].width as usize;
        if header_line.len() + perf_text.len() + 1 < width {
            let pad = width.saturating_sub(header_line.len() + perf_text.len());
            header_line.push_str(&" ".repeat(pad));
            header_line.push_str(&perf_text);
        } else {
            header_line.push(' ');
            header_line.push_str(&perf_text);
        }
    }

    // The update state goes beside the version, the perf readout (if on) after it.
    let mut header_spans = vec![Span::styled(version.to_string(), Style::new().cyan().bold())];
    // Output rate across every tool and machine, on every view (the menu bar's number).
    if let Some(tks) = overall_tks() {
        header_spans.push(Span::styled(format!("  ⚡ {tks:.0} tk/s"), Style::new().yellow().bold()));
    }
    header_spans.extend(crate::tui::summary_dashboard::update_spans());
    if header_line.len() > version.len() {
        header_spans.push(Span::styled(
            header_line[version.len()..].to_string(),
            Style::new().cyan().bold(),
        ));
    }
    let header = Paragraph::new(Text::from(vec![
        Line::from(header_spans),
        Line::styled("=".repeat(version.len()), Style::new().cyan().bold()),
    ]));
    frame.render_widget(header, chunks[0]);

    if has_data {
        // Create tab titles with Summary as first tab
        let mut tab_titles: Vec<Line> = vec![Line::from(" 📊 Summary ")];
        tab_titles.extend(filtered_stats.iter().map(|stats| {
            Line::from(format!(
                " {} ({}) ",
                stats.analyzer_name, stats.num_conversations,
            ))
        }));

        let tabs = Tabs::new(tab_titles)
            .select(tui_state.selected_tab)
            // .style(Style::default().add_modifier(Modifier::DIM))
            .highlight_style(Style::new().black().on_light_green())
            .padding("", "")
            .divider(" | ");

        frame.render_widget(tabs, chunks[1]);

        if tui_state.selected_tab == 0 && tui_state.records_view {
            draw_records_view(frame, chunks[2], filtered_stats);
            draw_summary_stats(frame, chunks[3], filtered_stats, format_options, false);
        } else if tui_state.selected_tab == 0 && tui_state.quotas_view {
            draw_quotas_view(frame, chunks[2], filtered_stats);
            draw_summary_stats(frame, chunks[3], filtered_stats, format_options, false);
        } else if tui_state.selected_tab == 0 {
            // Summary view - show cached aggregated data across all analyzers
            if let Some(summary_data) = &cached_summary_data {
                draw_summary_view(
                    frame,
                    chunks[2],
                    summary_data,
                    format_options,
                    filtered_stats,
                    tui_state,
                    render_time_utc,
                    render_time_system,
                );
                draw_summary_stats(frame, chunks[3], filtered_stats, format_options, false);
            }
        } else {
            // Individual analyzer view (adjust index since Summary is tab 0)
            let analyzer_index = tui_state.selected_tab - 1;
            if let Some(current_stats) = filtered_stats.get(analyzer_index)
                && let Some(current_table_state) = table_states.get_mut(analyzer_index)
            {
                // Debug: ensure table state is initialized for this analyzer
                if current_table_state.selected().is_none() {
                    current_table_state.select(Some(0));
                }
                // Main table
                let grouped;
                let table_stats: &AgenticCodingToolStats =
                    if tui_state.period == Period::Hour || !tui_state.filter.is_empty() {
                        grouped = group_slices(current_stats, tui_state.period, &tui_state.filter);
                        &grouped
                    } else if tui_state.period == Period::Day {
                        current_stats
                    } else {
                        grouped = group_by_period(current_stats, tui_state.period);
                        &grouped
                    };
                draw_daily_stats_table(
                    frame,
                    chunks[2],
                    table_stats,
                    format_options,
                    current_table_state,
                    tui_state.daily_stats_view_start,
                    tui_state.period,
                );

                // Summary stats - pass all filtered stats for aggregation
                draw_summary_stats(frame, chunks[3], filtered_stats, format_options, true);
            }
        }

        // Help text for data view with upload status
        let help_area = chunks[4];

        // Split help area horizontally: help text on left, upload status on right
        let help_chunks = Layout::horizontal([
            Constraint::Percentage(70), // Give more space to help text
            Constraint::Percentage(30), // Upload status on right
        ])
        .split(help_area);

        let mouse_mode_indicator = if tui_state.mouse_mode_enabled {
            "🖱️ Scroll"
        } else {
            "📝 Select"
        };

        // Get Slack status
        let _slack_status = match crate::config::Config::load() {
            Ok(Some(config)) => {
                if config.notifications.slack.enabled {
                    "☑ Slack"
                } else {
                    "☐ Slack"
                }
            }
            _ => "☐ Slack",
        };

        let help = if tui_state.selected_tab == 0 {
            Paragraph::new(format!(
                "?: all keys, ←/→ or h/l: tabs, ↑/↓ or j/k: days, p: {}, r: {}, v: verbose, d: debug log, s: Slack, m: {} mode, ^L: redraw, U: update, Esc: back, q: quit",
                if tui_state.quotas_view { "summary" } else { "quotas" },
                if tui_state.records_view { "summary" } else { "records" },
                mouse_mode_indicator,
            ))
            .style(Style::default().add_modifier(Modifier::DIM))
        } else {
            Paragraph::new(format!(
                "?: all keys, ←/→ or h/l: tabs, ↑/↓ or j/k: navigate, g: by {} (now {}), f: model, o: project, x: clear{}, +/-: load more data, d: debug log, s: Slack, m: toggle {} mode, ^L: redraw, U: update, Esc: back, q: quit",
                tui_state.period.next().name(),
                tui_state.period.name(),
                if tui_state.filter.is_empty() {
                    String::new()
                } else {
                    format!(" (showing{}, tokens and cost only)", tui_state.filter.describe())
                },
                mouse_mode_indicator,
            ))
            .style(Style::default().add_modifier(Modifier::DIM))
        };
        frame.render_widget(help, help_chunks[0]);

        // Upload status in bottom-right
        if let Ok(status) = upload_status.lock() {
            let (status_text, status_style) = match &*status {
                UploadStatus::None => (String::new(), Style::default()),
                UploadStatus::Uploading {
                    current,
                    total,
                    dots,
                } => {
                    // Always show animated dots - ignore is_counting
                    let dots_str = match dots % 4 {
                        0 => "   ",
                        1 => ".  ",
                        2 => ".. ",
                        _ => "...",
                    };
                    (
                        format!(
                            "Uploading {}/{} messages{}",
                            format_number(*current as u64, format_options),
                            format_number(*total as u64, format_options),
                            dots_str
                        ),
                        Style::default().add_modifier(Modifier::DIM),
                    )
                }
                UploadStatus::Uploaded => (
                    "✓ Uploaded successfully".to_string(),
                    Style::default().fg(Color::Green),
                ),
                UploadStatus::Failed(error) => {
                    // Show full error message - let the widget handle wrapping/display
                    (format!("✕ {error}"), Style::default().fg(Color::Red))
                }
                UploadStatus::MissingApiToken => (
                    "No API token for uploading".to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                UploadStatus::MissingServerUrl => (
                    "No server URL for uploading".to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                UploadStatus::MissingConfig => (
                    "Upload config incomplete".to_string(),
                    Style::default().fg(Color::Yellow),
                ),
            };

            if !status_text.is_empty() {
                let status_widget = Paragraph::new(status_text)
                    .style(status_style)
                    .alignment(ratatui::layout::Alignment::Right)
                    .wrap(ratatui::widgets::Wrap { trim: true });
                frame.render_widget(status_widget, help_chunks[1]);
            }
        }
        
        // Status bar for temporary messages (Slack toggle, etc.)
        if let (Some(status_msg), Some(timer)) = (&tui_state.status_message, &tui_state.status_message_timer) {
            // Auto-clear after 3 seconds
            if timer.elapsed().as_secs() >= 3 {
                tui_state.status_message = None;
                tui_state.status_message_timer = None;
            } else {
                let status_widget = Paragraph::new(status_msg.clone())
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    .alignment(ratatui::layout::Alignment::Center)
                    .wrap(ratatui::widgets::Wrap { trim: true });
                // Over the right-hand part of the help bar; there are only two parts (0.106 asked
                // for a third, and the "Press q to quit" message panicked the dashboard).
                frame.render_widget(Clear, help_chunks[1]);
                frame.render_widget(status_widget, help_chunks[1]);
            }
        }
    } else {
        // No data message
        let no_data_message = Paragraph::new(Text::styled(
            "You don't have any agentic development tool data.  Once you start using Claude Code, Codex CLI, or Kilo Code, you'll see some data here.",
            Style::default().add_modifier(Modifier::DIM),
        ));
        frame.render_widget(no_data_message, chunks[1]);

        // Help text for no-data view
        let mouse_mode_indicator = if tui_state.mouse_mode_enabled {
            "🖱️ Scroll"
        } else {
            "📝 Select"
        };
        let help = Paragraph::new(format!(
            "m: toggle {} mode, q/Esc: quit",
            mouse_mode_indicator
        ))
        .style(Style::default().add_modifier(Modifier::DIM));
        frame.render_widget(help, chunks[2]);
    }

    if tui_state.debug_overlay.enabled {
        render_debug_overlay(frame, tui_state);
    }
    if tui_state.help_open {
        render_help_overlay(frame);
    }
}

/// Every keyboard shortcut, grouped by where it works. Kept beside the key handling so the two
/// change together.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Everywhere",
        &[
            ("←/→  h/l", "switch tabs"),
            ("↑/↓  j/k", "move through days or rows"),
            ("Home/End  PgUp/PgDn", "jump"),
            ("Esc", "back one level (never quits)"),
            ("?", "this panel"),
            ("q  Ctrl-C", "quit"),
            ("U", "update; press again for other machines"),
            ("s", "Slack notifications on/off"),
            ("m", "mouse: scroll ↔ select text"),
            ("d", "debug log"),
            ("Ctrl-L", "redraw the screen"),
            ("Shift-Enter", "upload to Splitrail Cloud now"),
        ],
    ),
    (
        "Summary tab",
        &[("p", "plan quotas"), ("r", "records and plan closes"), ("v", "verbose: models, transcripts, servers")],
    ),
    (
        "Tool tabs",
        &[
            ("g", "rows by day → week → month → hour"),
            ("f", "filter by model (cycles)"),
            ("o", "filter by project (cycles)"),
            ("x", "clear filters"),
            ("+/-", "load more or fewer days"),
        ],
    ),
];

fn render_help_overlay(frame: &mut Frame) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, (group, keys)) in SHORTCUTS.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(*group, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
        for (key, what) in keys.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<22}"), Style::default().fg(Color::Cyan)),
                Span::raw(*what),
            ]));
        }
    }
    let area = frame.area();
    let width = 72.min(area.width.saturating_sub(2));
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            ratatui::widgets::Block::bordered().title(" Keyboard shortcuts · ? or Esc to close "),
        ),
        popup,
    );
}

fn render_debug_overlay(frame: &mut Frame, tui_state: &TuiState) {
    let area = frame.area();
    if area.height < 5 {
        return;
    }

    let height = area.height.min(12).max(5);
    let overlay_rect = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(height),
        width: area.width,
        height,
    };

    frame.render_widget(Clear, overlay_rect);

    let mut lines = Vec::new();
    lines.push(Line::styled(
        format!(
            "Debug log ({} lines, press D to close)",
            tui_state.debug_overlay.lines.len()
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ));

    let max_lines = overlay_rect.height.saturating_sub(2) as usize;
    let start = tui_state
        .debug_overlay
        .lines
        .len()
        .saturating_sub(max_lines);
    for entry in tui_state.debug_overlay.lines.iter().skip(start) {
        lines.push(Line::from(entry.as_str()));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("~/.splitrail.log"),
        )
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(paragraph, overlay_rect);
}

fn draw_daily_stats_table(
    frame: &mut Frame,
    area: Rect,
    stats: &AgenticCodingToolStats,
    format_options: &NumberFormatOptions,
    table_state: &mut TableState,
    view_start_offset: usize, // Pagination offset for viewing historical data
    period: Period,
) -> usize {
    // Convert BTreeMap to Vec and reverse to show most recent first
    let all_dates: Vec<(&String, &crate::types::DailyStats)> = stats.daily_stats.iter().collect();
    let total_days = all_dates.len();

    // Show the most recent 30 days by default, starting from the specified view offset (going backwards in time)
    let view_limit = 30; // Show 30 days by default

    // Since all_dates is in chronological order (oldest first, newest last),
    // for pagination we want to start from the end and move backwards in chunks of view_limit

    // Calculate start and end indices to get most recent days first
    // When view_start_offset is 0, we want the last view_limit days (most recent)
    // When view_start_offset is 1, we want the previous view_limit days, etc.
    let start_idx = if (view_start_offset + 1) * view_limit <= total_days {
        // Normal case: we can take a full view_limit window from the end
        total_days - (view_start_offset + 1) * view_limit
    } else {
        // Edge case: not enough days, take from the beginning
        0
    };
    let end_idx = std::cmp::min(total_days, total_days - view_start_offset * view_limit);

    // Ensure bounds are valid
    let start_idx = std::cmp::min(start_idx, end_idx);
    let end_idx = std::cmp::min(end_idx, total_days);

    let visible_dates = &all_dates[start_idx..end_idx];

    // Show most recent first (reverse the slice to get newest dates first)
    let visible_dates_reversed: Vec<_> = visible_dates.iter().rev().collect();

    // Find best values for highlighting among visible dates only
    let mut best_cost = 0.0;
    let mut best_cost_i = 0;
    let mut best_cached_tokens = 0;
    let mut best_cached_tokens_i = 0;
    let mut best_input_tokens = 0;
    let mut best_input_tokens_i = 0;
    let mut best_output_tokens = 0;
    let mut best_output_tokens_i = 0;
    let mut best_reasoning_tokens = 0;
    let mut best_reasoning_tokens_i = 0;
    let mut best_conversations = 0;
    let mut best_conversations_i = 0;
    let mut best_tool_calls = 0;
    let mut best_tool_calls_i = 0;

    for (i, (_, day_stats)) in visible_dates_reversed.iter().enumerate() {
        if day_stats.stats.cost > best_cost {
            best_cost = day_stats.stats.cost;
            best_cost_i = i;
        }
        if day_stats.stats.cached_tokens > best_cached_tokens {
            best_cached_tokens = day_stats.stats.cached_tokens;
            best_cached_tokens_i = i;
        }
        if day_stats.stats.input_tokens > best_input_tokens {
            best_input_tokens = day_stats.stats.input_tokens;
            best_input_tokens_i = i;
        }
        if day_stats.stats.output_tokens > best_output_tokens {
            best_output_tokens = day_stats.stats.output_tokens;
            best_output_tokens_i = i;
        }
        if day_stats.stats.reasoning_tokens > best_reasoning_tokens {
            best_reasoning_tokens = day_stats.stats.reasoning_tokens;
            best_reasoning_tokens_i = i;
        }
        if day_stats.conversations > best_conversations {
            best_conversations = day_stats.conversations;
            best_conversations_i = i;
        }
        if day_stats.stats.tool_calls > best_tool_calls {
            best_tool_calls = day_stats.stats.tool_calls;
            best_tool_calls_i = i;
        }
    }

    let _header = Row::new(vec![
        Cell::new(""),
        Cell::new("Date"),
        Cell::new(Text::from("Cost").right_aligned()),
        Cell::new(Text::from("Cached Tks").right_aligned()),
        Cell::new(Text::from("Inp Tks").right_aligned()),
        Cell::new(Text::from("Outp Tks").right_aligned()),
        Cell::new(Text::from("Reason Tks").right_aligned()),
        Cell::new(Text::from("Convs").right_aligned()),
        Cell::new(Text::from("Tools").right_aligned()),
        // Cell::new(Text::from("Lines").right_aligned()),
        Cell::new("Models"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    // Show most recent first (reverse the slice)
    let visible_dates_reversed: Vec<_> = visible_dates.iter().rev().collect();

    let header = Row::new(vec![
        Cell::new(""),
        Cell::new("Date"),
        Cell::new(Text::from("Cost").right_aligned()),
        Cell::new(Text::from("Cached Tks").right_aligned()),
        Cell::new(Text::from("Inp Tks").right_aligned()),
        Cell::new(Text::from("Outp Tks").right_aligned()),
        Cell::new(Text::from("Reason Tks").right_aligned()),
        Cell::new(Text::from("Convs").right_aligned()),
        Cell::new(Text::from("Tools").right_aligned()),
        // Cell::new(Text::from("Lines").right_aligned()),
        Cell::new("Models"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    let mut rows = Vec::new();
    let mut total_cost = 0.0;
    let mut total_cached = 0;
    let mut total_input = 0;
    let mut total_output = 0;
    let mut total_reasoning = 0;
    let mut total_tool_calls = 0;

    for (i, (date, day_stats)) in visible_dates_reversed.iter().enumerate() {
        total_cost += day_stats.stats.cost;
        total_cached += day_stats.stats.cached_tokens;
        total_input += day_stats.stats.input_tokens;
        total_output += day_stats.stats.output_tokens;
        total_reasoning += day_stats.stats.reasoning_tokens;
        total_tool_calls += day_stats.stats.tool_calls;

        let mut models_vec = day_stats.models.keys().cloned().collect::<Vec<String>>();
        models_vec.sort();
        let models = models_vec.join(", ");

        let lines_summary = format!(
            "{}/{}/{}",
            format_number(day_stats.stats.lines_read, format_options),
            format_number(day_stats.stats.lines_edited, format_options),
            format_number(day_stats.stats.lines_added, format_options)
        );

        // Check if this is an empty row
        let is_empty_row = day_stats.stats.cost == 0.0
            && day_stats.stats.cached_tokens == 0
            && day_stats.stats.input_tokens == 0
            && day_stats.stats.output_tokens == 0
            && day_stats.stats.reasoning_tokens == 0
            && day_stats.conversations == 0
            && day_stats.user_messages == 0
            && day_stats.ai_messages == 0
            && day_stats.stats.tool_calls == 0;

        // Create styled cells with colors matching original implementation
        let date_cell = if is_empty_row {
            Line::from(Span::styled(
                format_date_for_display(date),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else {
            Line::from(Span::raw(format_date_for_display(date)))
        };

        let cost_cell = if is_empty_row {
            Line::from(Span::styled(
                format!("${:.2}", day_stats.stats.cost),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_cost_i {
            Line::from(Span::styled(
                format!("${:.2}", day_stats.stats.cost),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::styled(
                format!("${:.2}", day_stats.stats.cost),
                Style::default().fg(Color::Yellow),
            ))
        }
        .right_aligned();

        let cached_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.stats.cached_tokens, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_cached_tokens_i {
            Line::from(Span::styled(
                format_number(day_stats.stats.cached_tokens, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::styled(
                format_number(day_stats.stats.cached_tokens, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        }
        .right_aligned();

        let input_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.stats.input_tokens, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_input_tokens_i {
            Line::from(Span::styled(
                format_number(day_stats.stats.input_tokens, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::raw(format_number(
                day_stats.stats.input_tokens,
                format_options,
            )))
        }
        .right_aligned();

        let output_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.stats.output_tokens, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_output_tokens_i {
            Line::from(Span::styled(
                format_number(day_stats.stats.output_tokens, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::raw(format_number(
                day_stats.stats.output_tokens,
                format_options,
            )))
        }
        .right_aligned();

        let reasoning_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.stats.reasoning_tokens, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_reasoning_tokens_i {
            Line::from(Span::styled(
                format_number(day_stats.stats.reasoning_tokens, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::raw(format_number(
                day_stats.stats.reasoning_tokens,
                format_options,
            )))
        }
        .right_aligned();

        let conv_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.conversations as u64, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_conversations_i {
            Line::from(Span::styled(
                format_number(day_stats.conversations as u64, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::raw(format_number(
                day_stats.conversations as u64,
                format_options,
            )))
        }
        .right_aligned();

        let tool_cell = if is_empty_row {
            Line::from(Span::styled(
                format_number(day_stats.stats.tool_calls as u64, format_options),
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else if i == best_tool_calls_i {
            Line::from(Span::styled(
                format_number(day_stats.stats.tool_calls as u64, format_options),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::styled(
                format_number(day_stats.stats.tool_calls as u64, format_options),
                Style::default().fg(Color::Green),
            ))
        }
        .right_aligned();

        let _lines_cell = if is_empty_row {
            Line::from(Span::styled(
                lines_summary,
                Style::default().add_modifier(Modifier::DIM),
            ))
        } else {
            Line::from(Span::styled(
                lines_summary,
                Style::default().fg(Color::Blue),
            ))
        }
        .right_aligned();

        let models_cell = Line::from(Span::styled(
            models,
            Style::default().add_modifier(Modifier::DIM),
        ));

        // Create arrow indicator for currently selected row
        let arrow_cell = if table_state.selected() == Some(i) {
            Line::from(Span::styled(
                "→",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::raw(""))
        };

        let row = Row::new(vec![
            arrow_cell,
            date_cell,
            cost_cell,
            cached_cell,
            input_cell,
            output_cell,
            reasoning_cell,
            conv_cell,
            tool_cell,
            // lines_cell,
            models_cell,
        ]);

        rows.push(row);
    }

    // Collect all unique models for the totals row (only from visible dates)
    let mut all_models = std::collections::HashSet::new();
    for (_, day_stats) in visible_dates_reversed.iter() {
        for model in day_stats.models.keys() {
            all_models.insert(model);
        }
    }
    let mut all_models_vec = all_models
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<String>>();
    all_models_vec.sort();
    let all_models_text = all_models_vec.join(", ");

    // Add separator row before totals
    let separator_row = Row::new(vec![
        Line::from(Span::styled(
            "",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "───────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "──────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "────────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "─────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "──────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "──────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            "──────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        /*
        Line::from(Span::styled(
            "───────────────────────",
            Style::default().add_modifier(Modifier::DIM),
        )),
        */
        Line::from(Span::styled(
            "─".repeat(all_models_text.len().max(18)),
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]);
    rows.push(separator_row);

    // Add totals row (only for visible dates)
    let _total_lines_r: u64 = visible_dates_reversed
        .iter()
        .map(|(_, s)| s.stats.lines_read)
        .sum();
    let _total_lines_e: u64 = visible_dates_reversed
        .iter()
        .map(|(_, s)| s.stats.lines_edited)
        .sum();
    let _total_lines_a: u64 = visible_dates_reversed
        .iter()
        .map(|(_, s)| s.stats.lines_added)
        .sum();

    let totals_row = Row::new(vec![
        // Arrow indicator for totals row when selected
        if table_state.selected() == Some(rows.len()) {
            Line::from(Span::styled(
                "→",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::raw(""))
        },
        Line::from(Span::styled(
            format!("Total ({}{})", visible_dates_reversed.len(), period.unit()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("${total_cost:.2}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(total_cached, format_options),
            Style::default()
                .add_modifier(Modifier::DIM)
                .add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(total_input, format_options),
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(total_output, format_options),
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(total_reasoning, format_options),
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(visible_dates_reversed.len() as u64, format_options), // Show number of visible days
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        Line::from(Span::styled(
            format_number(total_tool_calls as u64, format_options),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        /*
        Line::from(Span::styled(
            format!(
                "{}/{}/{}",
                format_number(total_lines_r, format_options),
                format_number(total_lines_e, format_options),
                format_number(total_lines_a, format_options)
            ),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ))
        .right_aligned(),
        */
        Line::from(Span::styled(
            all_models_text,
            Style::default().add_modifier(Modifier::DIM),
        )),
    ]);

    rows.push(totals_row);

    // Save the row count before moving rows into the table
    let total_rows = rows.len();

    let table = Table::new(
        rows,
        [
            Constraint::Length(1),  // Arrow
            Constraint::Length(12), // Date - reduced from 15 to 12 since we shortened the text to "Total (XXd)"
            Constraint::Length(10), // Cost
            Constraint::Length(15), // Cached: monthly and 30-day totals pass 10B ("14,722,853,747")
            Constraint::Length(12), // Input (was 8, increased significantly)
            Constraint::Length(12), // Output (was 9, increased)
            Constraint::Length(12), // Reasoning (was 11, increased)
            Constraint::Length(6),  // Convs
            Constraint::Length(6),  // Tools
            // Constraint::Length(23), // Lines
            Constraint::Min(10), // Models
        ],
    )
    .header(header)
    .block(Block::default().title(""))
    .row_highlight_style(Style::new().blue())
    .column_spacing(2);

    frame.render_stateful_widget(table, area, table_state);

    // Return the total number of rows in the table
    total_rows
}

fn draw_summary_stats(
    frame: &mut Frame,
    area: Rect,
    filtered_stats: &[&AgenticCodingToolStats],
    format_options: &NumberFormatOptions,
    show_totals: bool,
) {
    // Aggregate stats from all tools
    let mut total_cost: f64 = 0.0;
    let mut total_cached: u64 = 0;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut total_reasoning: u64 = 0;
    let mut total_tool_calls: u64 = 0;
    let mut all_days = std::collections::HashSet::new();

    for stats in filtered_stats {
        total_cost += stats
            .daily_stats
            .values()
            .map(|s| s.stats.cost)
            .sum::<f64>();
        total_cached += stats
            .daily_stats
            .values()
            .map(|s| s.stats.cached_tokens)
            .sum::<u64>();
        total_input += stats
            .daily_stats
            .values()
            .map(|s| s.stats.input_tokens)
            .sum::<u64>();
        total_output += stats
            .daily_stats
            .values()
            .map(|s| s.stats.output_tokens)
            .sum::<u64>();
        total_reasoning += stats
            .daily_stats
            .values()
            .map(|s| s.stats.reasoning_tokens)
            .sum::<u64>();
        total_tool_calls += stats
            .daily_stats
            .values()
            .map(|s| s.stats.tool_calls as u64)
            .sum::<u64>();

        // Collect unique days across all tools that have actual data
        for (day, day_stats) in &stats.daily_stats {
            if day_stats.stats.cost > 0.0
                || day_stats.stats.input_tokens > 0
                || day_stats.stats.output_tokens > 0
                || day_stats.stats.reasoning_tokens > 0
                || day_stats.stats.cached_tokens > 0
                || day_stats.stats.tool_calls > 0
                || day_stats.ai_messages > 0
                || day_stats.conversations > 0
            {
                all_days.insert(day);
            }
        }
    }

    let mut summary_lines: Vec<Line> = Vec::new();

    if show_totals {
        let total_tokens = total_cached + total_input + total_output;
        let tools_count = filtered_stats.len();

        // Define summary rows with labels and values
        let summary_rows = vec![
            ("Tools:", format!("{tools_count} tracked"), Color::Cyan),
            (
                "Tokens:",
                format_number(total_tokens, format_options),
                Color::LightBlue,
            ),
            (
                "Reasoning:",
                format_number(total_reasoning, format_options),
                Color::Red,
            ),
            (
                "Tool Calls:",
                format_number(total_tool_calls, format_options),
                Color::LightGreen,
            ),
            ("Cost:", format!("${total_cost:.2}"), Color::LightYellow),
            ("Days tracked:", all_days.len().to_string(), Color::White),
        ];

        // Find the maximum label width for alignment
        let max_label_width = summary_rows
            .iter()
            .map(|(label, _, _)| label.len())
            .max()
            .unwrap_or(0);

        // Create lines with consistent spacing
        summary_lines = summary_rows
            .into_iter()
            .map(|(label, value, color)| {
                Line::from(vec![
                    Span::raw(format!("{label:<max_label_width$}")),
                    Span::raw("      "), // 6 spaces between label and value
                    Span::styled(value, Style::new().fg(color).bold()),
                ])
            })
            .collect();

        summary_lines.insert(
            0,
            Line::from(Span::styled(
                "Overall Stats",
                Style::new().add_modifier(Modifier::BOLD),
            )),
        );
        summary_lines.insert(
            1,
            Line::from(Span::styled(
                "-------------",
                Style::new().add_modifier(Modifier::BOLD),
            )),
        );
    }

    let summary = Paragraph::new(summary_lines).block(Block::default());
    frame.render_widget(summary, area);
}

fn update_table_states(
    table_states: &mut Vec<TableState>,
    current_stats: &MultiAnalyzerStats,
    selected_tab: &mut usize,
) {
    let filtered_analyzers: Vec<&AgenticCodingToolStats> = current_stats
        .analyzer_stats
        .iter()
        .map(std::sync::Arc::as_ref)
        .filter(|stats| has_data(stats))
        .collect();

    let filtered_count = filtered_analyzers.len();

    // Create a map of analyzer name to old table state
    let mut old_states_by_name: std::collections::HashMap<String, TableState> =
        std::collections::HashMap::new();
    for (i, analyzer) in filtered_analyzers.iter().enumerate() {
        if i < table_states.len() {
            old_states_by_name.insert(analyzer.analyzer_name.clone(), table_states[i].clone());
        }
    }

    // Clear and rebuild table states, preserving by analyzer name
    table_states.clear();

    for analyzer in &filtered_analyzers {
        let state = if let Some(old_state) = old_states_by_name.get(&analyzer.analyzer_name) {
            // Preserve existing state for this analyzer
            old_state.clone()
        } else {
            // Create new state for new analyzers
            let mut new_state = TableState::default();
            new_state.select(Some(0));
            new_state
        };
        table_states.push(state);
    }

    // Ensure selected tab is within bounds (+1 for Summary tab)
    if *selected_tab > filtered_count {
        *selected_tab = filtered_count; // Last analyzer tab
    }
}

pub fn create_upload_progress_callback(
    format_options: &NumberFormatOptions,
) -> impl Fn(usize, usize) + '_ {
    static LAST_CURRENT: AtomicUsize = AtomicUsize::new(0);
    static DOTS: AtomicUsize = AtomicUsize::new(0);
    static LAST_DOTS_UPDATE: AtomicU64 = AtomicU64::new(0);

    move |current: usize, total: usize| {
        let last = LAST_CURRENT.load(Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let last_update = LAST_DOTS_UPDATE.load(Ordering::Relaxed);

        let mut should_update = false;

        if current != last {
            // Progress changed - update current but keep dots timing
            LAST_CURRENT.store(current, Ordering::Relaxed);
            should_update = true;
        }

        if now - last_update >= 500 {
            // 500ms between dot updates
            // Enough time passed - advance dots animation
            let dots = DOTS.load(Ordering::Relaxed);
            DOTS.store((dots + 1) % 4, Ordering::Relaxed);
            LAST_DOTS_UPDATE.store(now, Ordering::Relaxed);
            should_update = true;
        }

        if should_update {
            let current_dots = DOTS.load(Ordering::Relaxed);
            let dots_str = ".".repeat(current_dots);
            print!(
                "\r\x1b[KUploading {}/{} messages{}",
                format_number(current as u64, format_options),
                format_number(total as u64, format_options),
                dots_str
            );
            let _ = Write::flush(&mut stdout());
        }
    }
}

pub fn show_upload_success(total: usize, format_options: &NumberFormatOptions) {
    let _ = execute!(
        stdout(),
        Print("\r"),
        SetForegroundColor(crossterm::style::Color::DarkGreen),
        Print(format!(
            "✓ Successfully uploaded {} messages\n",
            format_number(total as u64, format_options)
        )),
        ResetColor
    );
}

pub fn show_upload_error(error: &str) {
    let _ = execute!(
        stdout(),
        Print("\r"),
        SetForegroundColor(crossterm::style::Color::DarkRed),
        Print(format!("✕ {}\n", error)),
        ResetColor
    );
}

#[cfg(test)]
mod period_tests {
    use super::*;

    #[test]
    fn days_group_into_iso_weeks_and_months_keeping_every_total() {
        let mut stats = AgenticCodingToolStats {
            daily_stats: Default::default(),
            num_conversations: 0,
            messages: Vec::new(),
            analyzer_name: "t".into(),
        };
        // Sun Dec 28 2025 is ISO week 52 of 2025; Mon Dec 29 starts 2026-W01.
        for (day, output, cost) in [("2025-12-28", 1, 1.0), ("2025-12-29", 10, 2.0), ("2026-01-02", 100, 4.0)] {
            let mut d = crate::types::DailyStats::default();
            d.stats.output_tokens = output;
            d.stats.cost = cost;
            d.stats.cache_creation_tokens = 7;
            d.stats.cache_creation_1h_tokens = 5;
            d.models.insert("m".into(), 1);
            stats.daily_stats.insert(day.into(), d);
        }
        let weeks = group_by_period(&stats, Period::Week);
        assert_eq!(weeks.daily_stats.keys().collect::<Vec<_>>(), ["2025-W52", "2026-W01"]);
        let w1 = &weeks.daily_stats["2026-W01"];
        assert_eq!((w1.stats.output_tokens, w1.stats.cost, w1.models["m"]), (110, 6.0, 2));
        assert_eq!((w1.stats.cache_creation_tokens, w1.stats.cache_creation_1h_tokens), (14, 10));
        let months = group_by_period(&stats, Period::Month);
        assert_eq!(months.daily_stats.keys().collect::<Vec<_>>(), ["2025-12", "2026-01"]);
        assert_eq!(months.daily_stats["2025-12"].stats.output_tokens, 11);
    }

    #[test]
    fn a_status_message_draws_without_panicking() {
        // 0.106-0.113 drew it into a third part of the two-part help bar: the first Esc at the
        // top ("Press q to quit") panicked the dashboard.
        let stats = AgenticCodingToolStats {
            daily_stats: Default::default(),
            num_conversations: 0,
            messages: Vec::new(),
            analyzer_name: "t".into(),
        };
        let mut state = TuiState {
            status_message: Some("Press q to quit".into()),
            status_message_timer: Some(std::time::Instant::now()),
            ..Default::default()
        };
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| {
                draw_ui(
                    frame,
                    &[&stats],
                    &crate::utils::NumberFormatOptions::default(),
                    &mut [TableState::default()],
                    &mut state,
                    Arc::new(Mutex::new(UploadStatus::None)),
                    &None,
                    chrono::Utc::now(),
                    SystemTime::now(),
                )
            })
            .unwrap();
        let screen: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("Press q to quit"));
    }

    #[test]
    fn the_help_panel_lists_every_key_the_dashboard_handles() {
        let listed: String = SHORTCUTS.iter().flat_map(|(_, keys)| keys.iter().map(|(k, _)| format!(" {k} "))).collect();
        for key in ["p", "r", "v", "g", "f", "o", "x", "s", "m", "d", "U", "?", "Esc", "Ctrl-L", "Shift-Enter", "+/-"] {
            assert!(listed.contains(&format!(" {key} ")) || listed.contains(&format!(" {key}  ")) || listed.contains(key), "{key} missing from the ? panel");
        }
        // Esc closes the panel before anything else.
        let mut state = TuiState { help_open: true, records_view: true, ..Default::default() };
        assert!(go_back(&mut state));
        assert!(!state.help_open && state.records_view);
    }

    #[test]
    fn esc_goes_back_one_level_at_a_time_and_never_past_the_top() {
        let mut state = TuiState {
            selected_tab: 2,
            period: Period::Hour,
            filter: SliceFilter { model: Some("opus".into()), project: None },
            ..Default::default()
        };
        assert!(go_back(&mut state));
        assert!(state.filter.is_empty());
        assert!(go_back(&mut state));
        assert_eq!(state.period, Period::Day);
        assert!(go_back(&mut state));
        assert_eq!(state.selected_tab, 0);
        state.summary_verbose_mode = true;
        state.records_view = true;
        assert!(go_back(&mut state));
        assert!(!state.records_view && state.summary_verbose_mode);
        assert!(go_back(&mut state));
        assert!(!state.summary_verbose_mode);
        // The top: nothing to go back from, and Esc does not quit.
        assert!(!go_back(&mut state));
    }

    fn slice(hour: u8, model: &str, project: &str, output: u64, cost: f64) -> crate::types::Slice {
        crate::types::Slice {
            hour,
            model: model.into(),
            project: project.into(),
            replies: 1,
            output_tokens: output,
            cost,
            ..Default::default()
        }
    }

    #[test]
    fn slices_give_hourly_rows_and_filter_by_model_and_project() {
        let mut stats = AgenticCodingToolStats {
            daily_stats: Default::default(),
            num_conversations: 0,
            messages: Vec::new(),
            analyzer_name: "t".into(),
        };
        let mut day = crate::types::DailyStats::default();
        day.slices = vec![
            slice(9, "opus", "splitrail", 100, 1.0),
            slice(9, "sonnet", "splitrail", 10, 0.1),
            slice(14, "opus", "mediasrc", 1000, 5.0),
        ];
        stats.daily_stats.insert("2026-10-17".into(), day);

        let hours = group_slices(&stats, Period::Hour, &SliceFilter::default());
        assert_eq!(hours.daily_stats.keys().collect::<Vec<_>>(), ["2026-10-17 09", "2026-10-17 14"]);
        assert_eq!(hours.daily_stats["2026-10-17 09"].stats.output_tokens, 110);
        assert_eq!(crate::utils::format_date_for_display("2026-10-17 09"), "10/17 09:00");

        let opus = SliceFilter { model: Some("opus".into()), project: None };
        let days = group_slices(&stats, Period::Day, &opus);
        assert_eq!((days.daily_stats["2026-10-17"].stats.output_tokens, days.daily_stats["2026-10-17"].stats.cost), (1100, 6.0));
        let both = SliceFilter { model: Some("opus".into()), project: Some("splitrail".into()) };
        assert_eq!(group_slices(&stats, Period::Month, &both).daily_stats["2026-10"].stats.output_tokens, 100);

        // `f` steps through models busiest first, then back to all.
        let models = slice_options(&stats, true);
        assert_eq!(models, ["opus", "sonnet"]);
        assert_eq!(cycle(&None, &models).as_deref(), Some("opus"));
        assert_eq!(cycle(&Some("opus".into()), &models).as_deref(), Some("sonnet"));
        assert_eq!(cycle(&Some("sonnet".into()), &models), None);
    }
}

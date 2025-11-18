use crate::types::{AgenticCodingToolStats, MultiAnalyzerStats};
use crate::utils::{NumberFormatOptions, format_date_for_display, format_number};
use crate::watcher::{FileWatcher, RealtimeStatsManager};
use anyhow::Result;
use chrono::Duration as ChronoDuration;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::style::{Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{ExecutableCommand, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Tabs};
use ratatui::{Frame, Terminal};
use std::io::{Write, stdout};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, Default)]
pub struct UiLayout {
    pub today_by_cli_rect: Option<Rect>,
    pub live_activity_rect: Option<Rect>,
}

#[derive(Debug, Default)]
pub struct TuiState {
    pub selected_tab: usize,
    pub scroll_offset: usize,
    pub summary_day_offset: usize,
    pub cli_scroll_offset: usize,
    pub cli_table_scroll_offset: usize, // For horizontal scroll
    pub layout: UiLayout,
    pub mouse_mode_enabled: bool, // Toggle between mouse scroll and text selection
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
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut tui_state = TuiState::default();
    // Enable mouse mode by default for consistent scrolling
    tui_state.mouse_mode_enabled = true;
    terminal.backend_mut().execute(EnableMouseCapture)?;

    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(run_app(
            &mut terminal,
            stats_receiver,
            format_options,
            &mut tui_state,
            upload_status,
            file_watcher,
            &mut stats_manager,
        ))
    });

    disable_raw_mode()?;
    terminal
        .backend_mut()
        .execute(DisableMouseCapture)?
        .execute(LeaveAlternateScreen)?;
    result
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
        .filter(|stats| has_data(stats))
        .collect();

    // Cache summary data to avoid recalculating on every redraw
    let mut cached_summary_data: Option<SummaryData> = Some(calculate_summary_data(
        &filtered_stats,
        tui_state.summary_day_offset,
    ));

    loop {
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
                .filter(|stats| has_data(stats))
                .collect();
            // Recalculate summary data when stats change
            cached_summary_data = Some(calculate_summary_data(
                &filtered_stats,
                tui_state.summary_day_offset,
            ));
            needs_redraw = true;
        }

        // Check for file watcher events
        while let Some(watcher_event) = file_watcher.try_recv() {
            if let Err(e) = stats_manager.handle_watcher_event(watcher_event).await {
                eprintln!("Error handling watcher event: {e}");
            }
        }

        // Poll Codex CLI periodically (every 5 seconds)
        if let Err(e) = stats_manager.poll_codex_if_needed().await {
            eprintln!("Error polling Codex CLI: {e}");
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

        // Only redraw if something has changed
        if needs_redraw {
            terminal.draw(|frame| {
                draw_ui(
                    frame,
                    &filtered_stats,
                    format_options,
                    &mut table_states,
                    tui_state,
                    upload_status.clone(),
                    &cached_summary_data,
                );
            })?;
            needs_redraw = false;
        }

        // Use a timeout to allow periodic refreshes for upload status updates
        if let Ok(event_available) = event::poll(Duration::from_millis(100)) {
            if !event_available {
                continue;
            }

            // Handle different event types
            match event::read()? {
                Event::Key(key) if key.is_press() => {
                    // Handle quitting.
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                        break;
                    }

                    // Handle mouse mode toggle
                    if matches!(key.code, KeyCode::Char('m')) {
                        tui_state.mouse_mode_enabled = !tui_state.mouse_mode_enabled;
                        if tui_state.mouse_mode_enabled {
                            terminal.backend_mut().execute(EnableMouseCapture)?;
                        } else {
                            // Disable mouse capture and flush any pending events
                            terminal.backend_mut().execute(DisableMouseCapture)?;
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

                    // Only handle navigation keys if we have data (`filtered_stats` is non-empty).
                    if filtered_stats.is_empty() {
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
                                    let total_rows = current_stats.daily_stats.len() + 3; // header + data + separator + totals
                                    if let Some(table_state) = table_states.get_mut(analyzer_index)
                                        && let Some(selected) = table_state.selected()
                                        && selected < total_rows.saturating_sub(1)
                                    {
                                        table_state.select(Some(
                                            if selected == current_stats.daily_stats.len() {
                                                selected + 2 // Skip separator row
                                            } else {
                                                selected + 1
                                            },
                                        ));
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
                                    && let Some(current_stats) = filtered_stats.get(analyzer_index)
                                    && let Some(table_state) = table_states.get_mut(analyzer_index)
                                    && let Some(selected) = table_state.selected()
                                    && selected > 0
                                {
                                    table_state.select(Some(selected.saturating_sub(
                                        if selected == current_stats.daily_stats.len() + 1 {
                                            2 // Skip separator row
                                        } else {
                                            1
                                        },
                                    )));
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
                                    let total_rows = current_stats.daily_stats.len() + 2;
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
                                    let total_rows = current_stats.daily_stats.len() + 2;
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
                        _ => {}
                    }
                }
                Event::Mouse(mouse_event) => {
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

fn draw_ui(
    frame: &mut Frame,
    filtered_stats: &[&AgenticCodingToolStats],
    format_options: &NumberFormatOptions,
    table_states: &mut [TableState],
    tui_state: &mut TuiState,
    upload_status: Arc<Mutex<UploadStatus>>,
    cached_summary_data: &Option<SummaryData>,
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
        Layout::vertical([
            Constraint::Length(3),                             // Header
            Constraint::Length(1),                             // Tabs
            Constraint::Min(3),                                // Main table
            Constraint::Length(summary_stats_height),          // Summary stats (0 for Summary tab)
            Constraint::Length(if has_error { 3 } else { 1 }), // Help text
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
    let header = Paragraph::new(Text::from(vec![
        Line::styled(
            "AGENTIC DEVELOPMENT TOOL ACTIVITY ANALYSIS",
            Style::new().cyan().bold(),
        ),
        Line::styled(
            "==========================================",
            Style::new().cyan().bold(),
        ),
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

        if tui_state.selected_tab == 0 {
            // Summary view - show cached aggregated data across all analyzers
            if let Some(summary_data) = &cached_summary_data {
                draw_summary_view(
                    frame,
                    chunks[2],
                    summary_data,
                    format_options,
                    filtered_stats,
                    tui_state,
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
                draw_daily_stats_table(
                    frame,
                    chunks[2],
                    current_stats,
                    format_options,
                    current_table_state,
                );

                // Summary stats - pass all filtered stats for aggregation
                draw_summary_stats(frame, chunks[3], filtered_stats, format_options, true);
            }
        }

        // Help text for data view with upload status
        let help_area = chunks[4];

        // Split help area horizontally: help text on left, upload status on right
        let help_chunks = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Min(20), // Allow flexible space for error messages
        ])
        .split(help_area);

        let mouse_mode_indicator = if tui_state.mouse_mode_enabled {
            "🖱️ Scroll"
        } else {
            "📝 Select"
        };

        let help = if tui_state.selected_tab == 0 {
            Paragraph::new(format!(
                "←/→ or h/l: tabs, ↑/↓ or j/k: navigate days, m: toggle {} mode, q/Esc: quit",
                mouse_mode_indicator
            ))
            .style(Style::default().add_modifier(Modifier::DIM))
        } else {
            Paragraph::new(format!(
                "←/→ or h/l: tabs, ↑/↓ or j/k: navigate, m: toggle {} mode, q/Esc: quit",
                mouse_mode_indicator
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
    } else {
        // No data message
        let no_data_message = Paragraph::new(Text::styled(
            "You don't have any agentic development tool data.  Once you start using Claude Code, Gemini CLI, or Codex CLI, you'll see some data here.",
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
}

fn draw_daily_stats_table(
    frame: &mut Frame,
    area: Rect,
    stats: &AgenticCodingToolStats,
    format_options: &NumberFormatOptions,
    table_state: &mut TableState,
) -> usize {
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

    // Find best values for highlighting
    // TODO: Let's refactor this.

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

    for (i, day_stats) in stats.daily_stats.values().enumerate() {
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

    let mut rows = Vec::new();
    let mut total_cost = 0.0;
    let mut total_cached = 0;
    let mut total_input = 0;
    let mut total_output = 0;
    let mut total_reasoning = 0;
    let mut total_tool_calls = 0;

    for (i, (date, day_stats)) in stats.daily_stats.iter().enumerate() {
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

    // Collect all unique models for the totals row
    let mut all_models = std::collections::HashSet::new();
    for day_stats in stats.daily_stats.values() {
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

    // Add totals row
    let _total_lines_r = stats
        .daily_stats
        .values()
        .map(|s| s.stats.lines_read)
        .sum::<u64>();
    let _total_lines_e = stats
        .daily_stats
        .values()
        .map(|s| s.stats.lines_edited)
        .sum::<u64>();
    let _total_lines_a = stats
        .daily_stats
        .values()
        .map(|s| s.stats.lines_added)
        .sum::<u64>();

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
            format!("Total ({}d)", stats.daily_stats.len()),
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
            format_number(stats.num_conversations, format_options),
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
            Constraint::Length(11), // Date
            Constraint::Length(10), // Cost
            Constraint::Length(13), // Cached (was 12, increased for larger numbers)
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

fn calculate_summary_data(
    filtered_stats: &[&AgenticCodingToolStats],
    day_offset: usize,
) -> SummaryData {
    // Calculate date ranges
    let now = chrono::Local::now();
    let today_start = now.date_naive();
    let yesterday_start = (now - ChronoDuration::days(1)).date_naive();
    let week_ago = (now - ChronoDuration::days(7)).date_naive();
    let two_weeks_ago = (now - ChronoDuration::days(14)).date_naive();
    let selected_day = (now - ChronoDuration::days(day_offset as i64)).date_naive();

    // Aggregate data for each time period
    let mut today_stats = AggregatedStats::default();
    let mut yesterday_stats = AggregatedStats::default();
    let mut week_stats = AggregatedStats::default();
    let mut two_week_stats = AggregatedStats::default();
    let mut selected_day_stats = AggregatedStats::default();

    for analyzer_stats in filtered_stats {
        for (date_str, day_stats) in &analyzer_stats.daily_stats {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                if date == today_start {
                    today_stats.add_day(day_stats);
                }
                if date == yesterday_start {
                    yesterday_stats.add_day(day_stats);
                }
                if date >= week_ago {
                    week_stats.add_day(day_stats);
                }
                if date >= two_weeks_ago {
                    two_week_stats.add_day(day_stats);
                }
                if date == selected_day {
                    selected_day_stats.add_day(day_stats);
                }
            }
        }
    }

    SummaryData {
        today_stats,
        yesterday_stats,
        week_stats,
        two_week_stats,
        selected_day_stats,
        selected_day_offset: day_offset,
        active_clis: filtered_stats.len(),
    }
}

fn draw_summary_view(
    frame: &mut Frame,
    area: Rect,
    summary_data: &SummaryData,
    format_options: &NumberFormatOptions,
    filtered_stats: &[&AgenticCodingToolStats],
    tui_state: &mut TuiState,
) {
    let SummaryData {
        today_stats,
        yesterday_stats,
        week_stats,
        two_week_stats,
        selected_day_stats: _,
        selected_day_offset,
        active_clis,
    } = summary_data;

    // Split area into parts: spacing + overview table + spacing + CLI breakdown table + visual panels
    let chunks = Layout::vertical([
        Constraint::Length(1),  // Space above overview
        Constraint::Length(11), // Overview table (header + 8 rows + border)
        Constraint::Length(1),  // Space between tables
        Constraint::Length(13), // CLI breakdown table (fixed height)
        Constraint::Length(1),  // Space before visual panels
        Constraint::Min(0),     // Visual CLI panels (takes remaining space)
    ])
    .split(area);

    // Store the rects for mouse handling
    tui_state.layout.today_by_cli_rect = Some(chunks[3]);
    tui_state.layout.live_activity_rect = Some(chunks[5]);

    // Create table rows
    let header = Row::new(vec![
        Cell::new(""),
        Cell::new(Text::from("Today").right_aligned()),
        Cell::new(Text::from("Yesterday").right_aligned()),
        Cell::new(Text::from("7 Days").right_aligned()),
        Cell::new(Text::from("14 Days").right_aligned()),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    let rows = vec![
        Row::new(vec![
            Cell::new(Line::from("💾 Cached Tks").style(Style::default().fg(Color::LightMagenta))),
            Cell::new(
                Line::from(format_number(today_stats.cached_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(yesterday_stats.cached_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.cached_tokens, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(two_week_stats.cached_tokens, format_options))
                    .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("📥 Input Tks").style(Style::default().fg(Color::LightBlue))),
            Cell::new(
                Line::from(format_number(today_stats.input_tokens, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(yesterday_stats.input_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.input_tokens, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(two_week_stats.input_tokens, format_options))
                    .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("📤 Output Tks").style(Style::default().fg(Color::LightCyan))),
            Cell::new(
                Line::from(format_number(today_stats.output_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(yesterday_stats.output_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.output_tokens, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(two_week_stats.output_tokens, format_options))
                    .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("🧠 Reasoning").style(Style::default().fg(Color::Red))),
            Cell::new(
                Line::from(format_number(today_stats.reasoning_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(
                    yesterday_stats.reasoning_tokens,
                    format_options,
                ))
                .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.reasoning_tokens, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(
                    two_week_stats.reasoning_tokens,
                    format_options,
                ))
                .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("🛠️ Tool Calls").style(Style::default().fg(Color::LightGreen))),
            Cell::new(
                Line::from(format_number(today_stats.tool_calls, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(yesterday_stats.tool_calls, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.tool_calls, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(two_week_stats.tool_calls, format_options))
                    .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("💬 Conversations").style(Style::default().fg(Color::Cyan))),
            Cell::new(
                Line::from(format_number(today_stats.conversations, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(yesterday_stats.conversations, format_options))
                    .right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(week_stats.conversations, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(two_week_stats.conversations, format_options))
                    .right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("📊 CLIs Active").style(Style::default().fg(Color::Magenta))),
            Cell::new(
                Line::from(format_number(*active_clis as u64, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(*active_clis as u64, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(*active_clis as u64, format_options)).right_aligned(),
            ),
            Cell::new(
                Line::from(format_number(*active_clis as u64, format_options)).right_aligned(),
            ),
        ]),
        Row::new(vec![
            Cell::new(Line::from("💰 Cost").style(Style::default().fg(Color::Yellow))),
            Cell::new(Line::from(format!("${:.2}", today_stats.cost)).right_aligned()),
            Cell::new(Line::from(format!("${:.2}", yesterday_stats.cost)).right_aligned()),
            Cell::new(Line::from(format!("${:.2}", week_stats.cost)).right_aligned()),
            Cell::new(Line::from(format!("${:.2}", two_week_stats.cost)).right_aligned()),
        ]),
    ];

    let table = Table::new(
        rows,
        [
            Constraint::Length(15), // Metric
            Constraint::Length(12), // Today
            Constraint::Length(12), // Yesterday
            Constraint::Length(12), // 7 Days
            Constraint::Length(12), // 14 Days
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("📈 Summary Overview")
            .title_style(Style::default().bold()),
    )
    .column_spacing(2);

    frame.render_widget(table, chunks[1]);

    // Calculate stats for the selected day for each CLI
    let now = chrono::Local::now();
    let selected_day_date = (now - ChronoDuration::days(*selected_day_offset as i64)).date_naive();

    // Collect data for the selected day for each CLI
    let mut cli_data: Vec<(
        String,
        u64,
        u64,
        u64,
        u64,
        f64,
        String,
        String,
        u64,
        u64,
        String,
    )> = Vec::new();
    let mut cli_meta: Vec<(i64, u8)> = Vec::new();
    for analyzer_stats in filtered_stats {
        let now_utc = chrono::Utc::now();
        let mut cached = 0u64;
        let mut input = 0u64;
        let mut output = 0u64;
        let mut reasoning = 0u64;
        let mut cost = 0.0;

        for (date_str, day_stats) in &analyzer_stats.daily_stats {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                if date == selected_day_date {
                    cached += day_stats.stats.cached_tokens;
                    input += day_stats.stats.input_tokens;
                    output += day_stats.stats.output_tokens;
                    reasoning += day_stats.stats.reasoning_tokens;
                    cost += day_stats.stats.cost;
                }
            }
        }

        // Find most recent message timestamp
        let last_activity = analyzer_stats.messages.iter().map(|msg| msg.date).max();

        let idle_time = if let Some(last_msg_time) = last_activity {
            let duration = now_utc.signed_duration_since(last_msg_time);
            if duration.num_days() > 0 {
                format!("{}d ago", duration.num_days())
            } else if duration.num_hours() > 0 {
                format!("{}h ago", duration.num_hours())
            } else if duration.num_minutes() > 0 {
                format!("{}m ago", duration.num_minutes())
            } else {
                "Just now".to_string()
            }
        } else {
            "No data".to_string()
        };

        // Get messages for the selected day and sort by timestamp
        let mut selected_day_messages: Vec<_> = analyzer_stats
            .messages
            .iter()
            .filter(|msg| msg.date.with_timezone(&chrono::Local).date_naive() == selected_day_date)
            .collect();
        selected_day_messages.sort_by_key(|msg| msg.date);

        // Count messages
        let message_count = selected_day_messages.len() as u64;

        // Count unique conversation sessions (by conversation_hash)
        let unique_sessions: std::collections::HashSet<_> = selected_day_messages
            .iter()
            .map(|msg| &msg.conversation_hash)
            .collect();
        let session_count = unique_sessions.len() as u64;

        // Calculate actual active time (sum of gaps < 15 minutes between consecutive messages)
        let active_time = if selected_day_messages.len() > 1 {
            let mut total_active_seconds = 0i64;
            for window in selected_day_messages.windows(2) {
                let gap = window[1].date.signed_duration_since(window[0].date);
                let gap_seconds = gap.num_seconds();
                // Only count gaps less than 15 minutes as "active time"
                if gap_seconds > 0 && gap_seconds < 900 {
                    total_active_seconds += gap_seconds;
                }
            }

            let hours = total_active_seconds / 3600;
            let minutes = (total_active_seconds % 3600) / 60;

            if hours > 0 {
                format!("{}h {}m", hours, minutes)
            } else if minutes > 0 {
                format!("{}m", minutes)
            } else if total_active_seconds > 0 {
                format!("{}s", total_active_seconds)
            } else {
                "< 1m".to_string()
            }
        } else {
            "0m".to_string()
        };

        // Determine CLI state based on last message and recent activity
        let (state, activity_age, activity_bucket) =
            if let Some(last_message) = analyzer_stats.messages.iter().max_by_key(|msg| msg.date) {
                let mut seconds_since_last = now_utc
                    .signed_duration_since(last_message.date)
                    .num_seconds();
                if seconds_since_last < 0 {
                    seconds_since_last = 0;
                }

                // Check for recent consecutive assistant messages (indicates active work)
                let recent_assistant_messages = analyzer_stats
                    .messages
                    .iter()
                    .filter(|msg| {
                        let age = now_utc.signed_duration_since(msg.date).num_seconds();
                        age < 60 && matches!(msg.role, crate::types::MessageRole::Assistant)
                    })
                    .count();

                if seconds_since_last < 30 && recent_assistant_messages > 1 {
                    // Multiple recent assistant messages = actively working
                    ("🟢 Active".to_string(), seconds_since_last, 0)
                } else if matches!(last_message.role, crate::types::MessageRole::Assistant)
                    && seconds_since_last < 120
                {
                    // Last message from assistant, recent = waiting for input
                    ("🟡 Waiting".to_string(), seconds_since_last, 0)
                } else if matches!(last_message.role, crate::types::MessageRole::User)
                    && seconds_since_last < 120
                {
                    // Last message from user, recent = processing
                    ("🔵 Processing".to_string(), seconds_since_last, 0)
                } else if seconds_since_last < 900 {
                    // No activity in last 15 min but recent = idle
                    ("● Idle".to_string(), seconds_since_last, 1)
                } else {
                    // Old activity = inactive
                    ("○ Inactive".to_string(), seconds_since_last, 2)
                }
            } else {
                ("○ No data".to_string(), i64::MAX / 4, 2)
            };

        cli_data.push((
            analyzer_stats.analyzer_name.clone(),
            cached,
            input,
            output,
            reasoning,
            cost,
            idle_time,
            active_time,
            session_count,
            message_count,
            state,
        ));
        cli_meta.push((activity_age, activity_bucket));
    }

    // Build CLI breakdown table with metrics as rows and CLIs as columns
    let mut cli_order: Vec<usize> = (0..cli_data.len()).collect();
    cli_order.sort_by_key(|&idx| {
        let (age, bucket) = cli_meta[idx];
        (bucket, age, idx)
    });

    let max_visible_cols = ((area.width - 15) / 15).max(1) as usize;
    let total_clis = cli_data.len();
    let start_col = tui_state.cli_table_scroll_offset.min(total_clis);
    let end_col = (start_col + max_visible_cols).min(total_clis);
    let visible_indices = &cli_order[start_col..end_col];

    let mut cli_header_cells = vec![Cell::new("")];
    for &idx in visible_indices {
        let (cli_name, _, _, _, _, _, _, _, _, _, _) = &cli_data[idx];
        cli_header_cells.push(Cell::new(Text::from(cli_name.clone()).right_aligned()));
    }
    let cli_header = Row::new(cli_header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let mut cli_constraints = vec![Constraint::Length(15)]; // Metric label
    for _ in 0..visible_indices.len() {
        cli_constraints.push(Constraint::Length(13)); // Each CLI column
    }

    let cli_rows = vec![
        // Cached Tokens row
        {
            let mut cells = vec![Cell::new(
                Line::from("💾 Cached Tks").style(Style::default().fg(Color::LightMagenta)),
            )];
            for &idx in visible_indices {
                let (_, cached, _, _, _, _, _, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*cached, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Input Tokens row
        {
            let mut cells = vec![Cell::new(
                Line::from("📥 Input Tks").style(Style::default().fg(Color::LightBlue)),
            )];
            for &idx in visible_indices {
                let (_, _, input, _, _, _, _, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*input, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Output Tokens row
        {
            let mut cells = vec![Cell::new(
                Line::from("📤 Output Tks").style(Style::default().fg(Color::LightCyan)),
            )];
            for &idx in visible_indices {
                let (_, _, _, output, _, _, _, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*output, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Reasoning row
        {
            let mut cells = vec![Cell::new(
                Line::from("🧠 Reasoning").style(Style::default().fg(Color::Red)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, reasoning, _, _, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*reasoning, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Sessions row
        {
            let mut cells = vec![Cell::new(
                Line::from("💬 Sessions").style(Style::default().fg(Color::Cyan)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, _, _, _, sessions, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*sessions, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Messages row
        {
            let mut cells = vec![Cell::new(
                Line::from("📨 Messages").style(Style::default().fg(Color::LightYellow)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, _, _, _, _, messages, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format_number(*messages, format_options)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Active Time row
        {
            let mut cells = vec![Cell::new(
                Line::from("⏱️ Active Time").style(Style::default().fg(Color::LightGreen)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, _, _, active_time, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(Line::from(active_time.clone()).right_aligned()));
            }
            Row::new(cells)
        },
        // Idle Time row
        {
            let mut cells = vec![Cell::new(
                Line::from("⏰ Idle Time").style(Style::default().fg(Color::DarkGray)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, _, idle_time, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(Line::from(idle_time.clone()).right_aligned()));
            }
            Row::new(cells)
        },
        // Cost row
        {
            let mut cells = vec![Cell::new(
                Line::from("💰 Cost").style(Style::default().fg(Color::Yellow)),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, cost, _, _, _, _, _) = &cli_data[idx];
                cells.push(Cell::new(
                    Line::from(format!("${:.2}", cost)).right_aligned(),
                ));
            }
            Row::new(cells)
        },
        // Status row
        {
            let mut cells = vec![Cell::new(
                Line::from("📡 Status").style(Style::default().fg(Color::White).bold()),
            )];
            for &idx in visible_indices {
                let (_, _, _, _, _, _, _, _, _, _, state) = &cli_data[idx];
                cells.push(Cell::new(Line::from(state.clone()).right_aligned()));
            }
            Row::new(cells)
        },
    ];

    // Format the title based on the selected day offset
    let table_title = if *selected_day_offset == 0 {
        "📊 Today by CLI".to_string()
    } else {
        // Format the selected date as "Weekday, Month Day, Year"
        let selected_date_with_tz = now - ChronoDuration::days(*selected_day_offset as i64);
        let weekday = selected_date_with_tz.format("%A").to_string(); // Monday, Tuesday, etc.
        let formatted_date = selected_date_with_tz.format("%B %d, %Y").to_string(); // November 15, 2025
        format!(
            "📊 {} ({}, {} days ago) by CLI",
            weekday, formatted_date, selected_day_offset,
        )
    };

    let cli_table = Table::new(cli_rows, cli_constraints)
        .header(cli_header)
        .block(
            Block::default()
                .title(table_title)
                .title_style(Style::default().bold()),
        )
        .column_spacing(2);

    frame.render_widget(cli_table, chunks[3]);

    // Draw visual CLI panels
    draw_visual_cli_panels(
        frame,
        chunks[5],
        filtered_stats,
        &cli_data,
        &cli_order,
        format_options,
        tui_state,
    );
}

// Helper function to create a horizontal bar visualization
fn create_bar(value: u64, max_value: u64, width: usize, color: Color) -> Line<'static> {
    let filled = if max_value > 0 {
        ((value as f64 / max_value as f64) * width as f64) as usize
    } else {
        0
    };
    let filled = filled.min(width);
    let empty = width.saturating_sub(filled);

    let filled_chars = "█".repeat(filled);
    let empty_chars = "░".repeat(empty);

    Line::from(vec![
        Span::styled(filled_chars, Style::default().fg(color)),
        Span::styled(empty_chars, Style::default().fg(Color::DarkGray)),
    ])
}

// Helper function to create a percentage display
fn create_percentage_bar(
    value: u64,
    total: u64,
    width: usize,
    color: Color,
) -> (Line<'static>, String) {
    let percentage = if total > 0 {
        (value as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let bar = create_bar(value, total, width, color);
    let pct_text = format!("{:>5.1}%", percentage);

    (bar, pct_text)
}

// Helper to create activity sparkline for last hour
fn create_activity_sparkline(stats: &AgenticCodingToolStats) -> Vec<Span<'static>> {
    let now = chrono::Utc::now();

    // Create 120 buckets (30-second intervals for last hour)
    let mut buckets = vec![0u64; 120];

    for msg in &stats.messages {
        let age = now.signed_duration_since(msg.date);
        if age.num_seconds() < 3600 && age.num_seconds() >= 0 {
            let bucket_idx = (age.num_seconds() / 30) as usize; // 30 sec intervals
            if bucket_idx < 120 {
                let total_tokens = msg.stats.input_tokens
                    + msg.stats.output_tokens
                    + msg.stats.reasoning_tokens;

                if total_tokens > 0 {
                    buckets[119 - bucket_idx] += total_tokens;
                } else {
                    // If there are no tokens, still count it as a message event
                    buckets[119 - bucket_idx] += 1;
                }
            }
        }
    }

    // Find max for scaling
    let max = *buckets.iter().max().unwrap_or(&1).max(&1);

    // Create sparkline with tiny dots like btop (truncate to 80 most recent)
    let chars = [' ', '⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷', '⣿'];
    buckets
        .iter()
        .skip(buckets.len().saturating_sub(80)) // Show the most recent 80 buckets (40 minutes)
        .map(|&count| {
            if count == 0 {
                Span::raw(" ")
            } else {
                let ratio = count as f64 / max as f64;
                let idx = (ratio * (chars.len() - 1) as f64) as usize;
                let ch = chars[idx.min(chars.len() - 1)];

                // Gradient colors: green (low) -> yellow (medium) -> red (high)
                let color = if ratio < 0.33 {
                    Color::Green
                } else if ratio < 0.66 {
                    Color::Yellow
                } else {
                    Color::Red
                };

                Span::styled(ch.to_string(), Style::default().fg(color))
            }
        })
        .collect()
}

// Helper function to get the current username
fn get_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "You".to_string())
}

// Helper function to simplify analyzer names to single words
fn simplify_analyzer_name(name: &str) -> &str {
    match name {
        "Claude Code" => "Claude",
        "Codex CLI" => "Codex",
        "Gemini CLI" => "Gemini",
        "GitHub Copilot" => "Copilot",
        "Cline" => "Cline",
        "Roo Code" => "Roo",
        "Kilo Code" => "Kilo",
        "Qwen Code" => "Qwen",
        "Q CLI" => "Q",
        _ => name, // Fallback to original name if not matched
    }
}

// Get last message preview
fn get_last_message_preview(
    stats: &AgenticCodingToolStats,
    max_len: usize,
) -> (String, Vec<(crate::types::MessageRole, String, String)>) {
    let username = get_username();
    let simplified_name = simplify_analyzer_name(&stats.analyzer_name);

    // First, filter messages to only those with displayable content
    let mut messages_with_content: Vec<_> = stats
        .messages
        .iter()
        .filter_map(|msg| {
            if let Some(content) = &msg.content {
                let single_line_content = content.replace('\n', " ").replace('\r', "");
                if !single_line_content.trim().is_empty() {
                    let msg_role_name = match msg.role {
                        crate::types::MessageRole::User => username.as_str(),
                        crate::types::MessageRole::Assistant => simplified_name,
                    };
                    return Some((
                        msg.date,
                        msg.role.clone(),
                        msg_role_name.to_string(),
                        single_line_content,
                    ));
                }
            }
            None
        })
        .collect();

    // Sort by date (newest first) and take last 5
    messages_with_content.sort_by_key(|(date, _, _, _)| std::cmp::Reverse(*date));
    messages_with_content.truncate(5);

    // Reverse to show oldest to newest
    messages_with_content.reverse();

    // Convert to the expected format
    let message_lines: Vec<_> = messages_with_content
        .into_iter()
        .map(|(_, role, role_name, content)| (role, role_name, content))
        .collect();

    // Get the role of the last message for status display
    if let Some(last_msg) = stats.messages.iter().max_by_key(|msg| msg.date) {
        let role = match last_msg.role {
            crate::types::MessageRole::User => username.as_str(),
            crate::types::MessageRole::Assistant => simplified_name,
        };
        (role.to_string(), message_lines)
    } else {
        ("—".to_string(), vec![])
    }
}

// Draw visual CLI panels section (compact 2-line design)
fn draw_visual_cli_panels(
    frame: &mut Frame,
    area: Rect,
    filtered_stats: &[&AgenticCodingToolStats],
    cli_data: &[(
        String,
        u64,
        u64,
        u64,
        u64,
        f64,
        String,
        String,
        u64,
        u64,
        String,
    )],
    cli_order: &[usize],
    _format_options: &NumberFormatOptions,
    tui_state: &mut TuiState,
) {
    if filtered_stats.is_empty() {
        return;
    }

    let mut lines = Vec::new();

    // Add blank line for spacing
    lines.push(Line::from(""));

    // Show 5 messages per CLI for better detail
    let messages_per_cli = 5;

    // Calculate how many CLIs can fit in the visible area
    let lines_per_cli = 1 + messages_per_cli + 1; // header + messages + spacing
    let available_height = area.height.saturating_sub(2) as usize; // Account for title/borders
    let max_visible_clis = (available_height / lines_per_cli.max(1)).max(1);

    let total_clis = cli_order.len();
    let start = tui_state.cli_scroll_offset.min(total_clis);
    let end = (start + max_visible_clis).min(total_clis);

    // Render visible CLI entries with scrolling support
    for &ordered_idx in &cli_order[start..end] {
        if ordered_idx >= filtered_stats.len() || ordered_idx >= cli_data.len() {
            continue;
        }
        let stats = filtered_stats[ordered_idx];
        let (
            cli_name,
            _cached,
            _input,
            _output,
            _reasoning,
            cost,
            _idle,
            _active,
            sessions,
            messages,
            state,
        ) = &cli_data[ordered_idx];

        // Line 1: CLI name, state, activity sparkline
        let sparkline = create_activity_sparkline(stats);
        let (last_role, last_time) = get_last_message_preview(stats, 50);

        let name_color = match state.as_str() {
            s if s.contains("Active") => Color::Green,
            s if s.contains("Waiting") => Color::Yellow,
            s if s.contains("Processing") => Color::Blue,
            s if s.contains("Idle") => Color::DarkGray,
            _ => Color::Gray,
        };

        // Build the line with colored sparkline spans
        let mut line_spans = vec![
            Span::styled(
                format!("{:12}", cli_name),
                Style::default().fg(name_color).bold(),
            ),
            Span::raw(" "),
        ];
        line_spans.extend(sparkline);
        line_spans.extend(vec![
            Span::raw("   "),
            Span::styled(
                state.chars().next().unwrap_or('⚫').to_string(),
                Style::default(),
            ),
            Span::raw(" "),
            Span::styled(_idle.clone(), Style::default().fg(Color::DarkGray)),
        ]);
        lines.push(Line::from(line_spans));

        // Add separator line below the sparkline graph (to end of content)
        // CLI name (12) + space + sparkline (80) + spacing + status + timestamp = ~109
        let line_width = 109;
        lines.push(Line::from(Span::styled(
            "─".repeat(line_width),
            Style::default().fg(Color::DarkGray),
        )));

        // Show messages, each limited to first line only
        let preview_width = area.width.saturating_sub(15) as usize;

        // Show messages based on dynamic limit with role-based styling
        for (role, role_name, content) in last_time.iter().take(messages_per_cli) {
            // Different styling for user vs assistant messages
            let (role_color, content_style) = match role {
                crate::types::MessageRole::User => (
                    Color::Cyan,                       // Bright cyan for user name
                    Style::default().fg(Color::White), // White for user message content
                ),
                crate::types::MessageRole::Assistant => (
                    Color::DarkGray,                               // Dim gray for assistant name
                    Style::default().fg(Color::DarkGray).italic(), // Dim italic for assistant content
                ),
            };

            // Format role and content separately
            let role_with_colon = format!("{}: ", role_name);
            let full_text_len = role_with_colon.len() + content.len();

            let content_text = if full_text_len > preview_width {
                let available_for_content = preview_width.saturating_sub(role_with_colon.len() + 1);
                if available_for_content > 0 && content.len() > available_for_content {
                    format!("{}…", &content[..available_for_content])
                } else {
                    content.clone()
                }
            } else {
                content.clone()
            };

            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(role_with_colon, Style::default().fg(role_color).bold()),
                Span::styled(content_text, content_style),
            ]));
        }

        // Add blank line for spacing between CLI entries
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title("📊 Live Activity")
            .title_style(Style::default().bold().fg(Color::Cyan)),
    );

    frame.render_widget(paragraph, area);
}

#[derive(Default, Clone)]
struct AggregatedStats {
    cost: f64,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    reasoning_tokens: u64,
    tool_calls: u64,
    conversations: u64,
}

#[derive(Clone)]
struct SummaryData {
    today_stats: AggregatedStats,
    yesterday_stats: AggregatedStats,
    week_stats: AggregatedStats,
    two_week_stats: AggregatedStats,
    selected_day_stats: AggregatedStats,
    selected_day_offset: usize,
    active_clis: usize,
}

impl AggregatedStats {
    fn add_day(&mut self, day_stats: &crate::types::DailyStats) {
        self.cost += day_stats.stats.cost;
        self.input_tokens += day_stats.stats.input_tokens;
        self.output_tokens += day_stats.stats.output_tokens;
        self.cached_tokens += day_stats.stats.cached_tokens;
        self.reasoning_tokens += day_stats.stats.reasoning_tokens;
        self.tool_calls += day_stats.stats.tool_calls as u64;
        self.conversations += day_stats.conversations as u64;
    }
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

use crate::notifications::{last_notification_age_seconds, next_notification_eta_seconds};
use crate::types::AgenticCodingToolStats;
use crate::utils::{
    NumberFormatOptions, calculate_overall_health, format_number, format_timestamp_for_live_view,
    get_health_color, get_health_status, get_warnings,
};
use chrono::Duration as ChronoDuration;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};
use std::collections::HashMap;

#[derive(Default, Clone)]
pub struct AggregatedStats {
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    pub tool_calls: u64,
    pub conversations: u64,
}

#[derive(Clone)]
pub struct SummaryData {
    pub today_stats: AggregatedStats,
    pub yesterday_stats: AggregatedStats,
    pub week_stats: AggregatedStats,
    pub two_week_stats: AggregatedStats,
    pub thirty_day_stats: AggregatedStats,
    #[allow(dead_code)]
    pub selected_day_stats: AggregatedStats,
    pub selected_day_offset: usize,
    pub active_clis: usize,
    // Cached sparklines and ring buffers to avoid expensive recalculation on every redraw
    pub sparkline_cache: HashMap<String, Vec<Span<'static>>>,
    pub sparkline_max_cache: HashMap<String, u64>,
    pub sparkline_buffers: HashMap<String, SparklineState>,
    // Cached tokens per second to avoid recalculating on every redraw
    pub tokens_per_second: f64,
}

pub fn build_activity_sparkline_cache(
    filtered_stats: &[&AgenticCodingToolStats],
    render_time: chrono::DateTime<chrono::Utc>,
) -> (
    HashMap<String, Vec<Span<'static>>>,
    HashMap<String, u64>,
    HashMap<String, SparklineState>,
) {
    build_activity_sparkline_cache_with_prev(filtered_stats, render_time, None, None)
}

pub fn build_activity_sparkline_cache_with_prev(
    filtered_stats: &[&AgenticCodingToolStats],
    render_time: chrono::DateTime<chrono::Utc>,
    prev_max: Option<&HashMap<String, u64>>,
    prev_buffers: Option<&HashMap<String, SparklineState>>,
) -> (
    HashMap<String, Vec<Span<'static>>>,
    HashMap<String, u64>,
    HashMap<String, SparklineState>,
) {
    let mut sparkline_cache = HashMap::new();
    let mut max_cache = HashMap::new();
    let mut buffer_cache = HashMap::new();
    for stats in filtered_stats {
        let prev = prev_max.and_then(|m| m.get(&stats.analyzer_name)).copied();
        let prev_buf = prev_buffers
            .and_then(|m| m.get(&stats.analyzer_name))
            .cloned();
        let (sparkline, stable_max, state) =
            create_activity_sparkline_with_prev(stats, render_time, prev, prev_buf);
        sparkline_cache.insert(stats.analyzer_name.clone(), sparkline);
        max_cache.insert(stats.analyzer_name.clone(), stable_max);
        buffer_cache.insert(stats.analyzer_name.clone(), state);
    }
    (sparkline_cache, max_cache, buffer_cache)
}

impl AggregatedStats {
    pub fn add_day(&mut self, day_stats: &crate::types::DailyStats) {
        self.cost += day_stats.stats.cost;
        self.input_tokens += day_stats.stats.input_tokens;
        self.output_tokens += day_stats.stats.output_tokens;
        self.cached_tokens += day_stats.stats.cached_tokens;
        self.reasoning_tokens += day_stats.stats.reasoning_tokens;
        self.tool_calls += day_stats.stats.tool_calls as u64;
        self.conversations += day_stats.conversations as u64;
    }
}

/// Calculate 15-minute moving average of output tokens per second
pub fn calculate_tokens_per_second(
    filtered_stats: &[&AgenticCodingToolStats],
    render_time: chrono::DateTime<chrono::Utc>,
) -> f64 {
    let fifteen_minutes_ago = render_time - ChronoDuration::minutes(15);

    // Collect all assistant messages from the last 15 minutes
    let mut total_output_tokens: u64 = 0;

    for analyzer_stats in filtered_stats {
        for message in &analyzer_stats.messages {
            // Only count assistant messages (they have output tokens)
            if message.role == crate::types::MessageRole::Assistant
                && message.date >= fifteen_minutes_ago
                && message.date <= render_time
            {
                total_output_tokens += message.stats.output_tokens;
            }
        }
    }

    // Calculate tokens per second over the 15-minute window
    // Always divide by full 15 minutes (900 seconds) for a true moving average
    let time_window_seconds = 15.0 * 60.0; // 900 seconds

    if total_output_tokens == 0 {
        0.0
    } else {
        total_output_tokens as f64 / time_window_seconds
    }
}

pub fn calculate_summary_data(
    filtered_stats: &[&AgenticCodingToolStats],
    day_offset: usize,
) -> SummaryData {
    // Calculate date ranges
    let now = chrono::Local::now();
    let today_start = now.date_naive();
    let yesterday_start = (now - ChronoDuration::days(1)).date_naive();
    let week_ago = (now - ChronoDuration::days(7)).date_naive();
    let two_weeks_ago = (now - ChronoDuration::days(14)).date_naive();
    let thirty_days_ago = (now - ChronoDuration::days(30)).date_naive();
    let selected_day = (now - ChronoDuration::days(day_offset as i64)).date_naive();

    // Aggregate data for each time period
    let mut today_stats = AggregatedStats::default();
    let mut yesterday_stats = AggregatedStats::default();
    let mut week_stats = AggregatedStats::default();
    let mut two_week_stats = AggregatedStats::default();
    let mut thirty_day_stats = AggregatedStats::default();
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
                if date >= thirty_days_ago {
                    thirty_day_stats.add_day(day_stats);
                }
                if date == selected_day {
                    selected_day_stats.add_day(day_stats);
                }
            }
        }
    }

    // Pre-calculate sparklines for all CLIs to cache them
    let render_time = chrono::Utc::now();
    let (sparkline_cache, sparkline_max_cache, sparkline_buffers) =
        build_activity_sparkline_cache(filtered_stats, render_time);

    // Pre-calculate tokens per second to cache it
    let tokens_per_second = calculate_tokens_per_second(filtered_stats, render_time);

    SummaryData {
        today_stats,
        yesterday_stats,
        week_stats,
        two_week_stats,
        thirty_day_stats,
        selected_day_stats,
        selected_day_offset: day_offset,
        active_clis: filtered_stats.len(),
        sparkline_cache,
        sparkline_max_cache,
        sparkline_buffers,
        tokens_per_second,
    }
}

pub fn draw_summary_view(
    frame: &mut Frame,
    area: Rect,
    summary_data: &SummaryData,
    format_options: &NumberFormatOptions,
    filtered_stats: &[&AgenticCodingToolStats],
    tui_state: &mut crate::tui::TuiState,
    render_time_utc: chrono::DateTime<chrono::Utc>,
    render_time_system: std::time::SystemTime,
) {
    // Load config for health display style
    let config = crate::config::Config::load()
        .unwrap_or(None)
        .unwrap_or_default();
    let use_braille = config.formatting.health_display_style == "braille";
    let SummaryData {
        today_stats,
        yesterday_stats,
        week_stats,
        two_week_stats,
        thirty_day_stats,
        selected_day_stats: _,
        selected_day_offset,
        active_clis,
        sparkline_cache: _,     // Accessed directly from summary_data below
        sparkline_max_cache: _, // Accessed directly from summary_data below
        sparkline_buffers: _,   // Accessed directly from summary_data below
        tokens_per_second: _,   // Accessed directly from summary_data below
    } = summary_data;

    // Split area into parts: spacing + overview table + spacing + CLI breakdown table + visual panels
    let chunks = Layout::vertical([
        Constraint::Length(1),  // Space above overview
        Constraint::Length(10), // Overview table (header + 6 rows + border) - reduced by 2 after removing cached/reasoning
        Constraint::Length(1),  // Space between tables
        Constraint::Length(14), // CLI breakdown table (fixed height)
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
        Cell::new(Text::from("30 Days").right_aligned()),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .height(1);

    let rows = vec![
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
            Cell::new(
                Line::from(format_number(thirty_day_stats.input_tokens, format_options))
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
            Cell::new(
                Line::from(format_number(thirty_day_stats.output_tokens, format_options))
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
            Cell::new(
                Line::from(format_number(thirty_day_stats.tool_calls, format_options))
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
            Cell::new(
                Line::from(format_number(thirty_day_stats.conversations, format_options))
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
            Cell::new(Line::from(format!("${:.2}", thirty_day_stats.cost)).right_aligned()),
        ]),
    ];

    // Split overview area: table on left, chart on right
    let overview_chunks = Layout::horizontal([
        Constraint::Length(82), // Table (15 + 13*5 + 2*5 spacing)
        Constraint::Length(2),  // Spacing between table and chart
        Constraint::Min(40),    // Chart (30 bars + borders + scale + stats) - use Min to expand as needed
    ])
    .split(chunks[1]);

    let table = Table::new(
        rows,
        [
            Constraint::Length(15), // Metric
            Constraint::Length(13), // Today - matches individual analyzer cached column width
            Constraint::Length(13), // Yesterday - matches individual analyzer cached column width
            Constraint::Length(13), // 7 Days - matches individual analyzer cached column width
            Constraint::Length(13), // 14 Days - matches individual analyzer cached column width
            Constraint::Length(13), // 30 Days - matches individual analyzer cached column width
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("📈 Overview")
            .title_style(Style::default().bold()),
    )
    .column_spacing(2);

    frame.render_widget(table, overview_chunks[0]);

    // Split chart area into two: total tokens (left) and output tokens (right)
    let chart_area = overview_chunks[2];
    let chart_chunks = Layout::horizontal([
        Constraint::Percentage(50), // Total tokens chart
        Constraint::Percentage(50), // Output tokens chart
    ])
    .split(chart_area);

    // Draw total tokens chart on the left
    draw_total_tokens_chart(
        frame,
        chart_chunks[0],
        filtered_stats,
        format_options,
    );

    // Draw output tokens chart on the right
    draw_output_tokens_chart(
        frame,
        chart_chunks[1],
        filtered_stats,
        format_options,
    );

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
        let now_utc = render_time_utc; // Use cached time instead of syscall
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
        // Streak row (replacing Cached Tokens)
        {
            let mut cells = vec![Cell::new(
                Line::from("🔥 Streak").style(Style::default().fg(Color::Red)),
            )];
            for &idx in visible_indices {
                let analyzer_stats = &filtered_stats[idx];

                // Count days in the last 30 days that have data
                let now = chrono::Local::now().date_naive();
                let thirty_days_ago = now - chrono::Duration::days(29); // Use 29 for exactly 30 days inclusive

                let days_with_data = analyzer_stats
                    .daily_stats
                    .iter()
                    .filter(|(date_str, _)| {
                        if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                            date >= thirty_days_ago && date <= now
                        } else {
                            false
                        }
                    })
                    .count();

                cells.push(Cell::new(
                    Line::from(format!("{}", days_with_data)).right_aligned(),
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
        // Health row
        {
            let mut cells = vec![Cell::new(
                Line::from("⚡ Health").style(Style::default().fg(Color::Green)),
            )];
            for &idx in visible_indices {
                let analyzer_stats = &filtered_stats[idx];
                let today = chrono::Local::now()
                    .date_naive()
                    .format("%Y-%m-%d")
                    .to_string();

                // Calculate health for this specific CLI
                let cli_messages: Vec<_> = analyzer_stats
                    .messages
                    .iter()
                    .filter(|msg| {
                        msg.date
                            .with_timezone(&chrono::Local)
                            .date_naive()
                            .format("%Y-%m-%d")
                            .to_string()
                            == today
                    })
                    .cloned()
                    .collect();

                let health = if cli_messages.is_empty() {
                    1.0 // No usage today = full health
                } else {
                    calculate_overall_health(&cli_messages, &today)
                };

                if use_braille {
                    // Create horizontal health bar (13 characters wide to fill the full column)
                    let braille_spans = create_braille_health_bar(health, 13);
                    cells.push(Cell::new(Line::from(braille_spans).right_aligned()));
                } else {
                    // Text display
                    let health_percent = (health * 100.0) as u32;
                    let health_status = get_health_status(health);
                    let health_color = match get_health_color(health) {
                        "green" => Color::Green,
                        "yellow" => Color::Yellow,
                        "orange" => Color::LightRed,
                        "red" => Color::Red,
                        _ => Color::Gray,
                    };

                    cells.push(Cell::new(
                        Line::from(format!("{}% {}", health_percent, health_status))
                            .style(Style::default().fg(health_color))
                            .right_aligned(),
                    ));
                }
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
        "📊 Today".to_string()
    } else {
        // Format the selected date as "Weekday, Month Day, Year"
        let selected_date_with_tz = now - ChronoDuration::days(*selected_day_offset as i64);
        let weekday = selected_date_with_tz.format("%A").to_string(); // Monday, Tuesday, etc.
        let formatted_date = selected_date_with_tz.format("%B %d, %Y").to_string(); // November 15, 2025
        format!(
            "📊 {} ({}, {} days ago)",
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
        render_time_utc,
        render_time_system,
        summary_data, // Pass summary_data to access sparkline cache
    );
}

// Helper function to create a horizontal bar visualization
#[allow(dead_code)]
pub fn create_bar(value: u64, max_value: u64, width: usize, color: Color) -> Line<'static> {
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
#[allow(dead_code)]
pub fn create_percentage_bar(
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

// Helper function to create ultra-granular Braille health bar (104 dots total)
// Health bars deplete from left to right (fill from right to left)
pub fn create_braille_health_bar(health: f64, width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    // Total dots available: 13 chars × 8 dots = 104 dots
    // Each 1% health = 1 dot
    let total_dots = width * 8;
    let dots_to_show = (health * total_dots as f64).round() as usize;
    let dots_to_show = dots_to_show.max(4); // Minimum 4 dots (single column)

    // Calculate full characters + partial character at boundary
    let full_chars = dots_to_show / 8; // Number of completely filled characters
    let remaining_dots = dots_to_show % 8; // Dots for the partial character

    // Braille characters for partial filling (right-to-left within character)
    // These fill from right side first, so bar depletes from left
    let partial_chars = [
        ' ', // 0 dots
        '⢀', // 1 dot (top-right)
        '⢠', // 2 dots (top-right + middle-right)
        '⢰', // 3 dots (top-right + middle-right + bottom-right)
        '⢸', // 4 dots (all right column)
        '⣸', // 5 dots (right column + top-left)
        '⣼', // 6 dots (right column + top-left + middle-left)
        '⣾', // 7 dots (all but bottom-left)
    ];

    // Calculate how many empty spaces to add at the left
    let empty_chars = width - full_chars - if remaining_dots > 0 { 1 } else { 0 };

    // Build the bar: empty spaces (left) -> partial char -> full chars (right)
    for i in 0..width {
        let ch = if i < empty_chars {
            // Leftmost characters are empty (health depleted)
            ' '
        } else if i == empty_chars && remaining_dots > 0 {
            // Transition character with partial filling
            partial_chars[remaining_dots]
        } else {
            // Rightmost characters are fully filled
            '⣿'
        };

        // Color based on health level
        let color = if health >= 0.8 {
            Color::Green
        } else if health >= 0.6 {
            Color::Yellow
        } else if health >= 0.4 {
            Color::LightRed
        } else {
            Color::Red
        };

        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
    }

    spans
}

// Helper to create activity sparkline for last hour
#[allow(dead_code)]
pub fn create_activity_sparkline(
    stats: &AgenticCodingToolStats,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Span<'static>> {
    create_activity_sparkline_with_prev(stats, now, None, None).0
}

#[derive(Clone)]
pub struct SparklineState {
    pub buckets: Vec<u64>, // fixed-length ring buffer (120 slots)
    pub last_slot_start: chrono::DateTime<chrono::Utc>,
}

pub fn create_activity_sparkline_with_prev(
    stats: &AgenticCodingToolStats,
    now: chrono::DateTime<chrono::Utc>,
    prev_max: Option<u64>,
    prev_state: Option<SparklineState>,
) -> (Vec<Span<'static>>, u64, SparklineState) {
    // Create 120 buckets (30-second intervals for last hour)
    let mut state = if let Some(mut prev) = prev_state {
        // Align the buffer forward based on elapsed time since last_slot_start
        let slot_len = chrono::Duration::seconds(30);
        let mut slots_to_advance =
            ((now - prev.last_slot_start).num_seconds() / 30).max(0) as usize;
        if slots_to_advance > 0 {
            slots_to_advance = slots_to_advance.min(120);
            prev.buckets.rotate_left(slots_to_advance);
            for b in prev.buckets.iter_mut().rev().take(slots_to_advance) {
                *b = 0;
            }
            prev.last_slot_start = prev.last_slot_start + slot_len * (slots_to_advance as i32);
        }
        prev
    } else {
        SparklineState {
            buckets: vec![0u64; 120],
            last_slot_start: now - chrono::Duration::seconds(30 * 120), // align in the past
        }
    };

    // Clear buckets before rebuilding from current window to avoid double-counting
    for b in state.buckets.iter_mut() {
        *b = 0;
    }

    // Only add messages that fall into the currently covered window
    let window_start = now - chrono::Duration::seconds(30 * 120);
    for msg in &stats.messages {
        if msg.date < window_start || msg.date > now {
            continue;
        }
        let age_secs = now.signed_duration_since(msg.date).num_seconds().max(0);
        let bucket_idx = (age_secs / 30) as usize;
        if bucket_idx < 120 {
            let total_tokens =
                msg.stats.input_tokens + msg.stats.output_tokens + msg.stats.reasoning_tokens;
            let target_idx = state
                .buckets
                .len()
                .saturating_sub(1)
                .saturating_sub(bucket_idx);
            if total_tokens > 0 {
                state.buckets[target_idx] += total_tokens;
            } else {
                state.buckets[target_idx] += 1;
            }
        }
    }

    // Find max for scaling. Freeze scale once established so older bars never rescale.
    // New peaks will simply hit the ceiling rather than shrinking prior bars.
    let raw_max = *state.buckets.iter().max().unwrap_or(&1).max(&1);
    let stable_max = prev_max.unwrap_or(raw_max).max(1);

    // Create sparkline with braille ramps (truncate to 80 most recent)
    let chars = [' ', '⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷', '⣿'];
    let spans = state
        .buckets
        .iter()
        .skip(state.buckets.len().saturating_sub(80)) // Show the most recent 80 buckets (40 minutes)
        .map(|&count| {
            if count == 0 {
                Span::raw(" ")
            } else {
                let capped = count.min(stable_max);
                let ratio = capped as f64 / stable_max as f64;
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
        .collect();

    (spans, stable_max, state)
}

// Helper function to get the current username
pub fn get_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "You".to_string())
}

// Helper function to simplify analyzer names to single words
pub fn simplify_analyzer_name(name: &str) -> &str {
    match name {
        "Claude Code" => "Claude",
        "Codex CLI" => "Codex",
        "Gemini CLI" => "Gemini",
        "GitHub Copilot" => "Copilot",
        " Cline" => " Cline",
        "Roo Code" => "Roo",
        "Kilo Code" => "Kilo",
        "Qwen Code" => "Qwen",
        "Q CLI" => "Q",
        _ => name, // Fallback to original name if not matched
    }
}

// Get last message preview
pub fn get_last_message_preview(
    stats: &AgenticCodingToolStats,
    _max_len: usize,
) -> (
    String,
    Vec<(
        chrono::DateTime<chrono::Utc>,
        crate::types::MessageRole,
        String,
        String,
        String,
        crate::types::Application,
    )>,
) {
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
                        msg.project_hash.clone(),
                        msg.application.clone(),
                    ));
                }
            }
            None
        })
        .collect();

    // Sort by date (newest first) and take last 5
    messages_with_content.sort_by_key(|(date, _, _, _, _, _)| std::cmp::Reverse(*date));
    messages_with_content.truncate(5);

    // Reverse to show oldest to newest
    messages_with_content.reverse();

    // Convert to the expected format
    let message_lines: Vec<_> = messages_with_content
        .into_iter()
        .map(
            |(date, role, role_name, content, project_hash, application)| {
                (date, role, role_name, content, project_hash, application)
            },
        )
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
pub fn draw_visual_cli_panels(
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
    tui_state: &mut crate::tui::TuiState,
    render_time_utc: chrono::DateTime<chrono::Utc>,
    render_time_system: std::time::SystemTime,
    summary_data: &SummaryData, // Access to sparkline cache
) {
    if filtered_stats.is_empty() {
        return;
    }

    let mut lines = Vec::new();

    // Add warnings if any
    let warnings = get_warnings();
    if !warnings.is_empty() {
        // Add blank line before warnings section for extra spacing
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "⚠️ Warnings:",
            Style::default().fg(Color::Yellow).bold(),
        )));
        for warning in warnings.iter().take(5) {
            lines.push(Line::from(Span::styled(
                format!("  {}", warning),
                Style::default().fg(Color::Yellow),
            )));
        }
        if warnings.len() > 5 {
            lines.push(Line::from(Span::styled(
                format!("  ... and {} more warnings", warnings.len() - 5),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::from(""));
    }

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
            _cost,
            _idle,
            _active,
            _sessions,
            _messages,
            state,
        ) = &cli_data[ordered_idx];

        // Line 1: CLI name, state, activity sparkline
        // Use cached sparkline instead of recalculating every second
        let sparkline = summary_data
            .sparkline_cache
            .get(&stats.analyzer_name)
            .cloned()
            .unwrap_or_else(|| {
                let prev = summary_data
                    .sparkline_max_cache
                    .get(&stats.analyzer_name)
                    .copied();
                let prev_state = summary_data
                    .sparkline_buffers
                    .get(&stats.analyzer_name)
                    .cloned();
                create_activity_sparkline_with_prev(stats, render_time_utc, prev, prev_state).0
            });
        let (_last_role, message_lines) = get_last_message_preview(stats, 50);

        let name_color = match state.as_str() {
            s if s.contains("Active") => Color::Green,
            s if s.contains("Waiting") => Color::Yellow,
            s if s.contains("Processing") => Color::Blue,
            s if s.contains("Idle") => Color::DarkGray,
            _ => Color::Gray,
        };

        // Build the line with colored sparkline spans
        let mut line_spans = vec![];

        // In verbose mode, show model name next to CLI name
        if tui_state.summary_verbose_mode {
            // Find the most recently used model for this CLI (more responsive to model switches)
            let most_recent_model = {
                // Find the message with the most recent timestamp that has a model
                let recent_msg_with_model = stats
                    .messages
                    .iter()
                    .filter(|msg| msg.model.is_some())
                    .max_by_key(|msg| msg.date.timestamp());

                match recent_msg_with_model {
                    Some(msg) => msg.model.as_ref().unwrap().clone(),
                    None => {
                        // Fallback to most commonly used model if no recent model found
                        stats
                            .messages
                            .iter()
                            .filter_map(|msg| msg.model.as_ref())
                            .fold(std::collections::HashMap::new(), |mut acc, model| {
                                *acc.entry(model.clone()).or_insert(0) += 1;
                                acc
                            })
                            .into_iter()
                            .max_by_key(|(_, count)| *count)
                            .map(|(model, _)| model)
                            .unwrap_or_else(|| String::from("—"))
                    }
                }
            };

            // Scrolling animation for long model names
            let model_display = if most_recent_model.len() > 15 {
                // Use time-based scrolling animation with cached time
                let now_millis = render_time_system
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;

                // Scroll position updates every 0.3 seconds, wrapping around
                let max_width = 15;
                let scroll_speed = 1; // characters per interval
                let scroll_interval_millis = 300; // milliseconds per scroll step

                let scroll_position = ((now_millis / scroll_interval_millis) * scroll_speed)
                    as usize
                    % (most_recent_model.len() + 3);

                // Create scrolling window with padding
                let padded_text = format!("{}   {}", most_recent_model, most_recent_model); // Add spacing and repeat
                let chars: Vec<char> = padded_text.chars().collect();

                // Extract visible window
                let visible: String = chars
                    .iter()
                    .cycle()
                    .skip(scroll_position)
                    .take(max_width)
                    .collect();

                visible
            } else {
                most_recent_model.clone()
            };

            line_spans.push(Span::styled(
                format!("{:12}", cli_name),
                Style::default().fg(name_color).bold(),
            ));
            // Bracket closes after model name, then pad to fixed width for alignment
            line_spans.push(Span::styled(
                format!(" [{}]", model_display),
                Style::default().fg(Color::DarkGray).italic(),
            ));
            // Add padding after bracket to align sparklines (total width: 1 + 2 + model_display + 1 = variable, pad to 19)
            let padding_needed = 19usize.saturating_sub(3 + model_display.len());
            line_spans.push(Span::raw(" ".repeat(padding_needed)));
            line_spans.push(Span::raw(" "));
        } else {
            line_spans.push(Span::styled(
                format!("{:12}", cli_name),
                Style::default().fg(name_color).bold(),
            ));
            line_spans.push(Span::raw(" "));
        }

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
        let line_width = area.width.saturating_sub(2) as usize;
        lines.push(Line::from(Span::styled(
            "─".repeat(line_width),
            Style::default().fg(Color::DarkGray),
        )));

        // Show messages, each limited to first line only
        let preview_width = area.width.saturating_sub(15) as usize;

        // Show messages based on dynamic limit with role-based styling
        let mut last_project_hash_displayed: Option<String> = None;

        for (timestamp, role, role_name, content, project_hash, application) in
            message_lines.iter().take(messages_per_cli)
        {
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

            let project_span = if tui_state.summary_verbose_mode && !project_hash.is_empty() {
                let project_label_text = if *application == crate::types::Application::ClaudeCode {
                    // For Claude Code, just truncate without adding a hash
                    let truncated = project_hash.chars().take(8).collect::<String>();
                    // Pad with spaces to ensure fixed width
                    format!("{:width$}", truncated, width = 8)
                } else {
                    crate::utils::truncate_project_label(project_hash, 8)
                };

                let is_changed = last_project_hash_displayed
                    .as_ref()
                    .map_or(false, |last| last != project_hash);
                last_project_hash_displayed = Some(project_hash.clone());

                let project_style = if is_changed {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD) // Highlight color
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                Span::styled(format!("#{} ", project_label_text), project_style) // Add space after hashtag
            } else {
                Span::raw("")
            };

            let timestamp_str = if tui_state.summary_verbose_mode {
                format!("{} ", format_timestamp_for_live_view(timestamp))
            } else {
                String::new()
            };

            let timestamp_style = if tui_state.summary_verbose_mode {
                match role {
                    crate::types::MessageRole::User => Style::default().fg(Color::White), // White for user timestamps
                    _ => Style::default().fg(Color::DarkGray),
                }
            } else {
                Style::default()
            };

            // Format role and content separately
            let role_with_colon = format!("{}: ", role_name);
            let full_text_len =
                project_span.width() + timestamp_str.len() + role_with_colon.len() + content.len();

            let content_text = if full_text_len > preview_width {
                let available_for_content = preview_width.saturating_sub(
                    project_span.width() + timestamp_str.len() + role_with_colon.len() + 1,
                );
                if available_for_content > 0 && content.len() > available_for_content {
                    // Use char-aware truncation to avoid panicking on multi-byte UTF-8 characters
                    let truncated: String = content.chars().take(available_for_content).collect();
                    format!("{}…", truncated)
                } else {
                    content.clone()
                }
            } else {
                content.clone()
            };

            lines.push(Line::from(vec![
                Span::raw("  "),
                project_span,
                Span::styled(timestamp_str, timestamp_style),
                Span::styled(role_with_colon, Style::default().fg(role_color).bold()),
                Span::styled(content_text, content_style),
            ]));
        }
        // Add blank line for spacing between CLI entries
        lines.push(Line::from(""));
    }

    let now = chrono::Local::now();
    let clock_text = now.format("%H:%M:%S").to_string();

    // Notification status (Slack + next ETA + last send)
    let config = crate::config::Config::load()
        .unwrap_or(None)
        .unwrap_or_default();
    let notif_cfg = &config.notifications;
    let notifications_enabled = notif_cfg.enabled
        && notif_cfg.slack.enabled
        && notif_cfg.slack.webhook_url.is_some();
    let waiting_secs = notif_cfg.waiting_seconds as i64;
    let stale_secs = (notif_cfg.stale_minutes * 60) as i64;

    let eta = if notifications_enabled {
        next_notification_eta_seconds(filtered_stats, waiting_secs, stale_secs)
    } else {
        None
    };
    let last_age = if notif_cfg.enabled {
        last_notification_age_seconds(90)
    } else {
        None
    };

    let slack_icon = if notifications_enabled { "✅" } else { "⬜" };
    
    // Create braille countdown indicator like the health bar
    let (countdown_bar, countdown_color, remaining_display) = if let Some(remaining) = eta {
        // eta is remaining time, so we want full when remaining=180s, empty when remaining=0s
        // This counts DOWN like health - depletes from left to right
        let progress = remaining as f64 / 180.0; // 180s = full, 0s = empty
        let bar_width = 16; // Width for countdown bar
        let total_dots = bar_width * 8;
        let dots_to_show = ((progress * total_dots as f64).round() as usize).max(4);
        
        // Calculate full characters + partial character at boundary
        let full_chars = dots_to_show / 8;
        let remaining_dots = dots_to_show % 8;
        
        // Braille characters for partial filling (right-to-left within character)
        // These fill from right side first, so bar depletes from left (counting down)
        let partial_chars = [
            ' ', // 0 dots
            '⢀', // 1 dot (top-right)
            '⢠', // 2 dots (top-right + middle-right)
            '⢰', // 3 dots (top-right + middle-right + bottom-right)
            '⢸', // 4 dots (all right column)
            '⣸', // 5 dots (right column + top-left)
            '⣼', // 6 dots (right column + top-left + middle-left)
            '⣾', // 7 dots (all but bottom-left)
        ];
        
        // Calculate how many empty spaces to add at the left
        let empty_chars = bar_width - full_chars - if remaining_dots > 0 { 1 } else { 0 };
        
        // Build the bar: empty spaces (left) -> partial char -> full chars (right)
        let mut bar_chars = Vec::new();
        for i in 0..bar_width {
            let ch = if i < empty_chars {
                ' ' // Leftmost characters are empty (time depleted)
            } else if i == empty_chars && remaining_dots > 0 {
                partial_chars[remaining_dots] // Transition character
            } else {
                '⣿' // Rightmost characters are fully filled
            };
            bar_chars.push(ch);
        }
        
        // Hot colors as it gets closer to zero (more urgent)
        let color = if remaining >= 120 {
            Color::Cyan // Cool - plenty of time
        } else if remaining >= 60 {
            Color::Green // Getting warmer
        } else if remaining >= 30 {
            Color::Yellow // Getting hot
        } else if remaining >= 15 {
            Color::LightRed // Very hot
        } else {
            Color::Red // EXTREME - about to send!
        };
        
        (bar_chars.into_iter().collect::<String>(), color, format!("{remaining:>3}s"))
    } else {
        ("                ".to_string(), Color::DarkGray, "  —".to_string())
    };
    
    let _countdown_indicator = format!("|{countdown_bar}| {remaining_display}");
    
    let last_icon = if last_age.is_some() { "✅" } else { "⬜" };
    let last_text = last_age
        .map(|s| format!("{s}s ago"))
        .unwrap_or_else(|| "—".to_string());
    
    // Fixed width sections to prevent position shifts - everything has fixed width
    // Slack: Slack ✅ = 8 chars (fixed)
    // Last: last ✅ 67s ago = 16 chars (fixed)
    // Countdown: |⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿| 108s = 30 chars
    // Total right section: 8 + 3 + 30 + 3 + 16 = 60 chars fixed
    
    let _slack_section = format!("| {} Slack", slack_icon); // Fixed 8 chars
    let next_section = format!("|{countdown_bar}| {remaining_display}"); // Fixed 25 chars with left boundary
    let last_section = format!("last {last_icon} {last_text:>7}"); // Fixed 16 chars for "XXs ago" or "—"

    // Use cached 15-minute moving average tokens per second (refreshed every 5 seconds)
    let tks_per_sec = summary_data.tokens_per_second;
    // Fixed-width format optimized for typical < 100 range: "42.3 tk/s", " 8.7 tk/s", " 0.7 tk/s"
    let tks_display = if tks_per_sec >= 100.0 {
        format!("{:.0} tk/s", tks_per_sec)           // "123 tk/s" (no decimal for 100+, rare)
    } else if tks_per_sec >= 1.0 {
        format!("{:>4.1} tk/s", tks_per_sec)         // "42.3 tk/s" or " 8.7 tk/s" (9 chars total)
    } else if tks_per_sec > 0.0 {
        format!("{:>4.1} tk/s", tks_per_sec)         // " 0.7 tk/s" (9 chars total)
    } else {
        "  — tk/s".to_string()                       // "  — tk/s" (9 chars total)
    };

    let left_text = format!("📊 Activity: {} | {}", clock_text, tks_display);
    
    // Build title line with fixed positioning - no dynamic width calculations
    let title_spans = vec![
        Span::raw(left_text),
        Span::raw("                 "), // Much larger spacer to push entire right section far right
        Span::styled(next_section, Style::default().fg(countdown_color)), // Countdown with color
        Span::raw(" | "), // Fixed separator
        Span::raw(last_section), // Fixed width last section
        Span::raw("     "), // Normal spacing before Slack
        Span::styled(
            slack_icon.to_string(), 
            Style::default().fg(if notifications_enabled { Color::Green } else { Color::DarkGray })
        ), // Colored checkmark only
        Span::raw(" Slack"), // Normal white text for "Slack"
    ];
    
    let title_line = Line::from(title_spans);

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(title_line)
            .title_style(Style::default().bold().fg(Color::Cyan)),
    );

    frame.render_widget(paragraph, area);
}

/// Draw total tokens chart (left side)
fn draw_total_tokens_chart(
    frame: &mut Frame,
    area: Rect,
    filtered_stats: &[&AgenticCodingToolStats],
    _format_options: &NumberFormatOptions,
) {
    use std::collections::BTreeMap;
    use ratatui::widgets::Borders;

    // Aggregate tokens by date across all analyzers
    let mut daily_tokens: BTreeMap<chrono::NaiveDate, u64> = BTreeMap::new();

    for stats in filtered_stats {
        for (date_str, day_stats) in &stats.daily_stats {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                let entry = daily_tokens.entry(date).or_insert(0);
                *entry += day_stats.stats.input_tokens + day_stats.stats.output_tokens;
            }
        }
    }

    // Get last 30 days (including today)
    let now = chrono::Local::now().date_naive();
    let thirty_days_ago = now - ChronoDuration::days(29);

    let mut chart_data: Vec<u64> = Vec::new();
    let mut max_total = 0u64;
    let mut min_total = u64::MAX;
    let mut sum_total = 0u64;
    let mut days_with_data = 0;

    // Collect last 30 days of data
    for i in 0..30 {
        let date = thirty_days_ago + ChronoDuration::days(i);
        let total = daily_tokens.get(&date).copied().unwrap_or(0);

        if total > 0 {
            max_total = max_total.max(total);
            min_total = min_total.min(total);
            sum_total += total;
            days_with_data += 1;
        }

        chart_data.push(total);
    }

    // If no data, show empty chart
    if max_total == 0 {
        let empty = Paragraph::new("No data")
            .block(Block::default()
                .title("Total Tokens")
                .title_style(Style::default().bold())
                .borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    }

    // Calculate average (only for days with data)
    let avg_total = if days_with_data > 0 {
        sum_total / days_with_data
    } else {
        0
    };

    if min_total == u64::MAX {
        min_total = 0;
    }

    // Create bar chart content
    let mut lines = Vec::new();

    // Determine bar height
    let chart_height = (area.height.saturating_sub(5)) as usize;

    if chart_height < 3 {
        let empty = Paragraph::new("Too small")
            .block(Block::default()
                .title("Total")
                .borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    }

    // Calculate Y-axis scale (labels only; rendering uses integer math)
    let scale = max_total as f64 / chart_height as f64;

    // Format token count
    let format_tokens = |t: u64| -> String {
        if t >= 1_000_000 {
            format!("{}M", t / 1_000_000)
        } else if t >= 1_000 {
            format!("{}K", t / 1_000)
        } else {
            format!("{}", t)
        }
    };

    // Dynamically size the Y-axis label column so longer labels (e.g., 60K)
    // don't push the bars to the right and cause visual gaps.
    let label_width = format_tokens(max_total).len().max(4);

    // Braille characters for bar rendering (bottom to top)
    let braille_chars = [' ', '⢀', '⢠', '⢰', '⢸', '⣸', '⣼', '⣾', '⣿'];

    // Precompute bar heights in subrows (8 per row) using integer math to avoid gaps
    let bar_subrows: Vec<usize> = chart_data
        .iter()
        .map(|&total| {
            if max_total == 0 {
                0
            } else {
                let numerator = total as u128 * 8 * chart_height as u128 + (max_total as u128 / 2);
                (numerator / max_total as u128) as usize
            }
        })
        .collect();

    // Draw bars from top to bottom with 8x granularity using braille
    for row in (0..chart_height).rev() {
        let mut spans = Vec::new();

        // Y-axis label
        if row == chart_height - 1 || row == chart_height / 2 || row == 0 {
            let label = format_tokens((row as f64 * scale) as u64);
            spans.push(Span::styled(
                format!("{:>width$}│", label, width = label_width),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(format!("{:>width$}│", "", width = label_width)));
        }

        // Draw bars with braille granularity
        for subrows in &bar_subrows {
            let full_rows = subrows / 8;
            let partial = subrows % 8;
            let row_from_bottom = row;

            let ch = if row_from_bottom < full_rows {
                braille_chars[8]
            } else if row_from_bottom == full_rows && partial > 0 {
                braille_chars[partial]
            } else {
                ' '
            };

            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(Color::Blue),
            ));
        }

        lines.push(Line::from(spans));
    }

    // X-axis with 7-day markers
    let mut x_axis = vec![Span::raw(format!("{:>width$}└", "", width = label_width))];
    for i in 0..30 {
        if i % 7 == 0 && i > 0 {
            x_axis.push(Span::styled("┴", Style::default().fg(Color::DarkGray)));
        } else {
            x_axis.push(Span::styled("─", Style::default().fg(Color::DarkGray)));
        }
    }
    lines.push(Line::from(x_axis));

    // X-axis labels
    let mut x_labels = vec![Span::raw(format!("{:>width$} ", "", width = label_width))];
    for i in 0..30 {
        if i % 7 == 0 {
            let date = thirty_days_ago + ChronoDuration::days(i);
            let date_label = date.format("%m/%d").to_string();
            x_labels.push(Span::styled(date_label, Style::default().fg(Color::DarkGray)));
            if i < 28 {
                x_labels.push(Span::raw("  "));
            }
        }
    }
    lines.push(Line::from(x_labels));

    // Stats line
    let stats = vec![
        Span::styled("Min:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(min_total), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled("Max:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(max_total), Style::default().fg(Color::Red)),
        Span::raw(" "),
        Span::styled("Avg:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(avg_total), Style::default().fg(Color::Yellow)),
    ];
    lines.push(Line::from(stats));

    let paragraph = Paragraph::new(lines)
        .block(Block::default()
            .title("📊 Total Tokens/Day")
            .title_style(Style::default().bold())
            .borders(Borders::ALL));

    frame.render_widget(paragraph, area);
}

/// Draw output tokens chart (right side)
fn draw_output_tokens_chart(
    frame: &mut Frame,
    area: Rect,
    filtered_stats: &[&AgenticCodingToolStats],
    _format_options: &NumberFormatOptions,
) {
    use std::collections::BTreeMap;
    use ratatui::widgets::Borders;

    // Aggregate output tokens by date across all analyzers
    let mut daily_output: BTreeMap<chrono::NaiveDate, u64> = BTreeMap::new();

    for stats in filtered_stats {
        for (date_str, day_stats) in &stats.daily_stats {
            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                let entry = daily_output.entry(date).or_insert(0);
                *entry += day_stats.stats.output_tokens;
            }
        }
    }

    // Get last 30 days (including today)
    let now = chrono::Local::now().date_naive();
    let thirty_days_ago = now - ChronoDuration::days(29);

    let mut chart_data: Vec<u64> = Vec::new();
    let mut max_output = 0u64;
    let mut min_output = u64::MAX;
    let mut sum_output = 0u64;
    let mut days_with_data = 0;

    // Collect last 30 days of data
    for i in 0..30 {
        let date = thirty_days_ago + ChronoDuration::days(i);
        let output = daily_output.get(&date).copied().unwrap_or(0);

        if output > 0 {
            max_output = max_output.max(output);
            min_output = min_output.min(output);
            sum_output += output;
            days_with_data += 1;
        }

        chart_data.push(output);
    }

    // If no data, show empty chart
    if max_output == 0 {
        let empty = Paragraph::new("No data")
            .block(Block::default()
                .title("Output Tokens")
                .title_style(Style::default().bold())
                .borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    }

    // Calculate average (only for days with data)
    let avg_output = if days_with_data > 0 {
        sum_output / days_with_data
    } else {
        0
    };

    if min_output == u64::MAX {
        min_output = 0;
    }

    // Create bar chart content
    let mut lines = Vec::new();

    // Determine bar height
    let chart_height = (area.height.saturating_sub(5)) as usize;

    if chart_height < 3 {
        let empty = Paragraph::new("Too small")
            .block(Block::default()
                .title("Output")
                .borders(Borders::ALL));
        frame.render_widget(empty, area);
        return;
    }

    // Calculate Y-axis scale (labels only; rendering uses integer math)
    let scale = max_output as f64 / chart_height as f64;

    // Format token count
    let format_tokens = |t: u64| -> String {
        if t >= 1_000_000 {
            format!("{}M", t / 1_000_000)
        } else if t >= 1_000 {
            format!("{}K", t / 1_000)
        } else {
            format!("{}", t)
        }
    };

    // Dynamically size the Y-axis label column so longer labels (e.g., 60K)
    // don't push the bars to the right and cause visual gaps.
    let label_width = format_tokens(max_output).len().max(4);

    // Braille characters for bar rendering (bottom to top)
    let braille_chars = [' ', '⢀', '⢠', '⢰', '⢸', '⣸', '⣼', '⣾', '⣿'];

    // Precompute bar heights in subrows (8 per row) using integer math to avoid gaps
    let bar_subrows: Vec<usize> = chart_data
        .iter()
        .map(|&output| {
            if max_output == 0 {
                0
            } else {
                let numerator = output as u128 * 8 * chart_height as u128 + (max_output as u128 / 2);
                (numerator / max_output as u128) as usize
            }
        })
        .collect();

    // Draw bars from top to bottom with 8x granularity using braille
    for row in (0..chart_height).rev() {
        let mut spans = Vec::new();

        // Y-axis label
        if row == chart_height - 1 || row == chart_height / 2 || row == 0 {
            let label = format_tokens((row as f64 * scale) as u64);
            spans.push(Span::styled(
                format!("{:>width$}│", label, width = label_width),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(format!("{:>width$}│", "", width = label_width)));
        }

        // Draw bars with braille granularity
        for subrows in &bar_subrows {
            let full_rows = subrows / 8;
            let partial = subrows % 8;
            let row_from_bottom = row;

            let ch = if row_from_bottom < full_rows {
                braille_chars[8]
            } else if row_from_bottom == full_rows && partial > 0 {
                braille_chars[partial]
            } else {
                ' '
            };

            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(Color::Green),
            ));
        }

        lines.push(Line::from(spans));
    }

    // X-axis with 7-day markers
    let mut x_axis = vec![Span::raw(format!("{:>width$}└", "", width = label_width))];
    for i in 0..30 {
        if i % 7 == 0 && i > 0 {
            x_axis.push(Span::styled("┴", Style::default().fg(Color::DarkGray)));
        } else {
            x_axis.push(Span::styled("─", Style::default().fg(Color::DarkGray)));
        }
    }
    lines.push(Line::from(x_axis));

    // X-axis labels
    let mut x_labels = vec![Span::raw(format!("{:>width$} ", "", width = label_width))];
    for i in 0..30 {
        if i % 7 == 0 {
            let date = thirty_days_ago + ChronoDuration::days(i);
            let date_label = date.format("%m/%d").to_string();
            x_labels.push(Span::styled(date_label, Style::default().fg(Color::DarkGray)));
            if i < 28 {
                x_labels.push(Span::raw("  "));
            }
        }
    }
    lines.push(Line::from(x_labels));

    // Stats line
    let stats = vec![
        Span::styled("Min:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(min_output), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled("Max:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(max_output), Style::default().fg(Color::Red)),
        Span::raw(" "),
        Span::styled("Avg:", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(format_tokens(avg_output), Style::default().fg(Color::Yellow)),
    ];
    lines.push(Line::from(stats));

    let paragraph = Paragraph::new(lines)
        .block(Block::default()
            .title("📤 Output Tokens/Day")
            .title_style(Style::default().bold())
            .borders(Borders::ALL));

    frame.render_widget(paragraph, area);
}

use crate::types::{AgenticCodingToolStats, Application};
use crate::utils::{calculate_overall_health, format_number, format_timestamp_for_live_view, get_health_color, get_health_status, get_warnings, NumberFormatOptions};
use chrono::Duration as ChronoDuration;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState, Tabs};
use ratatui::{Frame};
use std::collections::HashMap;
use std::io::{stdout, Write};
use crossterm::{ExecutableCommand, execute};
use crossterm::style::{Print, ResetColor, SetForegroundColor};

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
    pub selected_day_stats: AggregatedStats,
    pub selected_day_offset: usize,
    pub active_clis: usize,
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

pub fn draw_summary_view(
    frame: &mut Frame,
    area: Rect,
    summary_data: &SummaryData,
    format_options: &NumberFormatOptions,
    filtered_stats: &[&AgenticCodingToolStats],
    tui_state: &mut crate::tui::TuiState,
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
        Constraint::Length(12), // Overview table (header + 9 rows + border)
        Constraint::Length(1),  // Space between tables
        Constraint::Length(13), // CLI breakdown table (fixed height)
        Constraint::Length(1),  // Space before visual panels
        Constraint::Min(0),     // Visual CLI panels (takes remaining space)
    ])
    .split(area);

    // Store the rects for mouse handling
    tui_state.layout.today_by_cli_rect = Some(chunks[3]);
    tui_state.layout.live_activity_rect = Some(chunks[5]);

    // Calculate overall health
    let all_messages: Vec<_> = filtered_stats.iter().flat_map(|s| &s.messages).cloned().collect();
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let overall_health = calculate_overall_health(&all_messages, &today);
    let health_status = get_health_status(overall_health);
    let health_color = match get_health_color(overall_health) {
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "orange" => Color::LightRed,
        "red" => Color::Red,
        _ => Color::Gray,
    };

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
        Row::new(vec![
            Cell::new(Line::from("⚡ Health").style(Style::default().fg(health_color))),
            Cell::new(Line::from(format!("{:.0}% {}", overall_health * 100.0, health_status)).style(Style::default().fg(health_color)).right_aligned()),
            Cell::new(Line::raw("")),
            Cell::new(Line::raw("")),
            Cell::new(Line::raw("")),
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
            weekday,
            formatted_date,
            selected_day_offset,
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

// Helper to create activity sparkline for last hour
pub fn create_activity_sparkline(stats: &AgenticCodingToolStats) -> Vec<Span<'static>> {
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
    max_len: usize,
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
        .map(|(date, role, role_name, content, project_hash, application)| {
            (date, role, role_name, content, project_hash, application)
        })
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
            cost,
            _idle,
            _active,
            sessions,
            messages,
            state,
        ) = &cli_data[ordered_idx];

        // Line 1: CLI name, state, activity sparkline
        let sparkline = create_activity_sparkline(stats);
        let (_last_role, message_lines) = get_last_message_preview(stats, 50);

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
                    let mut truncated = project_hash.chars().take(8).collect::<String>();
                    // Pad with spaces to ensure fixed width
                    format!("{:width$}", truncated, width = 8)
                } else {
                    crate::utils::truncate_project_label(project_hash, 8)
                };

                let is_changed = last_project_hash_displayed.as_ref().map_or(false, |last| last != project_hash);
                last_project_hash_displayed = Some(project_hash.clone());

                let project_style = if is_changed {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) // Highlight color
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                Span::styled(format!("#{} ", project_label_text), project_style) // Add space after hashtag
            } else {
                Span::raw("")
            };

            let timestamp_str = if tui_state.summary_verbose_mode {
                format!(
                    "{} ",
                    format_timestamp_for_live_view(timestamp)
                )
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
                    format!("{}…", &content[..available_for_content])
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
    let title = format!("📊 Live Activity: {}", clock_text);

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .title(title)
            .title_style(Style::default().bold().fg(Color::Cyan)),
    );

    frame.render_widget(paragraph, area);
}
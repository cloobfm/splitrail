#[cfg(test)]
mod tests {
    use crate::utils::{hash_text, aggregate_by_date, format_number, warn_once, get_warnings, NumberFormatOptions, WARNINGS, WARNED_MESSAGES};
    use crate::types::{ConversationMessage, Stats, Application, MessageRole};
    use chrono::{DateTime, Utc};


    #[test]
    fn test_hash_text_consistency() {
        let text = "test message content";
        let hash1 = hash_text(text);
        let hash2 = hash_text(text);
        
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
        assert_eq!(hash1.len(), 64); // SHA256 produces 64-char hex string
    }

    #[test]
    fn test_hash_text_uniqueness() {
        let text1 = "test message content 1";
        let text2 = "test message content 2";
        
        let hash1 = hash_text(text1);
        let hash2 = hash_text(text2);
        
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_text_empty_string() {
        let hash = hash_text("");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64);
    }

    /// The day drill-down computes "active time" by summing gaps under 15 minutes between
    /// consecutive messages on that day. That is the only value it needs that is not already in
    /// DailyStats, and it is why the full message history is retained in RAM (BZL-14). Compute it
    /// during aggregation instead, so history can be dropped.
    #[test]
    fn aggregate_records_active_seconds_from_sub_15_minute_gaps() {
        fn at(hms: &str) -> ConversationMessage {
            let ts = format!("2024-01-01T{hms}Z");
            ConversationMessage {
                application: Application::ClaudeCode,
                date: ts.parse::<DateTime<Utc>>().unwrap(),
                project_hash: "p".into(),
                conversation_hash: "c".into(),
                local_hash: None,
                global_hash: hms.to_string(),
                model: None,
                stats: Stats::default(),
                role: MessageRole::User,
                content: None,
            }
        }

        // 12:00 -> 12:05 counts (300 s). 12:05 -> 12:40 is a 35 min gap, so it does not.
        // 12:40 -> 12:50 counts (600 s). Deliberately out of order to prove sorting happens.
        let messages = vec![at("12:40:00"), at("12:00:00"), at("12:50:00"), at("12:05:00")];

        let result = aggregate_by_date(&messages);
        let day = result
            .values()
            .find(|d| d.active_seconds > 0)
            .expect("the populated day should record active time");

        assert_eq!(day.active_seconds, 900, "300 s + 600 s, excluding the 35 min gap");
    }

    /// Only the last day or so of raw messages feeds the live views; everything older is already
    /// summarised in `daily_stats`. Trimming to that window is what takes RSS from ~500 MB to tens
    /// of MB (BZL-14) — and it must not disturb the aggregates.
    #[test]
    fn live_window_drops_old_messages_but_keeps_their_aggregates() {
        use crate::utils::{live_window_cutoff, retain_live_window};

        let now: DateTime<Utc> = "2024-06-01T12:00:00Z".parse().unwrap();
        let msg = |ts: &str| ConversationMessage {
            application: Application::ClaudeCode,
            date: ts.parse::<DateTime<Utc>>().unwrap(),
            project_hash: "p".into(),
            conversation_hash: "c".into(),
            local_hash: None,
            global_hash: ts.to_string(),
            model: None,
            stats: Stats::default(),
            role: MessageRole::User,
            content: None,
        };

        let messages = vec![
            msg("2024-01-15T09:00:00Z"), // ancient
            msg("2024-05-30T09:00:00Z"), // outside the 24h window
            msg("2024-06-01T09:00:00Z"), // inside
        ];
        let daily_stats = aggregate_by_date(&messages);
        let mut stats = crate::types::AgenticCodingToolStats {
            daily_stats: daily_stats.clone(),
            num_conversations: 1,
            messages,
            analyzer_name: "test".into(),
        };

        // No upload configured, so nothing pending has to be preserved.
        retain_live_window(&mut stats, live_window_cutoff(now, None));

        assert_eq!(stats.messages.len(), 1, "only the message inside the window survives");
        assert_eq!(stats.daily_stats, daily_stats, "aggregates are untouched");
        assert!(
            stats.daily_stats.contains_key("2024-01-15"),
            "history stays available in aggregate form"
        );
    }

    /// Trimming must never discard a message the uploader still owes the server.
    #[test]
    fn live_window_keeps_messages_the_uploader_has_not_sent() {
        use crate::utils::live_window_cutoff;

        let now: DateTime<Utc> = "2024-06-01T12:00:00Z".parse().unwrap();
        let watermark: DateTime<Utc> = "2024-03-01T00:00:00Z".parse().unwrap();

        let cutoff = live_window_cutoff(now, Some(watermark.timestamp_millis()));

        assert_eq!(cutoff, watermark, "a lagging watermark pulls the cutoff back");
    }

    #[test]
    fn test_aggregate_by_date_empty() {
        let messages = vec![];
        let result = aggregate_by_date(&messages);
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggregate_by_date_single_message() {
        let message = create_test_message("2024-01-01");
        let messages = vec![message];
        
        let result = aggregate_by_date(&messages);
        // aggregate_by_date fills every day from the earliest message through today with
        // empty stats so the TUI has a continuous series, so only check the populated day.
        assert!(result.contains_key("2024-01-01"));
        assert!(result.contains_key(&today_key()), "gaps are filled through today");
        
        let day_stats = &result["2024-01-01"];
        assert_eq!(day_stats.user_messages, 1);
        assert_eq!(day_stats.ai_messages, 0);
        assert_eq!(day_stats.conversations, 1);
    }

    #[test]
    fn test_aggregate_by_date_multiple_messages() {
        let messages = vec![
            create_test_message_with_role("2024-01-01", MessageRole::User),
            create_test_message_with_role("2024-01-01", MessageRole::Assistant),
            create_test_message_with_role("2024-01-02", MessageRole::User),
        ];
        
        let result = aggregate_by_date(&messages);
        assert!(result.contains_key("2024-01-01"));
        assert!(result.contains_key("2024-01-02"));
        
        let day1 = &result["2024-01-01"];
        assert_eq!(day1.user_messages, 1);
        assert_eq!(day1.ai_messages, 1);
        assert_eq!(day1.conversations, 1);
        
        // All three messages share one conversation hash, and a conversation is attributed to
        // the day it started, so day 2 has messages but no new conversation.
        let day2 = &result["2024-01-02"];
        assert_eq!(day2.user_messages, 1);
        assert_eq!(day2.ai_messages, 0);
        assert_eq!(day2.conversations, 0);
    }

    #[test]
    fn test_aggregate_by_date_stats_accumulation() {
        // Token/cost stats are only summed for AI messages (those with a model).
        let messages = vec![
            create_test_message_with_stats_and_role("2024-01-01", 1000, 500, 1.0, MessageRole::Assistant),
            create_test_message_with_stats_and_role("2024-01-01", 2000, 1000, 2.0, MessageRole::Assistant),
        ];
        
        let result = aggregate_by_date(&messages);
        let day_stats = &result["2024-01-01"];
        
        // 1000+2000 input, 500+1000 output, 1.0+2.0 cost, 1+1 tool calls
        assert_eq!(day_stats.stats.input_tokens, 3000, "Input tokens should be sum of all messages");
        assert_eq!(day_stats.stats.output_tokens, 1500, "Output tokens should be sum of all messages");
        assert_eq!(day_stats.stats.cost, 3.0, "Cost should be sum of all message costs");
        assert_eq!(day_stats.stats.tool_calls, 2, "Tool calls should be sum of all message tool calls");
    }

    #[test]
    fn test_aggregate_by_date_model_tracking() {
        let messages = vec![
            create_test_message_with_model("2024-01-01", "claude-3-sonnet"),
            create_test_message_with_model("2024-01-01", "claude-3-sonnet"),
            create_test_message_with_model("2024-01-01", "claude-3-haiku"),
        ];
        
        let result = aggregate_by_date(&messages);
        let day_stats = &result["2024-01-01"];
        
        assert_eq!(day_stats.models.len(), 2);
        assert_eq!(day_stats.models.get("claude-3-sonnet"), Some(&2));
        assert_eq!(day_stats.models.get("claude-3-haiku"), Some(&1));
    }

    #[test]
    fn test_format_number_basic() {
        let options = NumberFormatOptions::default();
        
        assert_eq!(format_number(0, &options), "0");
        assert_eq!(format_number(123, &options), "123");
        assert_eq!(format_number(1234567, &options), "1234567");
    }

    #[test]
    fn test_format_number_comma() {
        let options = NumberFormatOptions {
            use_comma: true,
            use_human: false,
            locale: "en".to_string(),
            decimal_places: 0,
        };
        
        assert_eq!(format_number(1234567, &options), "1,234,567");
        assert_eq!(format_number(1234567890, &options), "1,234,567,890");
    }

    #[test]
    fn test_format_number_human() {
        let options = NumberFormatOptions {
            use_comma: false,
            use_human: true,
            locale: "en".to_string(),
            decimal_places: 1,
        };
        
        assert_eq!(format_number(999, &options), "999"); // No decimal for < 1000
        assert_eq!(format_number(1000, &options), "1.0k");
        assert_eq!(format_number(1500, &options), "1.5k");
        assert_eq!(format_number(1000000, &options), "1.0m");
        assert_eq!(format_number(2500000000, &options), "2.5b");
    }

    #[test]
    fn test_warn_once_functionality() {
        // Clear any existing warnings
        if let Some(warnings) = WARNINGS.get() {
            if let Ok(mut warns) = warnings.lock() {
                warns.clear();
            }
        }
        if let Some(warned) = WARNED_MESSAGES.get() {
            if let Ok(mut warn_set) = warned.lock() {
                warn_set.clear();
            }
        }
        
        // First call should add warning
        warn_once("Test warning message");
        let warnings = get_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Test warning message"));
        
        // Second call should not add duplicate
        warn_once("Test warning message");
        let warnings = get_warnings();
        assert_eq!(warnings.len(), 1); // Still only one warning
        
        // Different message should be added
        warn_once("Different warning message");
        let warnings = get_warnings();
        assert_eq!(warnings.len(), 2);
    }

    // Helper functions for testing
    fn create_test_message(date_str: &str) -> ConversationMessage {
        create_test_message_with_role(date_str, MessageRole::User)
    }

    fn create_test_message_with_role(date_str: &str, role: MessageRole) -> ConversationMessage {
        create_test_message_with_stats_and_role(date_str, 1000, 500, 1.0, role)
    }

    fn create_test_message_with_model(date_str: &str, model: &str) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date: parse_date(date_str),
            project_hash: "test_project".to_string(),
            conversation_hash: "test_conversation".to_string(),
            local_hash: Some("test_local_hash".to_string()),
            global_hash: "test_global_hash".to_string(),
            model: Some(model.to_string()),
            stats: Stats {
                input_tokens: 1000,
                output_tokens: 500,
                cost: 1.0,
                tool_calls: 1,
                ..Default::default()
            },
            role: MessageRole::User,
            content: Some("Test message".to_string()),
        }
    }

    fn create_test_message_with_stats_and_role(
        date_str: &str, 
        input_tokens: u64, 
        output_tokens: u64, 
        cost: f64,
        role: MessageRole
    ) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date: parse_date(date_str),
            project_hash: "test_project".to_string(),
            conversation_hash: "test_conversation".to_string(),
            local_hash: Some("test_local_hash".to_string()),
            global_hash: "test_global_hash".to_string(),
            // Real user messages carry no model; aggregate_by_date uses `model` to split
            // user vs AI messages, so keep the helper consistent with real data.
            model: if role == MessageRole::Assistant {
                Some("claude-3-sonnet".to_string())
            } else {
                None
            },
            stats: Stats {
                input_tokens,
                output_tokens,
                cost,
                tool_calls: 1,
                ..Default::default()
            },
            role,
            content: Some("Test message".to_string()),
        }
    }

    /// Noon *local* time on the given day, expressed in UTC. aggregate_by_date buckets by
    /// local date, so midnight UTC would land on the previous day in any western timezone.
    fn parse_date(date_str: &str) -> DateTime<Utc> {
        use chrono::{Local, TimeZone};
        let naive = date_str
            .parse::<chrono::NaiveDate>()
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        Local
            .from_local_datetime(&naive)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn today_key() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }
}
use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use rayon::prelude::*;
use regex::Regex;
use serde::Deserialize;
use simd_json;
use simd_json::prelude::*;
use std::collections::HashSet;
use std::path::Path;

pub struct WarpDevAnalyzer;

impl WarpDevAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct WarpLogEntry {
    // For batch telemetry events
    batch: Option<Vec<WarpBatchItem>>,
    // For direct command execution logs
    command: Option<String>,
    output: Option<String>,
    exit_code: Option<i32>,
    // For AI context messages
    context_messages: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WarpBatchItem {
    #[serde(rename = "type")]
    event_type: String,
    properties: Option<WarpProperties>,
    original_timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WarpProperties {
    actual_next_command_run: Option<String>,
    #[serde(rename = "history_context")]
    history_context: Option<String>,
    #[serde(rename = "generate_ai_input_suggestions_request")]
    ai_suggestions_request: Option<WarpAiSuggestionsRequest>,
}

#[derive(Debug, Deserialize)]
struct WarpAiSuggestionsRequest {
    #[serde(rename = "context_messages")]
    context_messages: Option<Vec<String>>,
}

// GraphQL API response structures
#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    operation: String,
    timestamp: String,
    response: GraphQLResponseData,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponseData {
    data: Option<GraphQLData>,
}

#[derive(Debug, Deserialize)]
struct GraphQLData {
    user: Option<GraphQLUser>,
}

#[derive(Debug, Deserialize)]
struct GraphQLUser {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    user: Option<GraphQLUserData>,
}

#[derive(Debug, Deserialize)]
struct GraphQLUserData {
    #[serde(rename = "conversationUsage")]
    conversation_usage: Option<Vec<WarpConversation>>,
}

#[derive(Debug, Deserialize)]
struct WarpConversation {
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(rename = "lastUpdated")]
    last_updated: String,
    title: String,
    #[serde(rename = "usageMetadata")]
    usage_metadata: UsageMetadata,
}

#[derive(Debug, Deserialize)]
struct UsageMetadata {
    #[serde(rename = "contextWindowUsage")]
    context_window_usage: f64,
    #[serde(rename = "creditsSpent")]
    credits_spent: f64,
    summarized: bool,
    #[serde(rename = "tokenUsage")]
    token_usage: Vec<TokenUsage>,
    #[serde(rename = "toolUsageMetadata")]
    tool_usage_metadata: Option<ToolUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct TokenUsage {
    #[serde(rename = "modelId")]
    model_id: String,
    #[serde(rename = "totalTokens")]
    total_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ToolUsageMetadata {
    #[serde(rename = "runCommandsExecuted")]
    run_commands_executed: Option<u32>,
    #[serde(rename = "readFilesStats")]
    read_files_stats: Option<ToolStats>,
    #[serde(rename = "grepStats")]
    grep_stats: Option<ToolStats>,
    #[serde(rename = "applyFileDiffStats")]
    apply_file_diff_stats: Option<FileDiffStats>,
}

#[derive(Debug, Deserialize)]
struct ToolStats {
    count: u32,
}

#[derive(Debug, Deserialize)]
struct FileDiffStats {
    count: u32,
    #[serde(rename = "linesAdded")]
    lines_added: Option<u32>,
    #[serde(rename = "linesRemoved")]
    lines_removed: Option<u32>,
    #[serde(rename = "filesChanged")]
    files_changed: Option<u32>,
}

fn parse_warp_log_file(file_path: &Path) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(file_path)?;
    let mut interactions = Vec::new();

    // Find all "Body {" positions and extract complete JSON using brace counting
    let mut pos = 0;
    while let Some(body_start) = content[pos..].find("Body {") {
        let absolute_start = pos + body_start + 5; // Position after "Body "

        // Count braces to find the matching closing brace
        let mut brace_count = 0;
        let mut json_end = absolute_start;
        let chars: Vec<char> = content[absolute_start..].chars().collect();

        for (i, ch) in chars.iter().enumerate() {
            match ch {
                '{' => brace_count += 1,
                '}' => {
                    brace_count -= 1;
                    if brace_count == 0 {
                        json_end = absolute_start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }

        if brace_count == 0 && json_end > absolute_start {
            // Extract the complete JSON (including braces)
            let mut mutable_content = content[absolute_start..json_end].to_string();
            pos = json_end;

            match unsafe { simd_json::from_str::<WarpLogEntry>(&mut mutable_content) } {
                Ok(log_entry) => {
                    // Handle direct command execution logs
                    if let (Some(command), Some(_output)) = (&log_entry.command, &log_entry.output)
                    {
                        let timestamp_utc = chrono::Utc::now(); // TODO: extract from log if available
                        let pwd = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

                        // Create user message for command
                        interactions.push(ConversationMessage {
                            date: timestamp_utc,
                            application: Application::Warp,
                            project_hash: hash_text(&pwd),
                            conversation_hash: hash_text(&format!("{}{}", timestamp_utc, command)),
                            global_hash: hash_text(&format!(
                                "warp_cmd_{}_{}",
                                timestamp_utc.timestamp(),
                                &command[..std::cmp::min(command.len(), 20)]
                            )),
                            role: MessageRole::User,
                            content: Some(command.clone()),
                            model: None,
                            stats: Stats::default(),
                            local_hash: None,
                        });

                        // Create assistant message for command execution
                        interactions.push(ConversationMessage {
                            date: timestamp_utc,
                            application: Application::Warp,
                            project_hash: hash_text(&pwd),
                            conversation_hash: hash_text(&format!("{}{}", timestamp_utc, command)),
                            global_hash: hash_text(&format!(
                                "warp_out_{}_{}",
                                timestamp_utc.timestamp(),
                                &command[..std::cmp::min(command.len(), 20)]
                            )),
                            role: MessageRole::Assistant,
                            content: Some("Command executed".to_string()),
                            model: Some("Warp Terminal".to_string()),
                            stats: Stats {
                                terminal_commands: 1,
                                ..Stats::default()
                            },
                            local_hash: None,
                        });
                    }

                    // Handle batch telemetry events
                    if let Some(batch) = log_entry.batch {
                        for batch_item in batch {
                            if batch_item.event_type != "track" {
                                continue;
                            }

                            if let Some(properties) = batch_item.properties {
                                // Handle actual command runs
                                if let Some(command) = properties.actual_next_command_run {
                                    // Extract timestamp from original_timestamp or use current time as fallback
                                    let timestamp_str = batch_item
                                        .original_timestamp
                                        .clone()
                                        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

                                    if let Ok(timestamp) =
                                        DateTime::parse_from_rfc3339(&timestamp_str)
                                    {
                                        let timestamp_utc: DateTime<Utc> = timestamp.into();

                                        // For now, let's just use the home directory as pwd since it might not be in the properties
                                        let pwd = std::env::var("HOME")
                                            .unwrap_or_else(|_| ".".to_string());

                                        // Create a user message for the command
                                        interactions.push(ConversationMessage {
                                            date: timestamp_utc,
                                            application: Application::Warp,
                                            project_hash: hash_text(&pwd),
                                            conversation_hash: hash_text(&timestamp_str), // Use timestamp to group command/output
                                            global_hash: hash_text(&format!(
                                                "{}_{}_command",
                                                timestamp_str,
                                                &command[..std::cmp::min(command.len(), 20)]
                                            )),
                                            role: MessageRole::User,
                                            content: Some(command.clone()),
                                            model: None,
                                            stats: Stats::default(),
                                            local_hash: None,
                                        });

                                        // Create an assistant message for the command execution
                                        interactions.push(ConversationMessage {
                                            date: timestamp_utc,
                                            application: Application::Warp,
                                            project_hash: hash_text(&pwd),
                                            conversation_hash: hash_text(&timestamp_str),
                                            global_hash: hash_text(&format!(
                                                "{}_{}_output",
                                                timestamp_str,
                                                &command[..std::cmp::min(command.len(), 20)]
                                            )),
                                            role: MessageRole::Assistant,
                                            content: Some("Command executed".to_string()), // Placeholder for actual output
                                            model: Some("Warp Terminal".to_string()),
                                            stats: Stats {
                                                terminal_commands: 1,
                                                ..Stats::default()
                                            },
                                            local_hash: None,
                                        });
                                    }
                                }

                                // Handle history_context which contains multiple commands
                                if let Some(history_str) = properties.history_context {
                                    // History context contains multiple JSON objects as a string, separated by newlines
                                    for line in history_str.lines() {
                                        if !line.trim().is_empty() {
                                            let mut line_mutable = line.to_string();
                                            if let Ok(command_data) = unsafe {
                                                simd_json::from_str::<simd_json::OwnedValue>(
                                                    &mut line_mutable,
                                                )
                                            } {
                                                if let (Some(command), Some(start_ts)) = (
                                                    command_data
                                                        .get("command")
                                                        .and_then(|v| v.as_str()),
                                                    command_data
                                                        .get("start_ts")
                                                        .and_then(|v| v.as_str()),
                                                ) {
                                                    if let Ok(timestamp) =
                                                        DateTime::parse_from_rfc3339(start_ts)
                                                    {
                                                        let timestamp_utc: DateTime<Utc> =
                                                            timestamp.into();
                                                        let home_dir = std::env::var("HOME")
                                                            .unwrap_or_else(|_| ".".to_string());
                                                        let pwd = command_data
                                                            .get("pwd")
                                                            .and_then(|v| v.as_str())
                                                            .unwrap_or(home_dir.as_str());

                                                        // Create a user message for the command
                                                        interactions.push(ConversationMessage {
                                                            date: timestamp_utc,
                                                            application: Application::Warp,
                                                            project_hash: hash_text(pwd),
                                                            conversation_hash: hash_text(start_ts),
                                                            global_hash: hash_text(&format!(
                                                                "{}_{}_history",
                                                                start_ts,
                                                                &command[..std::cmp::min(
                                                                    command.len(),
                                                                    20
                                                                )]
                                                            )),
                                                            role: MessageRole::User,
                                                            content: Some(command.to_string()),
                                                            model: None,
                                                            stats: Stats::default(),
                                                            local_hash: None,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Handle AI context messages
                                if let Some(ai_request) = properties.ai_suggestions_request {
                                    if let Some(context_messages) = ai_request.context_messages {
                                        for context_str in context_messages {
                                            // These are JSON strings containing input/output pairs
                                            let mut context_mutable = context_str.clone();
                                            if let Ok(context_data) = unsafe {
                                                simd_json::from_str::<simd_json::OwnedValue>(
                                                    &mut context_mutable,
                                                )
                                            } {
                                                if let (Some(input), Some(output)) = (
                                                    context_data
                                                        .get("input")
                                                        .and_then(|v| v.as_str()),
                                                    context_data
                                                        .get("output")
                                                        .and_then(|v| v.as_str()),
                                                ) {
                                                    let timestamp_str = batch_item
                                                        .original_timestamp
                                                        .clone()
                                                        .unwrap_or_else(|| {
                                                            chrono::Utc::now().to_rfc3339()
                                                        });

                                                    if let Ok(timestamp) =
                                                        DateTime::parse_from_rfc3339(&timestamp_str)
                                                    {
                                                        let timestamp_utc: DateTime<Utc> =
                                                            timestamp.into();
                                                        let home_dir = std::env::var("HOME")
                                                            .unwrap_or_else(|_| ".".to_string());
                                                        let pwd = context_data
                                                            .get("context")
                                                            .and_then(|ctx| ctx.get("pwd"))
                                                            .and_then(|v| v.as_str())
                                                            .unwrap_or(home_dir.as_str());

                                                        // Create a user message for the input
                                                        interactions.push(ConversationMessage {
                                                            date: timestamp_utc,
                                                            application: Application::Warp,
                                                            project_hash: hash_text(pwd),
                                                            conversation_hash: hash_text(
                                                                &timestamp_str,
                                                            ),
                                                            global_hash: hash_text(&format!(
                                                                "{}_{}_ai_input",
                                                                timestamp_str,
                                                                &input[..std::cmp::min(
                                                                    input.len(),
                                                                    20
                                                                )]
                                                            )),
                                                            role: MessageRole::User,
                                                            content: Some(input.to_string()),
                                                            model: None,
                                                            stats: Stats::default(),
                                                            local_hash: None,
                                                        });

                                                        // Create an assistant message for the output
                                                        if !output.is_empty() {
                                                            interactions.push(
                                                                ConversationMessage {
                                                                    date: timestamp_utc,
                                                                    application: Application::Warp,
                                                                    project_hash: hash_text(pwd),
                                                                    conversation_hash: hash_text(
                                                                        &timestamp_str,
                                                                    ),
                                                                    global_hash: hash_text(
                                                                        &format!(
                                                                            "{}_{}_ai_output",
                                                                            timestamp_str,
                                                                            &input[..std::cmp::min(
                                                                                input.len(),
                                                                                20
                                                                            )]
                                                                        ),
                                                                    ),
                                                                    role: MessageRole::Assistant,
                                                                    content: Some(
                                                                        output.to_string(),
                                                                    ),
                                                                    model: Some(
                                                                        "Warp AI".to_string(),
                                                                    ),
                                                                    stats: Stats {
                                                                        tool_calls: 1,
                                                                        ..Stats::default()
                                                                    },
                                                                    local_hash: None,
                                                                },
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } // End of if let Some(batch)
                }
                Err(_e) => {
                    // Skip unparseable Body sections (e.g., GraphQL queries, other telemetry)
                }
            }
        } else {
            // Couldn't find matching closing brace, skip this entry
            pos = absolute_start + 1;
        }
    }

    Ok(interactions)
}

// Parse GraphQL conversation data from captured JSONL files
fn parse_graphql_conversations(file_path: &Path) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(file_path)?;
    let mut conversations = Vec::new();

    // Parse JSONL (one JSON object per line)
    for (line_num, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let mut mutable_line = line.to_string();
        match unsafe { simd_json::from_str::<GraphQLResponse>(&mut mutable_line) } {
            Ok(graphql_response) => {
                // Only process GetConversationUsage operations
                if graphql_response.operation != "GetConversationUsage" {
                    continue;
                }

                // Extract conversations
                if let Some(data) = graphql_response.response.data {
                    if let Some(user) = data.user {
                        if let Some(user_data) = user.user {
                            if let Some(conv_usage) = user_data.conversation_usage {
                                for conv in conv_usage {
                                    // Parse timestamp
                                    let timestamp = match DateTime::parse_from_rfc3339(&conv.last_updated) {
                                        Ok(dt) => dt.with_timezone(&Utc),
                                        Err(_) => {
                                            eprintln!("Failed to parse timestamp: {}", conv.last_updated);
                                            continue;
                                        }
                                    };

                                    // Calculate total tokens across all models
                                    let total_tokens: u64 = conv.usage_metadata.token_usage
                                        .iter()
                                        .map(|t| t.total_tokens)
                                        .sum();

                                    // Get primary model (first or most-used)
                                    let primary_model = conv.usage_metadata.token_usage
                                        .first()
                                        .map(|t| t.model_id.clone());

                                    // Extract tool usage stats
                                    let (bash_commands, file_reads, grep_searches) =
                                        if let Some(ref tool_meta) = conv.usage_metadata.tool_usage_metadata {
                                            (
                                                tool_meta.run_commands_executed.unwrap_or(0),
                                                tool_meta.read_files_stats.as_ref().map(|s| s.count).unwrap_or(0),
                                                tool_meta.grep_stats.as_ref().map(|s| s.count).unwrap_or(0),
                                            )
                                        } else {
                                            (0, 0, 0)
                                        };

                                    // Extract file diff stats
                                    let (lines_added, lines_removed, files_changed) =
                                        if let Some(ref tool_meta) = conv.usage_metadata.tool_usage_metadata {
                                            if let Some(ref diff_stats) = tool_meta.apply_file_diff_stats {
                                                (
                                                    diff_stats.lines_added.unwrap_or(0),
                                                    diff_stats.lines_removed.unwrap_or(0),
                                                    diff_stats.files_changed.unwrap_or(0),
                                                )
                                            } else {
                                                (0, 0, 0)
                                            }
                                        } else {
                                            (0, 0, 0)
                                        };

                                    // Convert credits to approximate USD (1 credit ≈ $0.22)
                                    let cost = conv.usage_metadata.credits_spent * 0.22;

                                    // Create user message (conversation start)
                                    conversations.push(ConversationMessage {
                                        date: timestamp,
                                        application: Application::Warp,
                                        project_hash: hash_text("warp_cloud"), // WARP doesn't expose project
                                        conversation_hash: hash_text(&conv.conversation_id),
                                        global_hash: hash_text(&format!("warp_graphql_{}", conv.conversation_id)),
                                        role: MessageRole::User,
                                        content: Some(conv.title.clone()),
                                        model: None,
                                        stats: Stats::default(),
                                        local_hash: None,
                                    });

                                    // Create assistant message with full stats
                                    // WARP doesn't split input/output, so estimate: 40% input, 60% output
                                    // (typical AI conversations have longer responses than prompts)
                                    let estimated_input = (total_tokens as f64 * 0.4) as u64;
                                    let estimated_output = (total_tokens as f64 * 0.6) as u64;

                                    conversations.push(ConversationMessage {
                                        date: timestamp,
                                        application: Application::Warp,
                                        project_hash: hash_text("warp_cloud"),
                                        conversation_hash: hash_text(&conv.conversation_id),
                                        global_hash: hash_text(&format!("warp_graphql_{}__resp", conv.conversation_id)),
                                        role: MessageRole::Assistant,
                                        content: Some(format!(
                                            "Conversation completed: {} (credits: {:.2}, context: {:.1}%)",
                                            conv.title,
                                            conv.usage_metadata.credits_spent,
                                            conv.usage_metadata.context_window_usage * 100.0
                                        )),
                                        model: primary_model,
                                        stats: Stats {
                                            // Estimate 40/60 split (WARP doesn't provide breakdown)
                                            input_tokens: estimated_input,
                                            output_tokens: estimated_output,
                                            cost,
                                            terminal_commands: bash_commands as u64,
                                            files_read: file_reads as u64,
                                            file_content_searches: grep_searches as u64,
                                            lines_added: lines_added as u64,
                                            lines_deleted: lines_removed as u64,
                                            files_edited: files_changed as u64,
                                            tool_calls: (bash_commands + file_reads + grep_searches) as u32,
                                            ..Stats::default()
                                        },
                                        local_hash: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to parse GraphQL line {}: {}", line_num + 1, e);
            }
        }
    }

    Ok(conversations)
}

#[async_trait]
impl Analyzer for WarpDevAnalyzer {
    fn display_name(&self) -> &'static str {
        "Warp"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        // Check multiple possible Warp data directory locations
        let possible_paths = [
            "Library/Application Support/Warp/warp_network.log", // Current Warp version
            "Library/Application Support/dev.warp.Warp-Stable/warp_network.log", // Older version
            "Library/Application Support/WarpTerminal/warp_network.log", // Alternative
        ];

        let mut existing_paths = Vec::new();
        for path_str in &possible_paths {
            if let Some(mut path) = dirs::home_dir() {
                path.push(path_str);
                if path.exists() {
                    existing_paths.push(path.to_string_lossy().into_owned());
                }
            }
        }

        existing_paths
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        // Add terminal telemetry log files
        for pattern in patterns {
            for entry in glob(&pattern)? {
                if let Ok(path) = entry {
                    if path.is_file() {
                        sources.push(DataSource { path });
                    }
                }
            }
        }

        // Add GraphQL captured conversation data
        if let Some(home) = dirs::home_dir() {
            let graphql_pattern = home
                .join("Projects/splitrail/schemas/warp/captured/*.jsonl")
                .to_string_lossy()
                .into_owned();

            for entry in glob(&graphql_pattern)? {
                if let Ok(path) = entry {
                    if path.is_file() {
                        sources.push(DataSource { path });
                    }
                }
            }
        }

        Ok(sources)
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let all_entries: Vec<ConversationMessage> = sources
            .into_par_iter()
            .filter_map(|source| {
                let path_str = source.path.to_string_lossy();

                // Determine file type and parse accordingly
                if path_str.contains("graphql_responses") || path_str.ends_with(".jsonl") {
                    // GraphQL conversation data
                    match parse_graphql_conversations(&source.path) {
                        Ok(messages) => Some(messages),
                        Err(e) => {
                            eprintln!(
                                "Failed to parse WARP GraphQL file {}: {e:#}",
                                source.path.display(),
                            );
                            None
                        }
                    }
                } else {
                    // Terminal telemetry log
                    match parse_warp_log_file(&source.path) {
                        Ok(messages) => Some(messages),
                        Err(e) => {
                            eprintln!(
                                "Failed to parse Warp log file {}: {e:#}",
                                source.path.display(),
                            );
                            None
                        }
                    }
                }
            })
            .flat_map(|messages| messages)
            .collect();

        let mut seen_hashes = HashSet::new();
        let deduplicated_entries: Vec<ConversationMessage> = all_entries
            .into_iter()
            .filter(|entry| {
                if seen_hashes.contains(&entry.global_hash) {
                    return false;
                }
                seen_hashes.insert(entry.global_hash.clone());
                true
            })
            .collect();

        Ok(deduplicated_entries)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);

        let num_conversations = daily_stats
            .values()
            .map(|stats| stats.conversations as u64)
            .sum();

        Ok(AgenticCodingToolStats {
            analyzer_name: self.display_name().to_string(),
            daily_stats,
            messages,
            num_conversations,
        })
    }

    fn is_available(&self) -> bool {
        self.discover_data_sources()
            .is_ok_and(|sources| !sources.is_empty())
    }
}

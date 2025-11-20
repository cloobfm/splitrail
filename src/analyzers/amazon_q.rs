use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simd_json::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct AmazonQAnalyzer;

impl AmazonQAnalyzer {
    pub fn new() -> Self {
        Self
    }

    fn get_database_path() -> Option<PathBuf> {
        let mut possible_paths = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            // macOS
            possible_paths.push(home_dir.join("Library").join("Application Support").join("amazon-q").join("data.sqlite3"));
            // Linux
            possible_paths.push(home_dir.join(".config").join("amazon-q").join("data.sqlite3"));
            possible_paths.push(home_dir.join(".local").join("share").join("amazon-q").join("data.sqlite3"));
        }

        // Windows
        if let Ok(appdata) = std::env::var("APPDATA") {
            possible_paths.push(PathBuf::from(appdata).join("amazon-q").join("data.sqlite3"));
        }

        for path in possible_paths {
            if path.exists() {
                return Some(path);
            }
        }

        None
    }
}

// Amazon Q conversation data structures
#[derive(Debug, Clone, Serialize, Deserialize)]
struct QConversation {
    conversation_id: String,
    #[serde(default)]
    next_message: Option<simd_json::OwnedValue>,
    #[serde(default)]
    history: Vec<QHistoryEntry>,
    #[serde(default)]
    valid_history_range: Option<Vec<u64>>,
    #[serde(default)]
    transcript: Option<Vec<String>>,
    #[serde(default)]
    tools: Option<simd_json::OwnedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QHistoryEntry {
    user: QUserMessage,
    assistant: QAssistantMessage,
    #[serde(default)]
    request_metadata: Option<QRequestMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QUserMessage {
    #[serde(default)]
    timestamp: Option<String>,
    content: QUserContent,
    #[serde(default)]
    env_context: Option<QEnvContext>,
    #[serde(default)]
    additional_context: Option<String>,
    #[serde(default)]
    images: Option<simd_json::OwnedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum QUserContent {
    Prompt {
        prompt: String,
    },
    ToolUseResults {
        tool_use_results: Vec<QToolUseResult>,
    },
    CancelledToolUses {
        prompt: String,
        tool_use_results: Vec<QToolUseResult>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QToolUseResult {
    tool_use_id: String,
    #[serde(default)]
    content: Vec<QToolResultContent>,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum QToolResultContent {
    Text(String),
    Json(simd_json::OwnedValue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QEnvContext {
    #[serde(default)]
    env_state: Option<QEnvState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QEnvState {
    #[serde(default)]
    current_working_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum QAssistantMessage {
    ToolUse {
        message_id: String,
        content: String,
        tool_uses: Vec<QToolUse>,
    },
    Response {
        message_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QToolUse {
    id: String,
    name: String,
    #[serde(default)]
    orig_name: Option<String>,
    #[serde(default)]
    args: simd_json::OwnedValue,
    #[serde(default)]
    orig_args: Option<simd_json::OwnedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QRequestMetadata {
    request_id: String,
    message_id: String,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    request_start_timestamp_ms: Option<i64>,
    #[serde(default)]
    stream_end_timestamp_ms: Option<i64>,
    #[serde(default)]
    user_prompt_length: Option<u64>,
    #[serde(default)]
    response_size: Option<u64>,
    #[serde(default)]
    time_to_first_chunk: Option<simd_json::OwnedValue>,
    #[serde(default)]
    time_between_chunks: Option<Vec<simd_json::OwnedValue>>,
    #[serde(default)]
    chat_conversation_type: Option<String>,
    #[serde(default)]
    tool_use_ids_and_names: Option<Vec<Vec<String>>>,
    #[serde(default)]
    message_meta_tags: Option<Vec<simd_json::OwnedValue>>,
}

// Parse a single Amazon Q conversation
// Public because Kiro CLI uses identical structure
pub fn parse_amazon_q_conversation(
    project_path: &str,
    conversation_json: &str,
) -> Result<Vec<ConversationMessage>> {
    // Parse using simd_json's serde API
    let mut owned_json = conversation_json.to_string();
    let conversation: QConversation = unsafe { simd_json::serde::from_str(&mut owned_json) }
        .context("Failed to parse Amazon Q conversation JSON structure")?;

    let project_hash = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let conversation_hash = hash_text(&conversation.conversation_id);

    let mut entries = Vec::new();

    for (idx, history_entry) in conversation.history.iter().enumerate() {
        // Parse timestamp from user message
        let timestamp = history_entry
            .user
            .timestamp
            .as_ref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                history_entry
                    .request_metadata
                    .as_ref()
                    .and_then(|m| m.request_start_timestamp_ms)
                    .and_then(|ms| DateTime::from_timestamp_millis(ms))
            })
            .unwrap_or_else(Utc::now);

        // Extract model from request metadata
        let model = history_entry
            .request_metadata
            .as_ref()
            .and_then(|m| m.model_id.clone());

        // Create user message
        let user_local_hash = format!("{}-user-{}", conversation_hash, idx);
        let user_global_hash = hash_text(&format!(
            "{}:{}:user:{}:{}",
            project_hash,
            conversation_hash,
            idx,
            timestamp.timestamp_millis()
        ));

        let user_content = format_q_user_message(&history_entry.user);
        entries.push(ConversationMessage {
            application: Application::AmazonQ,
            date: timestamp,
            project_hash: project_hash.clone(),
            conversation_hash: conversation_hash.clone(),
            local_hash: Some(user_local_hash),
            global_hash: user_global_hash,
            model: None,
            stats: Stats::default(),
            role: MessageRole::User,
            content: user_content,
        });

        // Create assistant message with stats
        let assistant_local_hash = format!("{}-assistant-{}", conversation_hash, idx);
        let assistant_global_hash = hash_text(&format!(
            "{}:{}:assistant:{}:{}",
            project_hash,
            conversation_hash,
            idx,
            timestamp.timestamp_millis()
        ));

        // Extract stats from request metadata
        let mut stats = Stats::default();

        if let Some(metadata) = &history_entry.request_metadata {
            // Use prompt length and response size as token estimates
            // Amazon Q doesn't expose exact token counts in the client-side data
            stats.input_tokens = metadata.user_prompt_length.unwrap_or(0);
            stats.output_tokens = metadata.response_size.unwrap_or(0);
        }

        // Calculate cost if we have a model and tokens
        if let Some(model_name) = &model {
            if stats.input_tokens > 0 || stats.output_tokens > 0 {
                stats.cost = calculate_total_cost(
                    model_name,
                    stats.input_tokens,
                    stats.output_tokens,
                    0, // Amazon Q doesn't have cache creation tokens in client data
                    0, // Amazon Q doesn't have cache read tokens in client data
                );
            }
        }

        // Count tool uses
        if let QAssistantMessage::ToolUse { tool_uses, .. } = &history_entry.assistant {
            stats.tool_calls = tool_uses.len() as u32;

            // Count file operations and shell commands based on tool names
            for tool_use in tool_uses {
                match tool_use.name.as_str() {
                    "read_file" | "read_many_files" => stats.files_read += 1,
                    "write_file" | "create_file" => stats.files_added += 1,
                    "edit_file" | "replace_string" => stats.files_edited += 1,
                    "delete_file" => stats.files_deleted += 1,
                    "execute_bash" | "run_shell_command" => stats.terminal_commands += 1,
                    "file_search" | "glob" => stats.file_searches += 1,
                    "grep" | "search_in_file" => stats.file_content_searches += 1,
                    "use_aws" => {
                        // AWS API calls - could track separately if needed
                    }
                    _ => {}
                }
            }
        }

        let assistant_content = format_q_assistant_message(&history_entry.assistant);
        entries.push(ConversationMessage {
            application: Application::AmazonQ,
            date: timestamp,
            project_hash: project_hash.clone(),
            conversation_hash: conversation_hash.clone(),
            local_hash: Some(assistant_local_hash),
            global_hash: assistant_global_hash,
            model,
            stats,
            role: MessageRole::Assistant,
            content: assistant_content,
        });
    }

    Ok(entries)
}

fn format_q_user_message(message: &QUserMessage) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(content) = format_q_user_content(&message.content) {
        if !content.is_empty() {
            parts.push(content);
        }
    }

    if let Some(context) = message
        .additional_context
        .as_deref()
        .filter(|ctx| !ctx.trim().is_empty())
    {
        parts.push(format!("Context:\n{context}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn format_q_user_content(content: &QUserContent) -> Option<String> {
    match content {
        QUserContent::Prompt { prompt } => {
            let trimmed = prompt.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        QUserContent::ToolUseResults { .. } | QUserContent::CancelledToolUses { .. } => None,
    }
}

fn format_q_assistant_message(message: &QAssistantMessage) -> Option<String> {
    match message {
        QAssistantMessage::Response { content, .. } => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        QAssistantMessage::ToolUse {
            content, tool_uses, ..
        } => {
            let mut parts = Vec::new();
            if !content.trim().is_empty() {
                parts.push(content.trim().to_string());
            }

            if !tool_uses.is_empty() {
                parts.push(format_tool_use_summary(tool_uses));
            }

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
        }
    }
}

fn format_tool_use_summary(tool_uses: &[QToolUse]) -> String {
    let mut lines = Vec::new();
    lines.push("Tool calls:".to_string());
    for tool in tool_uses {
        let mut entry = format!("- {}", tool.name);
        if let Some(orig) = &tool.orig_name {
            if orig != &tool.name {
                entry.push_str(&format!(" (source: {orig})"));
            }
        }
        if !tool.args.is_null() {
            entry.push_str(&format!(" args: {}", tool.args));
        }
        lines.push(entry);
    }
    lines.join("\n")
}

#[async_trait]
impl Analyzer for AmazonQAnalyzer {
    fn display_name(&self) -> &'static str {
        "Amazon Q"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        // Amazon Q uses SQLite, so we return the database path directly
        let mut patterns = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            // macOS
            patterns.push(home_dir.join("Library").join("Application Support").join("amazon-q").join("data.sqlite3").to_string_lossy().to_string());
            // Linux
            patterns.push(home_dir.join(".config").join("amazon-q").join("data.sqlite3").to_string_lossy().to_string());
            patterns.push(home_dir.join(".local").join("share").join("amazon-q").join("data.sqlite3").to_string_lossy().to_string());
        }

        // Windows
        if let Ok(appdata) = std::env::var("APPDATA") {
            patterns.push(PathBuf::from(appdata).join("amazon-q").join("data.sqlite3").to_string_lossy().to_string());
        }

        patterns
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        if let Some(db_path) = Self::get_database_path() {
            Ok(vec![DataSource { path: db_path }])
        } else {
            Ok(vec![])
        }
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        if sources.is_empty() {
            return Ok(vec![]);
        }

        // We expect only one source (the SQLite database)
        let db_path = &sources[0].path;

        // Open SQLite connection using rusqlite
        let conn =
            rusqlite::Connection::open(db_path).context("Failed to open Amazon Q database")?;

        // Query all conversations
        let mut stmt = conn
            .prepare("SELECT key, value FROM conversations")
            .context("Failed to prepare query")?;

        let conversations: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query conversations")?
            .filter_map(|r| r.ok())
            .collect();

        // Parse conversations in parallel
        // Note: Currently skipping failed parses due to ongoing debugging of JSON structure
        let all_entries: Vec<ConversationMessage> = conversations
            .into_par_iter()
            .flat_map(|(project_path, conversation_json)| {
                match parse_amazon_q_conversation(&project_path, &conversation_json) {
                    Ok(messages) => messages,
                    Err(_e) => {
                        // Silently skip - parsing issues being debugged
                        Vec::new()
                    }
                }
            })
            .collect();

        // Deduplicate by global hash
        let mut seen_hashes = HashSet::new();
        let deduplicated: Vec<ConversationMessage> = all_entries
            .into_iter()
            .filter(|msg| seen_hashes.insert(msg.global_hash.clone()))
            .collect();

        Ok(deduplicated)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let mut daily_stats = crate::utils::aggregate_by_date(&messages);

        // Remove any "unknown" entries
        daily_stats.retain(|date, _| date != "unknown");

        let num_conversations = daily_stats
            .values()
            .map(|stats| stats.conversations as u64)
            .sum();

        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        Self::get_database_path().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amazon_q_database_path() {
        // This test will only pass if the database exists
        if let Some(db_path) = AmazonQAnalyzer::get_database_path() {
            assert!(db_path.exists());
            assert_eq!(db_path.file_name().unwrap(), "data.sqlite3");
        }
    }
}

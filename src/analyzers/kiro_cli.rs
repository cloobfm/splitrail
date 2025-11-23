use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize, Deserializer};
use serde_json;
use simd_json::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// Helper function to deserialize null as default value
fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

// Kiro CLI specific data structures
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroConversation {
    conversation_id: String,
    #[serde(default)]
    next_message: Option<String>,
    history: Vec<KiroHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroHistoryEntry {
    user: KiroUserMessage,
    assistant: KiroAssistantMessage,
    #[serde(default)]
    request_metadata: Option<KiroRequestMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroUserMessage {
    #[serde(default)]
    timestamp: Option<String>,
    content: KiroUserContent,
    #[serde(default)]
    env_context: Option<KiroEnvContext>,
    #[serde(default)]
    additional_context: Option<String>,
    #[serde(default)]
    images: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum KiroUserContent {
    #[serde(rename = "Prompt")]
    Prompt {
        prompt: String,
    },
    #[serde(rename = "ToolUseResults")]
    ToolUseResults {
        tool_use_results: Vec<KiroToolUseResult>,
    },
    #[serde(rename = "CancelledToolUses")]
    CancelledToolUses {
        prompt: String,
        tool_use_results: Vec<KiroToolUseResult>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroToolUseResult {
    tool_use_id: String,
    #[serde(default)]
    content: Vec<KiroToolResultContent>,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum KiroToolResultContent {
    Text(String),
    Json(simd_json::OwnedValue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroEnvContext {
    #[serde(default)]
    env_state: Option<KiroEnvState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroEnvState {
    #[serde(default)]
    current_working_directory: Option<String>,
    #[serde(default)]
    operating_system: Option<String>,
    #[serde(default)]
    environment_variables: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroAssistantMessage {
    ToolUse: Option<KiroToolUseMessage>,
    Response: Option<KiroResponseMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroToolUseMessage {
    message_id: String,
    content: String,
    tool_uses: Vec<KiroToolUse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroResponseMessage {
    #[serde(default)]
    message_id: Option<String>,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroToolUse {
    id: String,
    name: String,
    #[serde(default)]
    orig_name: Option<String>,
    args: simd_json::OwnedValue,
    #[serde(default)]
    orig_args: Option<simd_json::OwnedValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroRequestMetadata {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    request_start_timestamp_ms: Option<i64>,
    #[serde(default)]
    stream_end_timestamp_ms: Option<i64>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    user_prompt_length: Option<u64>,
    #[serde(default)]
    response_size: Option<u64>,
    // Additional fields that Kiro CLI includes
    #[serde(default)]
    chat_conversation_type: Option<String>,
    #[serde(default)]
    message_meta_tags: Option<simd_json::OwnedValue>,
    #[serde(default)]
    time_between_chunks: Option<simd_json::OwnedValue>,
    #[serde(default)]
    time_to_first_chunk: Option<simd_json::OwnedValue>,
    #[serde(default)]
    tool_use_ids_and_names: Option<simd_json::OwnedValue>,
}

pub struct KiroCliAnalyzer;

impl KiroCliAnalyzer {
    pub fn new() -> Self {
        Self
    }

    // Parse a single Kiro CLI conversation
    fn parse_kiro_cli_conversation(
        project_path: &str,
        conversation_json: &str,
    ) -> Result<Vec<ConversationMessage>> {
    // Parse using regular serde for better error messages
    let conversation: KiroConversation = serde_json::from_str(conversation_json)
        .context("Failed to parse Kiro CLI conversation JSON structure")?;

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

            let user_content = Self::format_kiro_user_message(&history_entry.user);
            entries.push(ConversationMessage {
                application: Application::KiroCli,
                date: timestamp,
                project_hash: project_hash.clone(),
                conversation_hash: conversation_hash.clone(),
                local_hash: Some(user_local_hash),
                global_hash: user_global_hash,
                model: None,
                stats: Stats::default(),
                role: MessageRole::User,
                content: Some(user_content),
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
                // Kiro CLI doesn't expose exact token counts in the client-side data
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
                        0, // Kiro CLI doesn't have cache creation tokens in client data
                        0, // Kiro CLI doesn't have cache read tokens in client data
                    );
                }
            }

            let assistant_content = Self::format_kiro_assistant_message(&history_entry.assistant);
            entries.push(ConversationMessage {
                application: Application::KiroCli,
                date: timestamp,
                project_hash: project_hash.clone(),
                conversation_hash: conversation_hash.clone(),
                local_hash: Some(assistant_local_hash),
                global_hash: assistant_global_hash,
                model,
                stats,
                role: MessageRole::Assistant,
                content: Some(assistant_content),
            });
        }

        Ok(entries)
    }

    // Format Kiro CLI user message content
    fn format_kiro_user_message(user: &KiroUserMessage) -> String {
        match &user.content {
            KiroUserContent::Prompt { prompt } => {
                let mut content = prompt.clone();
                
                if let Some(env_ctx) = &user.env_context {
                    if let Some(env_state) = &env_ctx.env_state {
                        if let Some(cwd) = &env_state.current_working_directory {
                            content.push_str(&format!("\n\n[Working Directory: {}]", cwd));
                        }
                    }
                }
                
                if let Some(additional) = &user.additional_context {
                    if !additional.is_empty() {
                        content.push_str(&format!("\n\n[Additional Context: {}]", additional));
                    }
                }
                
                content
            }
            KiroUserContent::ToolUseResults { tool_use_results } => {
                let mut content = String::new();
                for result in tool_use_results {
                    content.push_str(&format!("Tool Result ({})\n", result.tool_use_id));
                    for item in &result.content {
                        match item {
                            KiroToolResultContent::Text(text) => {
                                content.push_str(text);
                            }
                            KiroToolResultContent::Json(json) => {
                                content.push_str(&format!("JSON: {}", json));
                            }
                        }
                        content.push('\n');
                    }
                }
                content
            }
            KiroUserContent::CancelledToolUses { prompt, tool_use_results } => {
                let mut content = format!("Cancelled Tool Uses\nPrompt: {}\n", prompt);
                for result in tool_use_results {
                    content.push_str(&format!("Cancelled Tool: {}\n", result.tool_use_id));
                }
                content
            }
        }
    }

    // Format Kiro CLI assistant message content
    fn format_kiro_assistant_message(assistant: &KiroAssistantMessage) -> String {
        if let Some(tool_use) = &assistant.ToolUse {
            let mut content = if !tool_use.content.is_empty() {
                tool_use.content.clone()
            } else {
                String::new()
            };

            for tool in &tool_use.tool_uses {
                content.push_str(&format!(
                    "\n\n[Tool Use: {}]\nArgs: {}",
                    tool.name,
                    tool.args
                ));
            }

            content
        } else if let Some(response) = &assistant.Response {
            response.content.clone()
        } else {
            "Empty assistant message".to_string()
        }
    }

    fn get_database_path() -> Option<PathBuf> {
        let mut possible_paths = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            // macOS
            possible_paths.push(
                home_dir
                    .join("Library")
                    .join("Application Support")
                    .join("kiro-cli")
                    .join("data.sqlite3"),
            );
            // Linux
            possible_paths.push(
                home_dir
                    .join(".config")
                    .join("kiro-cli")
                    .join("data.sqlite3"),
            );
            possible_paths.push(
                home_dir
                    .join(".local")
                    .join("share")
                    .join("kiro-cli")
                    .join("data.sqlite3"),
            );
        }

        // Windows
        if let Ok(appdata) = std::env::var("APPDATA") {
            possible_paths.push(PathBuf::from(appdata).join("kiro-cli").join("data.sqlite3"));
        }

        for path in possible_paths {
            if path.exists() {
                return Some(path);
            }
        }

        None
    }
}

#[async_trait]
impl Analyzer for KiroCliAnalyzer {
    fn display_name(&self) -> &'static str {
        "Kiro CLI"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        // Kiro CLI uses SQLite with identical structure to Amazon Q
        let mut patterns = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            // macOS
            patterns.push(
                home_dir
                    .join("Library")
                    .join("Application Support")
                    .join("kiro-cli")
                    .join("data.sqlite3")
                    .to_string_lossy()
                    .to_string(),
            );
            // Linux
            patterns.push(
                home_dir
                    .join(".config")
                    .join("kiro-cli")
                    .join("data.sqlite3")
                    .to_string_lossy()
                    .to_string(),
            );
            patterns.push(
                home_dir
                    .join(".local")
                    .join("share")
                    .join("kiro-cli")
                    .join("data.sqlite3")
                    .to_string_lossy()
                    .to_string(),
            );
        }

        // Windows
        if let Ok(appdata) = std::env::var("APPDATA") {
            patterns.push(
                PathBuf::from(appdata)
                    .join("kiro-cli")
                    .join("data.sqlite3")
                    .to_string_lossy()
                    .to_string(),
            );
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
            rusqlite::Connection::open(db_path).context("Failed to open Kiro CLI database")?;

        // Query all conversations - uses identical schema to Amazon Q
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

        // Parse conversations in parallel using the Kiro CLI parser
        let all_entries: Vec<ConversationMessage> = conversations
            .into_par_iter()
            .flat_map(|(project_path, conversation_json)| {
                match Self::parse_kiro_cli_conversation(&project_path, &conversation_json) {
                    Ok(messages) => messages,
                    Err(e) => {
                        // Log parsing errors for debugging with full details
                        eprintln!("Kiro CLI parsing error for project {}: {}", project_path, e);
                        eprintln!("Error chain: {:?}", e.chain().collect::<Vec<_>>());
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
    fn test_kiro_cli_database_path() {
        // This test will only pass if the database exists
        if let Some(db_path) = KiroCliAnalyzer::get_database_path() {
            assert!(db_path.exists());
            assert_eq!(db_path.file_name().unwrap(), "data.sqlite3");
        }
    }

    #[test]
    fn test_kiro_cli_uses_amazon_q_schema() {
        // Kiro CLI and Amazon Q use identical database schemas
        // This test verifies the database structure matches
        if let Some(db_path) = KiroCliAnalyzer::get_database_path() {
            let conn = rusqlite::Connection::open(db_path).expect("Failed to open database");

            // Check for expected tables
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();

            assert!(tables.contains(&"conversations".to_string()));
            assert!(tables.contains(&"history".to_string()));
            assert!(tables.contains(&"state".to_string()));
            assert!(tables.contains(&"auth_kv".to_string()));
            assert!(tables.contains(&"migrations".to_string()));
        }
    }
}

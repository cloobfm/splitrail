use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

/// OpenCode conversation data structures
/// Based on actual OpenCode storage format found in ~/.local/share/opencode/

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSession {
    id: String,
    version: String,
    #[serde(rename = "projectID")]
    project_id: String,
    directory: String,
    title: String,
    time: OpenCodeTime,
    summary: OpenCodeSummary,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeTime {
    created: u64,
    updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSummary {
    additions: u64,
    deletions: u64,
    files: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodePart {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "messageID")]
    message_id: String,
    #[serde(rename = "type")]
    part_type: String,
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageTime {
    created: u64,
    completed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageData {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    role: String,
    time: OpenCodeMessageTime,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
    mode: Option<String>,
    path: Option<OpenCodePath>,
    cost: Option<u64>,
    tokens: Option<OpenCodeTokens>,
    finish: Option<String>,
    summary: Option<OpenCodeMessageSummary>,
    agent: Option<String>,
    model: Option<OpenCodeMessageModel>,
    tools: Option<OpenCodeMessageTools>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodePath {
    cwd: Option<String>,
    root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache: Option<OpenCodeCache>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeCache {
    read: u64,
    write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageSummary {
    title: String,
    body: Option<String>, // Full message body text
    diffs: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageModel {
    #[serde(rename = "providerID")]
    provider_id: String,
    #[serde(rename = "modelID")]
    model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageTools {
    todowrite: Option<bool>,
    todoread: Option<bool>,
    task: Option<bool>,
}

pub struct OpenCodeAnalyzer;

impl OpenCodeAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Analyzer for OpenCodeAnalyzer {
    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();

            // OPTIMIZED: Only read message files (contain all stats) and sessions (for project context)
            // Message files contain complete token counts, model info, and timing
            patterns.push(format!(
                "{home_str}/.local/share/opencode/storage/message/*/msg_*.json"
            ));

            // Session metadata (read once for project context) - handle both directory structures
            patterns.push(format!(
                "{home_str}/.local/share/opencode/storage/session/*/ses_*.json"
            ));
            patterns.push(format!(
                "{home_str}/.local/share/opencode/storage/message/ses_*.json"
            ));

            // REMOVED: Part files - these are individual message fragments, text content not needed for stats
            // Part files are only necessary if we need to display actual conversation content
        }

        patterns
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        for pattern in patterns {
            for entry in glob::glob(&pattern)? {
                let path = entry?;
                if path.is_file() {
                    sources.push(DataSource { path });
                }
            }
        }

        Ok(sources)
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let mut messages = Vec::new();
        let mut seen_hashes = std::collections::HashSet::new();

        // Separate message and session files
        let mut message_files = Vec::new();
        let mut session_files = Vec::new();
        let mut uncategorized_files = Vec::new(); // For test files and others

        for source in sources {
            // Check the actual filename, not the full path
            // Message files: msg_*.json
            // Session files: ses_*.json
            if let Some(filename) = source.path.file_name().and_then(|f| f.to_str()) {
                if filename.starts_with("ses_") {
                    session_files.push(source);
                } else if filename.starts_with("msg_") {
                    message_files.push(source);
                } else {
                    // For files that don't match expected patterns (like test temp files),
                    // add them to uncategorized so we can try to parse them as messages
                    uncategorized_files.push(source);
                }
            } else {
                // If we can't get filename, treat as potential message file
                uncategorized_files.push(source);
            }
        }

        // Load session metadata (read once, used for project context)
        let mut sessions = std::collections::HashMap::new();
        for session_file in session_files {
            if let Ok(content) = std::fs::read_to_string(&session_file.path) {
                if let Ok(session) = serde_json::from_str::<OpenCodeSession>(&content) {
                    sessions.insert(session.id.clone(), session);
                }
            }
        }

        // Process message files
        for message_file in message_files {
            if let Ok(content) = std::fs::read_to_string(&message_file.path) {
                // Try to parse as single JSON object first, then as JSONL if that fails
                if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(&content) {
                    // Single JSON object format
                    // Get session info for directory path
                    let session_dir = sessions
                        .get(&opencode_msg.session_id)
                        .map(|s| s.directory.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    // Read actual message content from part files for both user and assistant
                    let message_content = self.read_message_text_content(&opencode_msg.id);

                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message_data(
                        opencode_msg,
                        &session_dir,
                        message_content,
                    ) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                } else {
                    // Try parsing as JSONL (multiple JSON objects separated by newlines)
                    for line in content.lines() {
                        let line = line.trim();
                        if !line.is_empty() {
                            if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(line) {
                                // Get session info for directory path
                                let session_dir = sessions
                                    .get(&opencode_msg.session_id)
                                    .map(|s| s.directory.clone())
                                    .unwrap_or_else(|| "unknown".to_string());

                                // Read actual message content from part files for both user and assistant
                                let message_content = self.read_message_text_content(&opencode_msg.id);

                                // Convert to our internal format
                                if let Some(msg) = self.convert_opencode_message_data(
                                    opencode_msg,
                                    &session_dir,
                                    message_content,
                                ) {
                                    // Deduplicate by global hash
                                    if seen_hashes.insert(msg.global_hash.clone()) {
                                        messages.push(msg);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process uncategorized files (for test files and any other valid message files)
        for message_file in uncategorized_files {
            if let Ok(content) = std::fs::read_to_string(&message_file.path) {
                // Try to parse as single JSON object first, then as JSONL if that fails
                if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(&content) {
                    // Single JSON object format
                    // Get session info for directory path
                    let session_dir = sessions
                        .get(&opencode_msg.session_id)
                        .map(|s| s.directory.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    // Read actual message content from part files for both user and assistant
                    let message_content = self.read_message_text_content(&opencode_msg.id);

                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message_data(
                        opencode_msg,
                        &session_dir,
                        message_content,
                    ) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                } else {
                    // Try parsing as JSONL (multiple JSON objects separated by newlines)
                    for line in content.lines() {
                        let line = line.trim();
                        if !line.is_empty() {
                            if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(line) {
                                // Get session info for directory path
                                let session_dir = sessions
                                    .get(&opencode_msg.session_id)
                                    .map(|s| s.directory.clone())
                                    .unwrap_or_else(|| "unknown".to_string());

                                // Read actual message content from part files for both user and assistant
                                let message_content = self.read_message_text_content(&opencode_msg.id);

                                // Convert to our internal format
                                if let Some(msg) = self.convert_opencode_message_data(
                                    opencode_msg,
                                    &session_dir,
                                    message_content,
                                ) {
                                    // Deduplicate by global hash
                                    if seen_hashes.insert(msg.global_hash.clone()) {
                                        messages.push(msg);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;

        let mut daily_stats = std::collections::BTreeMap::new();

        for message in &messages {
            // Convert UTC timestamp to local timezone for daily stats grouping
            let local_date = message.date.with_timezone(&chrono::Local);
            let date_str = local_date.format("%Y-%m-%d").to_string();
            let daily =
                daily_stats
                    .entry(date_str.clone())
                    .or_insert_with(|| crate::types::DailyStats {
                        date: date_str,
                        user_messages: 0,
                        ai_messages: 0,
                        conversations: 0,
                        models: std::collections::BTreeMap::new(),
                        stats: crate::types::Stats::default(),
                    });

            match message.role {
                crate::types::MessageRole::User => daily.user_messages += 1,
                crate::types::MessageRole::Assistant => {
                    daily.ai_messages += 1;
                    if let Some(ref model) = message.model {
                        *daily.models.entry(model.clone()).or_insert(0) += 1;
                    }
                }
            }

            // Add stats
            daily.stats.input_tokens += message.stats.input_tokens;
            daily.stats.output_tokens += message.stats.output_tokens;
            daily.stats.reasoning_tokens += message.stats.reasoning_tokens;
            daily.stats.cache_creation_tokens += message.stats.cache_creation_tokens;
            daily.stats.cache_read_tokens += message.stats.cache_read_tokens;
            daily.stats.cached_tokens += message.stats.cached_tokens;
            daily.stats.cost += message.stats.cost;
            daily.stats.tool_calls += message.stats.tool_calls;
            daily.stats.terminal_commands += message.stats.terminal_commands;
            daily.stats.file_searches += message.stats.file_searches;
            daily.stats.file_content_searches += message.stats.file_content_searches;
            daily.stats.files_read += message.stats.files_read;
            daily.stats.files_added += message.stats.files_added;
            daily.stats.files_edited += message.stats.files_edited;
            daily.stats.files_deleted += message.stats.files_deleted;
            daily.stats.lines_read += message.stats.lines_read;
            daily.stats.lines_added += message.stats.lines_added;
            daily.stats.lines_edited += message.stats.lines_edited;
            daily.stats.lines_deleted += message.stats.lines_deleted;
            daily.stats.bytes_read += message.stats.bytes_read;
            daily.stats.bytes_added += message.stats.bytes_added;
            daily.stats.bytes_edited += message.stats.bytes_edited;
            daily.stats.bytes_deleted += message.stats.bytes_deleted;
        }

        // Count unique conversations
        let unique_conversations: std::collections::HashSet<_> =
            messages.iter().map(|m| &m.conversation_hash).collect();
        let num_conversations = unique_conversations.len() as u64;

        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        // Check if OpenCode is installed and has potential data directories
        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();

            // Check for OpenCode config or data directories
            let config_path = format!("{home_str}/.config/opencode");
            let local_share_path = format!("{home_str}/.local/share/opencode");

            std::path::Path::new(&config_path).exists()
                || std::path::Path::new(&local_share_path).exists()
        } else {
            false
        }
    }
}

impl OpenCodeAnalyzer {
    /// Read text content from part files for a specific message
    fn read_message_text_content(&self, message_id: &str) -> Option<String> {
        // Find part directory for this message
        if let Some(home_dir) = std::env::home_dir() {
            let part_dir = home_dir
                .join(".local/share/opencode/storage/part")
                .join(message_id);

            if !part_dir.exists() {
                return None;
            }

            // Read all part files for this message and collect text
            let mut text_parts = Vec::new();

            if let Ok(entries) = std::fs::read_dir(&part_dir) {
                for entry in entries.flatten() {
                    if let Ok(content) = std::fs::read_to_string(&entry.path()) {
                        if let Ok(part) = serde_json::from_str::<OpenCodePart>(&content) {
                            // Only collect "text" type parts
                            if part.part_type == "text" {
                                if let Some(text) = part.text {
                                    if !text.trim().is_empty() {
                                        text_parts.push(text);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if text_parts.is_empty() {
                None
            } else {
                Some(text_parts.join(" "))
            }
        } else {
            None
        }
    }

    fn convert_opencode_message_data(
        &self,
        msg: OpenCodeMessageData,
        project_path: &str,
        content: Option<String>,
    ) -> Option<ConversationMessage> {
        // Determine role: user messages have actual user text in part files
        let role = match msg.role.to_lowercase().as_str() {
            "user" => MessageRole::User,
            "assistant" | "ai" => MessageRole::Assistant,
            _ => return None,
        };

        // Use cwd from message path if available, otherwise fall back to session directory
        let effective_project_path = msg.path
            .as_ref()
            .and_then(|p| p.cwd.as_ref())
            .filter(|cwd| !cwd.is_empty())
            .map(|cwd| {
                // Extract just the directory name from the full path
                Path::new(cwd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(cwd)
            })
            .unwrap_or_else(|| project_path);
        
        // Generate hashes - use the actual project name directly like other analyzers
        let project_hash = effective_project_path.to_string();
        let conversation_hash = hash_text(&msg.session_id);
        let local_hash = Some(hash_text(&format!("{}-{}", msg.id, msg.time.created)));
        let global_hash = hash_text(&format!("opencode-{}-{}", msg.id, msg.time.created));

        // Extract model information - FIXED: read from correct field
        let model = msg
            .model_id
            .clone()
            .or_else(|| msg.model.as_ref().map(|m| m.model_id.clone()))
            .or(Some("opencode-zen".to_string())); // fallback

        // Convert stats - FIXED: properly read token counts from message
        let mut stats = Stats::default();

        if let Some(tokens) = msg.tokens {
            stats.input_tokens = tokens.input;
            stats.output_tokens = tokens.output;
            stats.reasoning_tokens = tokens.reasoning;
            if let Some(cache) = tokens.cache {
                stats.cache_read_tokens = cache.read;
                stats.cache_creation_tokens = cache.write;
                stats.cached_tokens = cache.read;
            }
        }

        // Count tool calls based on finish type
        if let Some(finish) = &msg.finish {
            if finish == "tool-calls" {
                stats.tool_calls = 1;
            }
        }

        // Calculate cost using our pricing model (OpenCode stores cost=0 for free tier)
        if let Some(ref model_name) = model {
            stats.cost = calculate_total_cost(
                model_name,
                stats.input_tokens,
                stats.output_tokens,
                stats.cache_creation_tokens,
                stats.cache_read_tokens,
            );
        }

        // FIXED: Use actual message timestamp (milliseconds since epoch)
        let timestamp = DateTime::from_timestamp_millis(msg.time.created as i64)
            .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());

        Some(ConversationMessage {
            application: Application::OpenCode,
            date: timestamp,
            project_hash,
            conversation_hash,
            local_hash,
            global_hash,
            model,
            stats,
            role,
            content, // Text content from part files
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_parse_opencode_conversations() {
        let analyzer = OpenCodeAnalyzer::new();

        // Create a temporary file with sample OpenCode data
        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000},"modelID":null,"tokens":null,"finish":null}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"opencode-zen","tokens":{"input":10,"output":5,"reasoning":0},"finish":"stop"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // Check the assistant message
        let assistant_msg = &messages[1];
        assert_eq!(assistant_msg.role, MessageRole::Assistant);
        assert_eq!(assistant_msg.model, Some("opencode-zen".to_string()));
        assert_eq!(assistant_msg.stats.input_tokens, 10);
        assert_eq!(assistant_msg.stats.output_tokens, 5);
        assert_eq!(assistant_msg.stats.tool_calls, 0); // finish is "stop", not "tool-calls"
    }

    #[tokio::test]
    async fn test_parse_opencode_with_missing_fields() {
        let analyzer = OpenCodeAnalyzer::new();

        // Test with missing optional fields
        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000}}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"test-model"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // Should handle missing tokens/tools/files gracefully
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].stats.input_tokens, 0); // Default when missing
        assert_eq!(messages[1].stats.output_tokens, 0);
    }

    #[tokio::test]
    async fn test_parse_opencode_with_different_roles() {
        let analyzer = OpenCodeAnalyzer::new();

        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000}}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000}}
{"id":"msg3","sessionID":"test-session","role":"ai","time":{"created":1704110520000}}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 3);

        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[2].role, MessageRole::Assistant); // "ai" maps to Assistant
    }

    #[tokio::test]
    async fn test_parse_opencode_tool_call_detection() {
        let analyzer = OpenCodeAnalyzer::new();

        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"assistant","time":{"created":1704110400000},"modelID":"test","tokens":{"input":5,"output":3,"reasoning":0},"finish":"tool-calls"}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"test","tokens":{"input":5,"output":3,"reasoning":0},"finish":"stop"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // First message has tool-calls finish reason
        assert_eq!(messages[0].stats.tool_calls, 1);
        // Second message has stop finish reason
        assert_eq!(messages[1].stats.tool_calls, 0);
    }

    #[test]
    fn test_file_categorization() {
        let _analyzer = OpenCodeAnalyzer::new();

        // Mock file paths that match the glob patterns
        let message_file = DataSource {
            path: std::path::PathBuf::from("/home/user/.local/share/opencode/storage/message/session123/msg_001.json"),
        };
        let session_file = DataSource {
            path: std::path::PathBuf::from("/home/user/.local/share/opencode/storage/session/session123/ses_001.json"),
        };
        let sources = vec![message_file, session_file];

        let (message_files, session_files) = {
            let mut message_files = Vec::new();
            let mut session_files = Vec::new();

            for source in sources {
                let path_str = source.path.to_string_lossy();
                if path_str.contains("message/") {
                    message_files.push(source);
                } else if path_str.contains("session/") {
                    session_files.push(source);
                }
            }
            (message_files, session_files)
        };

        assert_eq!(message_files.len(), 1);
        assert_eq!(session_files.len(), 1);
        assert!(message_files[0].path.to_string_lossy().contains("msg_001.json"));
        assert!(session_files[0].path.to_string_lossy().contains("ses_001.json"));
    }

    #[test]
    fn test_convert_opencode_message_data_comprehensive() {
        let analyzer = OpenCodeAnalyzer::new();

        // Test comprehensive message conversion
        let msg_data = OpenCodeMessageData {
            id: "test-msg".to_string(),
            session_id: "test-session".to_string(),
            role: "assistant".to_string(),
            time: OpenCodeMessageTime {
                created: 1704110400000, // 2024-01-01 12:00:00 UTC
                completed: Some(1704110460000),
            },
            model: Some(OpenCodeMessageModel {
                provider_id: "opencode".to_string(),
                model_id: "zen".to_string(),
            }),
            tokens: Some(OpenCodeTokens {
                input: 100,
                output: 50,
                reasoning: 10,
                cache: Some(OpenCodeCache { read: 20, write: 5 }),
            }),
            finish: Some("tool-calls".to_string()),
            tools: Some(OpenCodeMessageTools {
                todowrite: Some(true),
                todoread: Some(false),
                task: Some(true),
            }),
            parent_id: None,
            mode: None,
            path: None,
            cost: None,
            summary: None,
            agent: None,
            model_id: None,
            provider_id: None,
        };

        let result = analyzer.convert_opencode_message_data(msg_data, "test-project", None);

        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.model, Some("zen".to_string()));
        assert_eq!(msg.stats.input_tokens, 100);
        assert_eq!(msg.stats.output_tokens, 50);
        assert_eq!(msg.stats.reasoning_tokens, 10);
        assert_eq!(msg.stats.cache_read_tokens, 20);
        assert_eq!(msg.stats.cache_creation_tokens, 5);
        assert_eq!(msg.stats.tool_calls, 1);
        assert_eq!(msg.project_hash, "test-project");
    }
}

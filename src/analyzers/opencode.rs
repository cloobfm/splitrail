use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
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
    snapshot: Option<String>,
    time: Option<OpenCodeTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageData {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    role: String,
    time: OpenCodeTime,
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

/// Combined message structure for analysis
#[derive(Debug, Clone)]
struct OpenCodeMessage {
    id: String,
    session_id: String,
    timestamp: DateTime<Utc>,
    role: String,
    content: String,
    parts: Vec<OpenCodePart>,
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
            
            // Actual OpenCode data storage locations
            // Complete messages (with model info) - PRIORITY
            patterns.push(format!("{home_str}/.local/share/opencode/storage/message/*/msg_*.json"));
            
            // Message parts (individual messages)
            patterns.push(format!("{home_str}/.local/share/opencode/storage/part/msg_*/prt_*.json"));
            
            // Session metadata
            patterns.push(format!("{home_str}/.local/share/opencode/storage/session/*/ses_*.json"));
            
            // Legacy/alternative locations
            patterns.push(format!("{home_str}/.config/opencode/sessions/**/*.jsonl"));
            patterns.push(format!("{home_str}/.local/share/opencode/sessions/**/*.jsonl"));
            patterns.push(format!("{home_str}/.opencode/sessions/**/*.jsonl"));
            
            // Project-specific OpenCode data
            patterns.push("**/.opencode/sessions/**/*.jsonl".to_string());
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
        
        // Separate message files, session files and part files
        let mut message_files = Vec::new();
        let mut session_files = Vec::new();
        let mut part_files = Vec::new();
        
        for source in sources {
            let path_str = source.path.to_string_lossy();
            if path_str.contains("message/") {
                message_files.push(source);
            } else if path_str.contains("session/") {
                session_files.push(source);
            } else if path_str.contains("part/") {
                part_files.push(source);
            }
        }
        
        // Load session metadata
        let mut sessions = std::collections::HashMap::new();
        for session_file in session_files {
            if let Ok(content) = std::fs::read_to_string(&session_file.path) {
                if let Ok(session) = serde_json::from_str::<OpenCodeSession>(&content) {
                    sessions.insert(session.id.clone(), session);
                }
            }
        }
        
        // Process complete message files first (these have model info)
        for message_file in message_files {
            if let Ok(content) = std::fs::read_to_string(&message_file.path) {
                if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(&content) {
                    // Get session info for directory path
                    let session_dir = sessions.get(&opencode_msg.session_id)
                        .map(|s| s.directory.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    
                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message_data(opencode_msg, &session_dir) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                }
            }
        }
        
        // Group parts by message
        let mut message_parts: std::collections::HashMap<String, Vec<OpenCodePart>> = std::collections::HashMap::new();
        for part_file in part_files {
            if let Ok(content) = std::fs::read_to_string(&part_file.path) {
                if let Ok(part) = serde_json::from_str::<OpenCodePart>(&content) {
                    message_parts.entry(part.message_id.clone()).or_default().push(part);
                }
            }
        }
        
        // Convert grouped parts into messages
        for (message_id, parts) in message_parts {
            if let Some(first_part) = parts.first() {
                if let Some(session) = sessions.get(&first_part.session_id) {
                    // Convert timestamp from milliseconds to DateTime<Utc>
                    let timestamp = DateTime::from_timestamp_millis(session.time.created as i64)
                        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());
                    
                    // Combine text from all parts
                    let content: String = parts.iter()
                        .filter_map(|p| p.text.as_ref())
                        .map(|s| s.as_str())
                        .collect::<Vec<&str>>()
                        .join("\n");
                    
                    // Determine role based on content and part types
                    let role = if parts.iter().any(|p| p.part_type == "text" && !content.trim().is_empty()) {
                        if content.starts_with("I'll") || content.starts_with("Let me") || content.contains("function") || content.contains("code") {
                            "assistant"
                        } else {
                            "user"
                        }
                    } else {
                        "assistant" // Default to assistant for non-text parts
                    };
                    
                    let opencode_msg = OpenCodeMessage {
                        id: message_id.clone(),
                        session_id: first_part.session_id.clone(),
                        timestamp,
                        role: role.to_string(),
                        content,
                        parts,
                    };
                    
                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message(opencode_msg, &session.directory) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
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
        let mut num_conversations = 0;
        
        for message in &messages {
            let date_str = message.date.format("%Y-%m-%d").to_string();
            let daily = daily_stats.entry(date_str.clone()).or_insert_with(|| crate::types::DailyStats {
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
        let unique_conversations: std::collections::HashSet<_> = messages
            .iter()
            .map(|m| &m.conversation_hash)
            .collect();
        num_conversations = unique_conversations.len() as u64;

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
            
            std::path::Path::new(&config_path).exists() || 
            std::path::Path::new(&local_share_path).exists()
        } else {
            false
        }
    }
}

impl OpenCodeAnalyzer {
    fn convert_opencode_message_data(
        &self,
        msg: OpenCodeMessageData,
        project_path: &str,
    ) -> Option<ConversationMessage> {
        // Convert role
        let role = match msg.role.to_lowercase().as_str() {
            "user" => MessageRole::User,
            "assistant" | "ai" => MessageRole::Assistant,
            _ => return None, // Skip unknown roles
        };

        // Generate hashes
        let project_hash = hash_text(project_path);
        let conversation_hash = hash_text(&msg.session_id);
        let local_hash = Some(hash_text(&format!("{}-{}", msg.id, msg.time.created)));
        let global_hash = hash_text(&format!("opencode-{}-{}", msg.id, msg.time.created));

        // Extract model information - ACTUAL MODEL DETECTION
        let model = msg.model_id.clone()
            .or_else(|| msg.model.as_ref().map(|m| m.model_id.clone()))
            .or_else(|| Some("opencode-zen".to_string())); // fallback

        // Convert stats - use actual token counts if available
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
        if let Some(finish) = msg.finish {
            if finish == "tool-calls" {
                stats.tool_calls = 1;
            }
        }
        
        // Calculate cost using actual model pricing
        stats.cost = msg.cost.unwrap_or(0) as f64 / 1000000.0; // Convert from micro-units if present

        Some(ConversationMessage {
            application: Application::OpenCode,
            date: DateTime::from_timestamp_millis(msg.time.created as i64)
                .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap()),
            project_hash,
            conversation_hash,
            local_hash,
            global_hash,
            model,
            stats,
            role,
            content: None, // Message files don't have content field
        })
    }

    fn convert_opencode_message(
        &self,
        msg: OpenCodeMessage,
        project_path: &str,
    ) -> Option<ConversationMessage> {
        // Convert role
        let role = match msg.role.to_lowercase().as_str() {
            "user" => MessageRole::User,
            "assistant" | "ai" => MessageRole::Assistant,
            _ => return None, // Skip unknown roles
        };

        // Generate hashes
        let project_hash = hash_text(project_path);
        let conversation_hash = hash_text(&msg.session_id);
        let local_hash = Some(hash_text(&format!("{}-{}", msg.id, msg.timestamp)));
        let global_hash = hash_text(&format!("opencode-{}-{}", msg.id, msg.timestamp));

        // Convert stats - OpenCode doesn't provide token counts, so we estimate
        let mut stats = Stats::default();
        
        // Estimate tokens from content length (rough approximation: 1 token ≈ 4 characters)
        let content_length = msg.content.len() as u64;
        if role == MessageRole::User {
            stats.input_tokens = content_length / 4;
        } else {
            stats.output_tokens = content_length / 4;
        }
        
        // Count reasoning parts
        let reasoning_parts = msg.parts.iter().filter(|p| p.part_type == "reasoning").count() as u64;
        stats.reasoning_tokens = reasoning_parts * 50; // Rough estimate
        
        // Count tool calls based on part types
        let tool_parts = msg.parts.iter().filter(|p| 
            p.part_type == "step-start" || 
            p.part_type == "tool-use" || 
            p.part_type == "tool-result"
        ).count() as u32;
        stats.tool_calls = tool_parts;
        
        // Estimate file operations from content analysis
        if msg.content.to_lowercase().contains("read") || msg.content.to_lowercase().contains("open") {
            stats.files_read += 1;
        }
        if msg.content.to_lowercase().contains("write") || msg.content.to_lowercase().contains("create") {
            stats.files_added += 1;
        }
        if msg.content.to_lowercase().contains("edit") || msg.content.to_lowercase().contains("modify") {
            stats.files_edited += 1;
        }
        if msg.content.to_lowercase().contains("delete") || msg.content.to_lowercase().contains("remove") {
            stats.files_deleted += 1;
        }
        
        // Estimate terminal commands
        if msg.content.to_lowercase().contains("cargo") || 
           msg.content.to_lowercase().contains("npm") || 
           msg.content.to_lowercase().contains("git") ||
           msg.content.to_lowercase().contains("ls") ||
           msg.content.to_lowercase().contains("cd") {
            stats.terminal_commands += 1;
        }
        
        // Estimate searches
        if msg.content.to_lowercase().contains("search") || 
           msg.content.to_lowercase().contains("find") ||
           msg.content.to_lowercase().contains("grep") {
            stats.file_searches += 1;
        }

        // Calculate cost using model pricing - OpenCode uses various models
        // Default to opencode-zen (free model) for now
        let model = Some("opencode-zen".to_string());
        stats.cost = 0.0; // OpenCode's zen model is free

        Some(ConversationMessage {
            application: Application::OpenCode,
            date: msg.timestamp,
            project_hash,
            conversation_hash,
            local_hash,
            global_hash,
            model,
            stats,
            role,
            content: Some(msg.content),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_parse_opencode_conversations() {
        let analyzer = OpenCodeAnalyzer::new();
        
        // Create a temporary file with sample OpenCode data
        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","timestamp":"2024-01-01T12:00:00Z","role":"user","content":"Hello","model":null,"tokens":null,"tools":null,"files":null}
{"id":"msg2","timestamp":"2024-01-01T12:01:00Z","role":"assistant","content":"Hi there!","model":"opencode-zen","tokens":{"input":10,"output":5},"tools":[{"name":"read","duration_ms":100,"success":true}],"files":[{"path":"test.rs","operation":"read","bytes":100,"lines":10}]}
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
        assert_eq!(assistant_msg.stats.files_read, 2); // 1 from tools + 1 from files
    }
}
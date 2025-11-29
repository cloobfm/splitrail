use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use std::collections::BTreeMap;

/// Droid CLI session data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DroidSession {
    #[serde(rename = "type")]
    session_type: String,
    id: String,
    title: String,
    owner: String,
    version: i32,
    cwd: String,
}

/// Droid CLI session settings with token usage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DroidSessionSettings {
    #[serde(rename = "assistantActiveTimeMs")]
    assistant_active_time_ms: u64,
    model: String,
    #[serde(rename = "reasoningEffort")]
    reasoning_effort: String,
    #[serde(rename = "autonomyMode")]
    autonomy_mode: String,
    #[serde(rename = "tokenUsage")]
    token_usage: DroidTokenUsage,
}

/// Droid CLI token usage structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DroidTokenUsage {
    #[serde(rename = "inputTokens")]
    input_tokens: u32,
    #[serde(rename = "outputTokens")]
    output_tokens: u32,
    #[serde(rename = "cacheCreationTokens")]
    cache_creation_tokens: u32,
    #[serde(rename = "cacheReadTokens")]
    cache_read_tokens: u32,
    #[serde(rename = "thinkingTokens")]
    thinking_tokens: u32,
}

/// Droid CLI analyzer for Factory AI Droid CLI data
pub struct DroidCliAnalyzer;

impl DroidCliAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Analyzer for DroidCliAnalyzer {
    fn display_name(&self) -> &'static str {
        "Droid CLI"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();
            patterns.push(format!("{home_str}/.factory/sessions/*/*.jsonl"));
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

        for source in sources {
            let session_messages = self.parse_session_file(&source.path).await?;
            messages.extend(session_messages);
        }

        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let conversations = self.parse_conversations(sources).await?;
        
        let mut daily_stats: BTreeMap<String, crate::types::DailyStats> = BTreeMap::new();
        
        for conversation in &conversations {
            let date_str = conversation.date.format("%Y-%m-%d").to_string();
            let daily = daily_stats.entry(date_str).or_insert_with(|| crate::types::DailyStats {
                date: conversation.date.format("%Y-%m-%d").to_string(),
                user_messages: 0,
                ai_messages: 0,
                conversations: 0,
                models: BTreeMap::new(),
                stats: Stats::default(),
            });

            if conversation.role == MessageRole::Assistant {
                daily.ai_messages += 1;
            } else {
                daily.user_messages += 1;
            }

            if let Some(ref model) = conversation.model {
                *daily.models.entry(model.clone()).or_insert(0) += 1;
            }

            // Merge stats
            daily.stats.input_tokens += conversation.stats.input_tokens;
            daily.stats.output_tokens += conversation.stats.output_tokens;
            daily.stats.reasoning_tokens += conversation.stats.reasoning_tokens;
            daily.stats.cache_creation_tokens += conversation.stats.cache_creation_tokens;
            daily.stats.cache_read_tokens += conversation.stats.cache_read_tokens;
            daily.stats.cached_tokens += conversation.stats.cached_tokens;
            daily.stats.cost += conversation.stats.cost;
            daily.stats.tool_calls += conversation.stats.tool_calls;
        }

        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations: conversations.len() as u64,
            messages: conversations,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        std::env::home_dir()
            .map(|h| h.join(".factory").join("sessions").exists())
            .unwrap_or(false)
    }
}

impl DroidCliAnalyzer {
    async fn parse_session_file(&self, file_path: &Path) -> Result<Vec<ConversationMessage>> {
        let content = fs::read_to_string(file_path)?;
        let mut messages = Vec::new();

        // Parse JSONL file
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(session) = serde_json::from_str::<DroidSession>(line) {
                if session.session_type == "session_start" {
                    // Look for corresponding settings file
                    let settings_path = file_path.with_extension("settings.json");
                    if let Ok(settings_content) = fs::read_to_string(&settings_path) {
                        if let Ok(settings) = serde_json::from_str::<DroidSessionSettings>(&settings_content) {
                            if let Ok(message) = self.create_conversation_message(&session, &settings, file_path) {
                                messages.push(message);
                            }
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    fn create_conversation_message(
        &self,
        session: &DroidSession,
        settings: &DroidSessionSettings,
        file_path: &Path,
    ) -> Result<ConversationMessage> {
        let timestamp = self.extract_timestamp_from_path(file_path)?;
        
        // Calculate costs using Droid CLI's model pricing
        let model_name = self.normalize_model_name(&settings.model);
        let total_cost = calculate_total_cost(
            &model_name,
            settings.token_usage.input_tokens as u64,
            settings.token_usage.output_tokens as u64,
            settings.token_usage.cache_creation_tokens as u64,
            settings.token_usage.cache_read_tokens as u64,
        );

        // Extract project info from working directory
        let project_hash = self.extract_project_hash(&session.cwd);

        // Create global hash
        let global_hash = format!("droid-cli:{}:{}", session.id, timestamp.timestamp());

        Ok(ConversationMessage {
            application: Application::DroidCli,
            date: timestamp,
            project_hash,
            conversation_hash: session.id.clone(),
            local_hash: Some(session.id.clone()),
            global_hash,
            model: Some(model_name.clone()),
            stats: Stats {
                input_tokens: settings.token_usage.input_tokens as u64,
                output_tokens: settings.token_usage.output_tokens as u64,
                reasoning_tokens: settings.token_usage.thinking_tokens as u64,
                cache_creation_tokens: settings.token_usage.cache_creation_tokens as u64,
                cache_read_tokens: settings.token_usage.cache_read_tokens as u64,
                cached_tokens: settings.token_usage.cache_read_tokens as u64, // Droid uses cache_read as cached
                cost: total_cost,
                tool_calls: 0, // Droid CLI doesn't track tool calls in session data
                terminal_commands: 0,
                file_searches: 0,
                file_content_searches: 0,
                files_read: 0,
                files_added: 0,
                files_edited: 0,
                files_deleted: 0,
                lines_read: 0,
                lines_added: 0,
                lines_edited: 0,
                lines_deleted: 0,
                bytes_read: 0,
                bytes_added: 0,
                bytes_edited: 0,
                bytes_deleted: 0,
                todos_created: 0,
                todos_completed: 0,
                todos_in_progress: 0,
                todo_writes: 0,
                todo_reads: 0,
                code_lines: 0,
                docs_lines: 0,
                data_lines: 0,
                media_lines: 0,
                config_lines: 0,
                other_lines: 0,
                rate_limits: None, // Droid CLI doesn't provide rate limit info
            },
            role: MessageRole::Assistant, // Droid CLI sessions are always from assistant
            content: Some(format!("Droid CLI session: {}", session.title)),
        })
    }

    fn extract_timestamp_from_path(&self, file_path: &Path) -> Result<DateTime<Utc>> {
        // Try to get file modification time as session timestamp
        let metadata = fs::metadata(file_path)?;
        let modified = metadata.modified()?;
        Ok(DateTime::from(modified))
    }

    fn normalize_model_name(&self, model: &str) -> String {
        match model {
            "claude-opus-4-5-20251101" => "claude-opus-4-5-20251101".to_string(),
            "claude-sonnet-4-20250514" => "claude-sonnet-4-20250514".to_string(),
            "claude-sonnet-4-1-20250714" => "claude-sonnet-4-1-20250714".to_string(),
            "claude-haiku-4-20250514" => "claude-haiku-4-20250514".to_string(),
            "gpt-5" => "gpt-5".to_string(),
            "gpt-5-mini" => "gpt-5-mini".to_string(),
            "gpt-5-nano" => "gpt-5-nano".to_string(),
            _ => model.to_string(),
        }
    }

    fn extract_project_hash(&self, cwd: &str) -> String {
        // Create a simple hash from the working directory
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        cwd.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs::File;
    use std::io::Write;

    #[tokio::test]
    async fn test_parse_droid_session() {
        let temp_dir = TempDir::new().unwrap();
        let session_dir = temp_dir.path().join("test-session");
        fs::create_dir_all(&session_dir).unwrap();

        // Create a mock session file
        let session_file = session_dir.join("session.jsonl");
        let mut file = File::create(&session_file).unwrap();
        writeln!(
            file,
            r#"{{"type":"session_start","id":"test-id","title":"Test Session","owner":"user","version":2,"cwd":"/Users/user/test-project"}}"#
        ).unwrap();

        // Create a mock settings file
        let settings_file = session_dir.join("session.settings.json");
        let mut settings = File::create(&settings_file).unwrap();
        writeln!(
            settings,
            r#"{{
  "assistantActiveTimeMs": 60000,
  "model": "claude-opus-4-5-20251101",
  "reasoningEffort": "off",
  "autonomyMode": "normal",
  "tokenUsage": {{
    "inputTokens": 1000,
    "outputTokens": 500,
    "cacheCreationTokens": 2000,
    "cacheReadTokens": 1000,
    "thinkingTokens": 0
  }}
}}"#
        ).unwrap();

        let analyzer = DroidCliAnalyzer;
        let messages = analyzer.parse_session_file(&session_file).await.unwrap();

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.conversation_hash, "test-id");
        assert_eq!(message.application, Application::DroidCli);
        assert_eq!(message.model, Some("claude-opus-4-5-20251101".to_string()));
        assert_eq!(message.stats.input_tokens, 1000);
        assert_eq!(message.stats.output_tokens, 500);
    }
}
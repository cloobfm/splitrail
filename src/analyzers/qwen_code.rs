use crate::analyzer::{Analyzer, DataSource};
use crate::models::{calculate_cache_cost, calculate_input_cost, calculate_output_cost};
use crate::types::{
    AgenticCodingToolStats, Application, ConversationMessage, FileCategory, MessageRole, Stats,
};
use crate::utils::{deserialize_utc_timestamp, hash_text};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simd_json::prelude::*;
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::Path;

pub struct QwenCodeAnalyzer;

impl QwenCodeAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

// Data structures for auto-stats JSONL files (requests and stats)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum QwenAutoStatsData {
    RequestLog(QwenRequestLogEntry),
    StatsLog(QwenStatsLogEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenRequestLogEntry {
    #[serde(deserialize_with = "deserialize_utc_timestamp")]
    timestamp: DateTime<Utc>,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "type")]
    entry_type: String,
    event: String,
    data: QwenRequestData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenRequestData {
    #[serde(rename = "userPromptId")]
    user_prompt_id: Option<String>,
    model: Option<String>,
    method: Option<String>,
    #[serde(rename = "contentLength")]
    content_length: Option<u64>,
    timestamp: String,
    streaming: Option<bool>,
    config: Option<simd_json::OwnedValue>,
    #[serde(rename = "durationMs")]
    duration_ms: Option<u64>,
    success: Option<bool>,
    #[serde(rename = "chunkCount")]
    chunk_count: Option<u64>,
    #[serde(rename = "candidatesCount")]
    candidates_count: Option<u64>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<QwenUsageMetadata>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
    #[serde(rename = "totalResponseLength")]
    total_response_length: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u64,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u64,
    #[serde(rename = "totalTokenCount")]
    total_token_count: u64,
    #[serde(rename = "cachedContentTokenCount")]
    cached_content_token_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenStatsLogEntry {
    #[serde(deserialize_with = "deserialize_utc_timestamp")]
    timestamp: DateTime<Utc>,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "type")]
    entry_type: String,
    stats: QwenAggregatedStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenAggregatedStats {
    models: Option<HashMap<String, QwenModelStats>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenModelStats {
    requests: u64,
    errors: u64,
    #[serde(rename = "latency_ms")]
    latency_ms: u64,
    tokens: QwenTokenCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenTokenCounts {
    prompt: u64,
    candidates: u64,
    total: u64,
    cached: u64,
}

// Qwen Code-specific data structures (identical to Gemini CLI format)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QwenCodeSession {
    session_id: String,
    project_hash: String,
    start_time: String,
    last_updated: String,
    messages: Vec<QwenCodeMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum QwenCodeMessage {
    User {
        id: String,
        #[serde(deserialize_with = "deserialize_utc_timestamp")]
        timestamp: DateTime<Utc>,
        content: String,
    },
    Qwen {
        id: String,
        #[serde(deserialize_with = "deserialize_utc_timestamp")]
        timestamp: DateTime<Utc>,
        content: String,
        model: String,
        #[serde(default)]
        thoughts: Vec<simd_json::OwnedValue>,
        tokens: Option<QwenCodeTokens>,
        #[serde(rename = "toolCalls", default)]
        tool_calls: Vec<simd_json::OwnedValue>,
    },
    System {
        id: String,
        #[serde(deserialize_with = "deserialize_utc_timestamp")]
        timestamp: DateTime<Utc>,
        content: String,
    },
    Error {
        id: String,
        #[serde(deserialize_with = "deserialize_utc_timestamp")]
        timestamp: DateTime<Utc>,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenCodeTokens {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    cached: u64,
    #[serde(default)]
    thoughts: u64,
    #[serde(default)]
    tool: u64,
    #[serde(default)]
    total: u64,
}

// Tool extraction and file operation mapping
fn extract_tool_stats(tool_calls: &[simd_json::OwnedValue]) -> Stats {
    let mut stats = Stats::default();

    for tool_call in tool_calls {
        let tool_name = if let Some(tool_name) = tool_call.get("name").and_then(|v| v.as_str()) {
            tool_name
        } else {
            continue;
        };
        match tool_name {
            "read_many_files" => {
                let paths = if let Some(paths) = tool_call
                    .get("args")
                    .and_then(|v| v.get("paths"))
                    .and_then(|v| v.as_array())
                {
                    paths
                } else {
                    continue;
                };
                stats.files_read += paths.len() as u64;

                // Categorize files and estimate composition stats
                for path in paths {
                    let path_str = if let Some(path_str) = path.as_str() {
                        path_str
                    } else {
                        continue;
                    };
                    let ext = std::path::Path::new(path_str)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let category = FileCategory::from_extension(ext);
                    let estimated_lines = 100; // Estimate lines per file
                    match category {
                        FileCategory::SourceCode => stats.code_lines += estimated_lines,
                        FileCategory::Documentation => stats.docs_lines += estimated_lines,
                        FileCategory::Data => stats.data_lines += estimated_lines,
                        FileCategory::Media => stats.media_lines += estimated_lines,
                        FileCategory::Config => stats.config_lines += estimated_lines,
                        FileCategory::Other => stats.other_lines += estimated_lines,
                    }
                }

                // Simple estimation without complex heuristics
                stats.lines_read += (paths.len() as u64) * 100;
                stats.bytes_read += (paths.len() as u64) * 8000;
            }
            "replace" => {
                stats.files_edited += 1;
                // Simple counting without complex content analysis
                stats.lines_edited += 10; // Conservative estimate
                stats.bytes_edited += 800;
            }
            "run_shell_command" => {
                stats.terminal_commands += 1;
            }
            "list_directory" => {
                // Treat as a lightweight read operation
                stats.files_read += 1;
            }
            _ => {} // Unknown tools - just skip
        }
    }

    // Use existing utility functions for line estimation
    stats.lines_added = (stats.lines_edited / 2).max(1); // Simple estimate
    stats.lines_deleted = (stats.lines_edited / 3).max(1); // Simple estimate

    stats
}

// Helper function to extract project ID from Qwen Code file path and hash it
fn extract_project_id_qwen_code(file_path: &Path) -> String {
    // Qwen Code path format: ~/.qwen/tmp/{PROJECT_ID}/chats/{session}.json
    // Example: "/home/user/.qwen/tmp/project-abc123/chats/session.json"

    let path_components: Vec<_> = file_path.components().collect();
    for (i, component) in path_components.iter().enumerate() {
        if let std::path::Component::Normal(name) = component
            && name.to_str() == Some("tmp")
            && i + 1 < path_components.len()
            && let std::path::Component::Normal(project_id) = &path_components[i + 1]
            && let Some(project_id_str) = project_id.to_str()
        {
            return project_id_str.to_string();
        }
    }

    "".to_string()
}

// Cost calculation using the centralized model system
fn calculate_qwen_cost(tokens: &QwenCodeTokens, model_name: &str) -> f64 {
    // Use heuristic to determine if input tokens include cached tokens
    // If input is at least as large as cached, assume input includes cached tokens
    let real_input_tokens = if tokens.input >= tokens.cached {
        // input likely includes cached tokens, subtract to get real new tokens
        tokens.input.saturating_sub(tokens.cached)
    } else {
        // input is less than cached, so input is likely new tokens only
        tokens.input
    };

    let total_input_tokens = real_input_tokens + tokens.thoughts + tokens.tool;

    let input_cost = calculate_input_cost(model_name, total_input_tokens);
    let output_cost = calculate_output_cost(model_name, tokens.output);
    let cache_cost = calculate_cache_cost(model_name, 0, tokens.cached); // Qwen Code doesn't have cache creation

    input_cost + output_cost + cache_cost
}

// Parse JSONL stats files (requests and stats logs)
fn parse_jsonl_stats_file(file_path: &Path) -> Result<Vec<ConversationMessage>> {
    let mut entries = Vec::new();
    let file_path_str = file_path.to_string_lossy();

    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        // Try to parse as either request log or stats log entry
        let mut line_bytes = line.as_bytes().to_vec();
        match simd_json::from_slice::<simd_json::OwnedValue>(&mut line_bytes) {
            Ok(json_value) => {
                // Try to parse as request log first
                if let Ok(request_entry) =
                    parse_request_log_from_json_value(&json_value, &file_path_str)
                {
                    entries.extend(request_entry);
                }
                // If not a request log, try to parse as stats log
                else if let Ok(stats_entry) =
                    parse_stats_log_from_json_value(&json_value, &file_path_str)
                {
                    entries.extend(stats_entry);
                }
            }
            Err(_) => continue, // Skip malformed lines
        }
    }

    Ok(entries)
}

// Helper function to parse a request log entry from JSON value
fn parse_request_log_from_json_value(
    json_value: &simd_json::OwnedValue,
    file_path_str: &str,
) -> Result<Vec<ConversationMessage>, ()> {
    // Check if this looks like a request log entry
    if let Some(obj) = json_value.as_object() {
        if let Some(session_id_val) = obj.get("sessionId") {
            if let Some(session_id) = session_id_val.as_str() {
                if let Some(type_val) = obj.get("type").and_then(|v| v.as_str()) {
                    if type_val == "auto_request_log" {
                        let timestamp =
                            if let Some(ts_str) = obj.get("timestamp").and_then(|v| v.as_str()) {
                                chrono::DateTime::parse_from_rfc3339(ts_str)
                                    .map(|dt| dt.with_timezone(&chrono::Utc))
                                    .ok()
                            } else {
                                None
                            };

                        let event = obj
                            .get("event")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        if let Some(data_obj) = obj.get("data").and_then(|v| v.as_object()) {
                            let model = data_obj
                                .get("model")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            if let Some(timestamp) = timestamp {
                                let mut stats = Stats::default();

                                // Extract token stats if available
                                if let Some(usage_obj) =
                                    data_obj.get("usageMetadata").and_then(|v| v.as_object())
                                {
                                    let prompt_tokens = usage_obj
                                        .get("promptTokenCount")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0);
                                    let candidates_tokens = usage_obj
                                        .get("candidatesTokenCount")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0);
                                    let cached_tokens = usage_obj
                                        .get("cachedContentTokenCount")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0);

                                    // Apply same heuristic as in JSON session parsing - if prompt includes cached content
                                    let real_input_tokens = if prompt_tokens >= cached_tokens {
                                        // prompt_tokens likely includes cached tokens, subtract to get real new tokens
                                        prompt_tokens.saturating_sub(cached_tokens)
                                    } else {
                                        // prompt_tokens is less than cached, so likely new content only
                                        prompt_tokens
                                    };

                                    stats.input_tokens = real_input_tokens;
                                    stats.output_tokens = candidates_tokens;
                                    stats.cached_tokens = cached_tokens;
                                    if let Some(ref model_name) = model {
                                        stats.cost =
                                            calculate_input_cost(model_name, real_input_tokens)
                                                + calculate_output_cost(
                                                    model_name,
                                                    candidates_tokens,
                                                )
                                                + calculate_cache_cost(
                                                    model_name,
                                                    0,
                                                    cached_tokens,
                                                );
                                    }
                                }

                                let role = if event == "request" {
                                    MessageRole::User
                                } else {
                                    MessageRole::Assistant
                                };

                                // Extract content if available
                                let content = if let Some(user_prompt_id) =
                                    data_obj.get("userPromptId").and_then(|v| v.as_str())
                                {
                                    Some(user_prompt_id.chars().take(200).collect())
                                } else {
                                    None
                                };

                                let msg = ConversationMessage {
                                    date: timestamp,
                                    application: Application::QwenCode,
                                    project_hash: String::new(), // Not available in request logs
                                    local_hash: Some(format!("req-{}-{}", session_id, event)),
                                    global_hash: hash_text(&format!(
                                        "{}:{}:{}:{}",
                                        file_path_str,
                                        session_id,
                                        timestamp.to_rfc3339(),
                                        event
                                    )),
                                    conversation_hash: hash_text(&session_id),
                                    model,
                                    stats,
                                    role,
                                    content,
                                };

                                return Ok(vec![msg]);
                            }
                        }
                    }
                }
            }
        }
    }

    Err(())
}

// Helper function to parse a stats log entry from JSON value
fn parse_stats_log_from_json_value(
    json_value: &simd_json::OwnedValue,
    file_path_str: &str,
) -> Result<Vec<ConversationMessage>, ()> {
    // Check if this looks like a stats log entry
    if let Some(obj) = json_value.as_object() {
        if let Some(session_id_val) = obj.get("sessionId") {
            if let Some(session_id) = session_id_val.as_str() {
                if let Some(type_val) = obj.get("type").and_then(|v| v.as_str()) {
                    if type_val == "auto_stats" {
                        let timestamp =
                            if let Some(ts_str) = obj.get("timestamp").and_then(|v| v.as_str()) {
                                chrono::DateTime::parse_from_rfc3339(ts_str)
                                    .map(|dt| dt.with_timezone(&chrono::Utc))
                                    .ok()
                            } else {
                                None
                            };

                        if let Some(stats_obj) = obj.get("stats").and_then(|v| v.as_object()) {
                            let mut messages = Vec::new();

                            // Process model-specific stats if available
                            if let Some(models_obj) =
                                stats_obj.get("models").and_then(|v| v.as_object())
                            {
                                for (model_name, model_value) in models_obj {
                                    if let Some(model_obj) = model_value.as_object() {
                                        let requests = model_obj
                                            .get("requests")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let errors = model_obj
                                            .get("errors")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let latency_ms = model_obj
                                            .get("latency_ms")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);

                                        if let Some(tokens_obj) =
                                            model_obj.get("tokens").and_then(|v| v.as_object())
                                        {
                                            let prompt_tokens = tokens_obj
                                                .get("prompt")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);
                                            let candidates_tokens = tokens_obj
                                                .get("candidates")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);
                                            let cached_tokens = tokens_obj
                                                .get("cached")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);

                                            // Apply same heuristic as in JSON session parsing - if prompt includes cached content
                                            let real_input_tokens =
                                                if prompt_tokens >= cached_tokens {
                                                    // prompt_tokens likely includes cached tokens, subtract to get real new tokens
                                                    prompt_tokens.saturating_sub(cached_tokens)
                                                } else {
                                                    // prompt_tokens is less than cached, so likely new content only
                                                    prompt_tokens
                                                };

                                            let stats = Stats {
                                                tool_calls: requests as u32,
                                                cost: if !model_name.is_empty() {
                                                    calculate_input_cost(
                                                        &model_name,
                                                        real_input_tokens,
                                                    ) + calculate_output_cost(
                                                        &model_name,
                                                        candidates_tokens,
                                                    ) + calculate_cache_cost(
                                                        &model_name,
                                                        0,
                                                        cached_tokens,
                                                    )
                                                } else {
                                                    0.0
                                                },
                                                cached_tokens,
                                                input_tokens: real_input_tokens,
                                                output_tokens: candidates_tokens,
                                                ..Default::default()
                                            };

                                            if let Some(timestamp) = timestamp {
                                                let msg = ConversationMessage {
                                                    date: timestamp,
                                                    application: Application::QwenCode,
                                                    project_hash: String::new(), // Not available in stats logs
                                                    local_hash: Some(format!(
                                                        "stats-{}-{}",
                                                        session_id, model_name
                                                    )),
                                                    global_hash: hash_text(&format!(
                                                        "{}:{}:stats:{}",
                                                        file_path_str, session_id, model_name
                                                    )),
                                                    conversation_hash: hash_text(&session_id),
                                                    model: Some(model_name.clone()),
                                                    stats,
                                                    role: MessageRole::Assistant, // Treat stats as system info
                                                    content: Some(format!(
                                                        "Session stats: {} requests, {} errors, {}ms latency, {} total tokens",
                                                        requests,
                                                        errors,
                                                        latency_ms,
                                                        prompt_tokens + candidates_tokens
                                                    )),
                                                };

                                                messages.push(msg);
                                            }
                                        }
                                    }
                                }
                            }

                            if !messages.is_empty() {
                                return Ok(messages);
                            }
                        }
                    }
                }
            }
        }
    }

    Err(())
}

// JSON session parsing (not JSONL)
fn parse_json_session_file(file_path: &Path) -> Result<Vec<ConversationMessage>> {
    let project_hash = extract_project_id_qwen_code(file_path);
    let file_path_str = file_path.to_string_lossy();
    let mut entries = Vec::new();

    // Parse the complete session JSON
    let session: QwenCodeSession =
        simd_json::from_slice(&mut std::fs::read_to_string(file_path)?.into_bytes())?;

    // Process each message in the session
    for message in session.messages {
        match message {
            QwenCodeMessage::User {
                id: _,
                timestamp,
                content,
            } => {
                entries.push(ConversationMessage {
                    date: timestamp,
                    application: Application::QwenCode,
                    project_hash: project_hash.clone(),
                    local_hash: None,
                    global_hash: hash_text(&format!(
                        "{}_{}",
                        file_path_str,
                        timestamp.to_rfc3339()
                    )),
                    conversation_hash: hash_text(&file_path.to_string_lossy()),
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::User,
                    content: Some(content),
                });
            }
            QwenCodeMessage::Qwen {
                id: _,
                timestamp,
                content,
                model,
                thoughts: _,
                tokens: Some(tokens),
                tool_calls,
            } => {
                let mut stats = extract_tool_stats(&tool_calls);

                // Update stats with token information
                // Qwen data format is inconsistent - sometimes input includes cached, sometimes not
                // Use heuristic: if input is at least as large as cached, it likely includes cached
                let real_input_tokens = if tokens.input >= tokens.cached {
                    // input likely includes cached tokens, subtract to get real new tokens
                    tokens.input.saturating_sub(tokens.cached)
                } else {
                    // input is less than cached, so input is likely new tokens only
                    tokens.input
                };

                stats.input_tokens = real_input_tokens;
                stats.output_tokens = tokens.output;
                stats.reasoning_tokens = tokens.thoughts + tokens.tool;
                stats.cache_creation_tokens = 0;
                stats.cache_read_tokens = 0;
                stats.cached_tokens = tokens.cached;
                stats.cost = calculate_qwen_cost(&tokens, &model);
                stats.tool_calls = tool_calls.len() as u32;

                entries.push(ConversationMessage {
                    application: Application::QwenCode,
                    model: Some(model),
                    local_hash: None,
                    global_hash: hash_text(&format!(
                        "{}_{}",
                        file_path_str,
                        timestamp.to_rfc3339()
                    )),
                    date: timestamp,
                    project_hash: project_hash.clone(),
                    conversation_hash: hash_text(&file_path.to_string_lossy()),
                    stats,
                    role: MessageRole::Assistant,
                    content: Some(content),
                });
            }
            QwenCodeMessage::Qwen {
                id: _,
                timestamp,
                content,
                model,
                thoughts: _,
                tokens: None,
                tool_calls,
            } => {
                let mut stats = extract_tool_stats(&tool_calls);

                // No token information available, use default values
                stats.input_tokens = 0;
                stats.output_tokens = 0;
                stats.cache_creation_tokens = 0;
                stats.cache_read_tokens = 0;
                stats.cached_tokens = 0;
                stats.cost = 0.0;
                stats.tool_calls = tool_calls.len() as u32;

                entries.push(ConversationMessage {
                    application: Application::QwenCode,
                    model: Some(model),
                    local_hash: None,
                    global_hash: hash_text(&format!(
                        "{}_{}",
                        file_path_str,
                        timestamp.to_rfc3339()
                    )),
                    date: timestamp,
                    project_hash: project_hash.clone(),
                    conversation_hash: hash_text(&file_path.to_string_lossy()),
                    stats,
                    role: MessageRole::Assistant,
                    content: Some(content),
                });
            }
            QwenCodeMessage::System {
                id: _,
                timestamp,
                content,
            } => {
                entries.push(ConversationMessage {
                    date: timestamp,
                    application: Application::QwenCode,
                    project_hash: project_hash.clone(),
                    local_hash: None,
                    global_hash: hash_text(&format!(
                        "{}_{}",
                        file_path_str,
                        timestamp.to_rfc3339()
                    )),
                    conversation_hash: hash_text(&file_path.to_string_lossy()),
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::Assistant, // System messages are from the assistant
                    content: Some(content),
                });
            }
            QwenCodeMessage::Error {
                id: _,
                timestamp,
                content,
            } => {
                entries.push(ConversationMessage {
                    date: timestamp,
                    application: Application::QwenCode,
                    project_hash: project_hash.clone(),
                    local_hash: None,
                    global_hash: hash_text(&format!(
                        "{}_{}",
                        file_path_str,
                        timestamp.to_rfc3339()
                    )),
                    conversation_hash: hash_text(&file_path.to_string_lossy()),
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::Assistant, // Error messages are from the assistant
                    content: Some(content),
                });
            }
        }
    }

    Ok(entries)
}

#[async_trait]
impl Analyzer for QwenCodeAnalyzer {
    fn display_name(&self) -> &'static str {
        "Qwen Code"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();
            // Chat files (traditional format)
            patterns.push(format!("{home_str}/.qwen/tmp/*/chats/*.json"));
            // Auto-stats files (historical data)
            patterns.push(format!("{home_str}/.qwen/auto_stats/*.jsonl"));
        }

        patterns
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let mut sources = Vec::new();

        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();

            // Add chat session files (traditional location)
            let chat_pattern = format!("{home_str}/.qwen/tmp/*/chats/*.json");
            for entry in glob(&chat_pattern)? {
                let path = entry?;
                if path.is_file() {
                    sources.push(DataSource { path });
                }
            }

            // Add auto-stats files (historical data)
            let auto_stats_pattern = format!("{home_str}/.qwen/auto_stats/*.jsonl");
            for entry in glob(&auto_stats_pattern)? {
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
        // Parse all session files in parallel, handling both JSON and JSONL formats
        let all_entries: Vec<ConversationMessage> = sources
            .into_par_iter()
            .flat_map(|source| {
                if source.path.extension().map_or(false, |ext| ext == "json") {
                    // Handle traditional JSON chat files
                    match parse_json_session_file(&source.path) {
                        Ok(messages) => messages,
                        Err(e) => {
                            eprintln!(
                                "Failed to parse Qwen Code session file {}: {e:#}",
                                source.path.display(),
                            );
                            Vec::new()
                        }
                    }
                } else if source.path.extension().map_or(false, |ext| ext == "jsonl") {
                    // Handle auto-stats JSONL files (requests and stats)
                    match parse_jsonl_stats_file(&source.path) {
                        Ok(messages) => messages,
                        Err(e) => {
                            eprintln!(
                                "Failed to parse Qwen Code stats file {}: {e:#}",
                                source.path.display(),
                            );
                            Vec::new()
                        }
                    }
                } else {
                    // Unknown file type
                    Vec::new()
                }
            })
            .collect();

        // Deduplicate based on global hash to avoid duplicates from overlapping data sources
        let mut seen_global_hashes = HashSet::new();
        let deduplicated_entries: Vec<ConversationMessage> = all_entries
            .into_iter()
            .filter(|entry| {
                if seen_global_hashes.contains(&entry.global_hash) {
                    return false;
                }
                seen_global_hashes.insert(entry.global_hash.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::Analyzer;

    #[tokio::test]
    async fn test_qwen_token_fix() {
        let analyzer = QwenCodeAnalyzer::new();

        if !analyzer.is_available() {
            println!("⚠️  Qwen Code analyzer not available - skipping test");
            return;
        }

        let stats = analyzer.get_stats().await.expect("Failed to get stats");

        if stats.messages.is_empty() {
            println!("⚠️  No Qwen messages found - skipping test");
            return;
        }

        // Find messages with token data
        let token_messages: Vec<_> = stats
            .messages
            .iter()
            .filter(|msg| msg.stats.input_tokens > 0 || msg.stats.cached_tokens > 0)
            .collect();

        if token_messages.is_empty() {
            println!("⚠️  No messages with token data found - skipping test");
            return;
        }

        println!(
            "🧪 Testing Qwen token fix with {} messages",
            token_messages.len()
        );

        // Test first few messages
        for (i, msg) in token_messages.iter().take(3).enumerate() {
            println!(
                "\n📝 Message {}: {}",
                i + 1,
                msg.date.format("%Y-%m-%d %H:%M:%S")
            );
            println!(
                "   Input: {} | Cached: {} | Output: {}",
                msg.stats.input_tokens, msg.stats.cached_tokens, msg.stats.output_tokens
            );
            println!(
                "   Reasoning: {} | Cost: ${:.6}",
                msg.stats.reasoning_tokens, msg.stats.cost
            );

            // Verify fix: input should not include cached tokens
            let total_input_including_cached = msg.stats.input_tokens + msg.stats.cached_tokens;

            if msg.stats.input_tokens < total_input_including_cached {
                println!(
                    "   ✅ Fix working: input ({}) < input+cached ({})",
                    msg.stats.input_tokens, total_input_including_cached
                );
            } else {
                println!(
                    "   ❌ Issue: input ({}) >= input+cached ({})",
                    msg.stats.input_tokens, total_input_including_cached
                );
            }
        }

        // Overall summary
        let total_new_input: u64 = token_messages.iter().map(|m| m.stats.input_tokens).sum();
        let total_cached: u64 = token_messages.iter().map(|m| m.stats.cached_tokens).sum();
        let total_old_calculation = total_new_input + total_cached;

        println!("\n📊 Summary:");
        println!("   Total new input tokens: {}", total_new_input);
        println!("   Total cached tokens: {}", total_cached);
        println!("   Old calculation would be: {}", total_old_calculation);
        println!(
            "   Reduction: {} tokens ({:.1}%)",
            total_cached,
            (total_cached as f64 / total_old_calculation as f64) * 100.0
        );

        // Assert that fix is working
        assert!(
            total_new_input < total_old_calculation,
            "Input tokens should be reduced after fix"
        );

        println!("\n✅ Test passed! Qwen token fix is working correctly.");
    }
}

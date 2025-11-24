use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simd_json::prelude::*;
use std::collections::HashSet;
use std::path::Path;

pub struct KiloCodeAnalyzer;

impl KiloCodeAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

// Kilo Code-specific data structures based on the discovered format

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KiloCodeUiMessage {
    Say {
        ts: i64,
        say: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        images: Vec<String>,
        #[serde(default)]
        partial: bool,
    },
    Ask {
        ts: i64,
        ask: String,
        #[serde(default)]
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiloCodeApiRequest {
    #[serde(default, rename = "apiProtocol")]
    api_protocol: String,
    #[serde(default, rename = "tokensIn")]
    tokens_in: u64,
    #[serde(default, rename = "tokensOut")]
    tokens_out: u64,
    #[serde(default, rename = "cacheWrites")]
    cache_writes: u64,
    #[serde(default, rename = "cacheReads")]
    cache_reads: u64,
    #[serde(default)]
    cost: f64,
    #[serde(default)]
    usage_missing: bool,
}

// Helper function to extract project ID from Kilo Code file path and hash it
fn extract_and_hash_project_id_kilo_code(file_path: &Path) -> String {
    // Kilo Code path formats:
    // - VS Code extension: ~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/{UUID}/
    // - CLI: ~/.kilocode/cli/global/tasks/{UUID}/
    // We'll use the parent directory of tasks as the project identifier (global storage path)

    let path_components: Vec<_> = file_path.components().collect();
    for (i, component) in path_components.iter().enumerate() {
        if let std::path::Component::Normal(name) = component
            && name.to_str() == Some("tasks")
            && i > 0
            && let std::path::Component::Normal(project_id) = &path_components[i - 1]
            && let Some(project_id_str) = project_id.to_str()
        {
            return hash_text(project_id_str);
        }
    }

    // Fallback: hash the full file path
    hash_text(&file_path.to_string_lossy())
}

// Helper function to extract model from environment details text
fn extract_model_from_text(text: &str) -> Option<String> {
    // Look for <model>...</model> tags in the text
    if let Some(start) = text.find("<model>")
        && let Some(end) = text[start..].find("</model>")
    {
        let model = &text[start + 7..start + end];
        return Some(model.to_string());
    }
    None
}

// Helper function to clean up message text (similar to Claude Code's clean_message_text)
fn clean_message_text(text: &str) -> String {
    let mut result = text.to_string();

    // Remove "Caveat:" messages and everything after until the next paragraph
    if let Some(caveat_pos) = result.find("Caveat:") {
        if let Some(double_newline) = result[caveat_pos..].find("\n\n") {
            result.replace_range(caveat_pos..caveat_pos + double_newline + 2, "");
        } else {
            // If no double newline, remove everything from Caveat to the end
            result.truncate(caveat_pos);
        }
    }

    // Parse command tags like <command-name>/clear</command-name> to just /clear
    while let Some(start_tag_pos) = result.find("<command-name>") {
        if let Some(end_tag_pos) = result[start_tag_pos..].find("</command-name>") {
            let command_start = start_tag_pos + "<command-name>".len();
            let command_end = start_tag_pos + end_tag_pos;
            let command = result[command_start..command_end].to_string();
            result.replace_range(
                start_tag_pos..command_end + "</command-name>".len(),
                &command,
            );
        } else {
            break;
        }
    }

    // Remove other common tags
    result = result.replace("<command-message>", "");
    result = result.replace("</command-message>", "");
    result = result.replace("<command-args>", "");
    result = result.replace("</command-args>", "");
    result = result.replace("<local-command-stdout>", "");
    result = result.replace("</local-command-stdout>", "");

    // Clean up extra whitespace
    result.trim().to_string()
}

// Determine if a reasoning message will be followed by a non-empty assistant response
fn reasoning_followed_by_non_empty_text(
    messages: &[KiloCodeUiMessage],
    current_index: usize,
) -> bool {
    for message in messages.iter().skip(current_index + 1) {
        match message {
            KiloCodeUiMessage::Say { say, text, .. } => match say.as_str() {
                "text" | "completion_result" | "error" => {
                    // Only return true if the text is non-empty after cleaning
                    let cleaned = clean_message_text(text);
                    return !cleaned.is_empty();
                }
                "api_req_started" | "user_feedback" => return false,
                _ => continue,
            },
            KiloCodeUiMessage::Ask { .. } => continue,
        }
    }
    false
}

// Parse a single Kilo Code task directory
fn parse_kilo_code_task_directory(task_dir: &Path) -> Result<Vec<ConversationMessage>> {
    let project_hash = extract_and_hash_project_id_kilo_code(task_dir);

    // Get the conversation hash from the task directory name (UUID)
    let conversation_hash = task_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(hash_text)
        .unwrap_or_else(|| hash_text(&task_dir.to_string_lossy()));

    // Try to extract model from api_conversation_history.json
    let mut current_model: Option<String> = None;
    let api_history_path = task_dir.join("api_conversation_history.json");
    if api_history_path.exists()
        && let Ok(mut content) = std::fs::read_to_string(&api_history_path).map(|s| s.into_bytes())
        && let Ok(history) = simd_json::from_slice::<Vec<simd_json::OwnedValue>>(&mut content)
    {
        // Look for model in user messages with environment_details (iterate forward and keep last one)
        for entry in history.iter() {
            if let Some(role) = entry.get("role").and_then(|r| r.as_str())
                && role == "user"
                && let Some(content_arr) = entry.get("content").and_then(|c| c.as_array())
            {
                for content_item in content_arr {
                    if let Some(text) = content_item.get("text").and_then(|t| t.as_str())
                        && let Some(model) = extract_model_from_text(text)
                    {
                        current_model = Some(model);
                    }
                }
            }
        }
    }

    // Read ui_messages.json
    let ui_messages_path = task_dir.join("ui_messages.json");
    if !ui_messages_path.exists() {
        return Ok(Vec::new());
    }

    let mut ui_messages_content = std::fs::read_to_string(&ui_messages_path)?.into_bytes();
    let ui_messages: Vec<KiloCodeUiMessage> = simd_json::from_slice(&mut ui_messages_content)
        .context("Failed to parse ui_messages.json")?;

    let mut entries = Vec::new();
    let mut message_index = 0;

    // Process ui_messages to extract API requests with token/cost data
    let mut pending_api_stats: Option<Stats> = None;
    let mut last_role: Option<MessageRole> = None;

    for (idx, message) in ui_messages.iter().enumerate() {
        match message {
            KiloCodeUiMessage::Say { ts, say, text, .. } => match say.as_str() {
                "api_req_started" => {
                    let mut text_bytes = text.clone().into_bytes();
                    if let Ok(api_req) =
                        simd_json::from_slice::<KiloCodeApiRequest>(&mut text_bytes)
                    {
                        let cost = if let Some(ref model) = current_model {
                            calculate_total_cost(
                                model,
                                api_req.tokens_in,
                                api_req.tokens_out,
                                api_req.cache_writes,
                                api_req.cache_reads,
                            )
                        } else {
                            api_req.cost // fallback to log cost if no model
                        };

                        pending_api_stats = Some(Stats {
                            input_tokens: api_req.tokens_in,
                            output_tokens: api_req.tokens_out,
                            cache_creation_tokens: api_req.cache_writes,
                            cache_read_tokens: api_req.cache_reads,
                            cached_tokens: api_req.cache_writes + api_req.cache_reads,
                            cost,
                            tool_calls: if api_req.tokens_out > 0 { 1 } else { 0 },
                            ..Default::default()
                        });
                    } else {
                        // If the embedded JSON can't be parsed, treat it as a regular assistant message
                        let cleaned = clean_message_text(text);
                        if cleaned.is_empty() {
                            continue;
                        }
                        let date = DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                        let local_hash = format!("{}-{}", conversation_hash, message_index);
                        let global_hash = hash_text(&format!(
                            "{}:{}:{}:{}",
                            project_hash, conversation_hash, message_index, ts
                        ));
                        entries.push(ConversationMessage {
                            application: Application::KiloCode,
                            date,
                            project_hash: project_hash.clone(),
                            conversation_hash: conversation_hash.clone(),
                            local_hash: Some(local_hash),
                            global_hash,
                            model: current_model.clone(),
                            stats: Stats::default(),
                            role: MessageRole::Assistant,
                            content: Some(cleaned),
                        });
                        message_index += 1;
                        last_role = Some(MessageRole::Assistant);
                    }
                }
                "checkpoint_saved" | "condense_context" | "command_output" => continue,
                "reasoning" => {
                    // Skip reasoning if it will be followed by non-empty text
                    // (the text will contain the full response including reasoning)
                    if reasoning_followed_by_non_empty_text(&ui_messages, idx) {
                        continue;
                    }

                    // If reasoning is not followed by text (or followed by empty text),
                    // use the reasoning content as the message content
                    if let Some(stats) = pending_api_stats.take() {
                        let cleaned = clean_message_text(text);
                        let content = if !cleaned.is_empty() {
                            Some(cleaned)
                        } else {
                            None
                        };

                        let date = DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                        let local_hash = format!("{}-{}", conversation_hash, message_index);
                        let global_hash = hash_text(&format!(
                            "{}:{}:{}:{}",
                            project_hash, conversation_hash, message_index, ts
                        ));
                        entries.push(ConversationMessage {
                            application: Application::KiloCode,
                            date,
                            project_hash: project_hash.clone(),
                            conversation_hash: conversation_hash.clone(),
                            local_hash: Some(local_hash),
                            global_hash,
                            model: current_model.clone(),
                            stats,
                            role: MessageRole::Assistant,
                            content,
                        });
                        message_index += 1;
                        last_role = Some(MessageRole::Assistant);
                    }
                }
                "completion_result" | "error" => {
                    let cleaned = clean_message_text(text);
                    if cleaned.is_empty() {
                        continue;
                    }

                    let stats = pending_api_stats.take().unwrap_or_default();
                    let date = DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                    let local_hash = format!("{}-{}", conversation_hash, message_index);
                    let global_hash = hash_text(&format!(
                        "{}:{}:{}:{}",
                        project_hash, conversation_hash, message_index, ts
                    ));

                    entries.push(ConversationMessage {
                        application: Application::KiloCode,
                        date,
                        project_hash: project_hash.clone(),
                        conversation_hash: conversation_hash.clone(),
                        local_hash: Some(local_hash),
                        global_hash,
                        model: current_model.clone(),
                        stats,
                        role: MessageRole::Assistant,
                        content: Some(cleaned),
                    });

                    message_index += 1;
                    last_role = Some(MessageRole::Assistant);
                }
                "text" => {
                    let cleaned = clean_message_text(text);
                    if cleaned.is_empty() {
                        if let Some(stats) = pending_api_stats.take() {
                            let date =
                                DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                            let local_hash = format!("{}-{}", conversation_hash, message_index);
                            let global_hash = hash_text(&format!(
                                "{}:{}:{}:{}",
                                project_hash, conversation_hash, message_index, ts
                            ));
                            entries.push(ConversationMessage {
                                application: Application::KiloCode,
                                date,
                                project_hash: project_hash.clone(),
                                conversation_hash: conversation_hash.clone(),
                                local_hash: Some(local_hash),
                                global_hash,
                                model: current_model.clone(),
                                stats,
                                role: MessageRole::Assistant,
                                content: None,
                            });
                            message_index += 1;
                            last_role = Some(MessageRole::Assistant);
                        }
                        continue;
                    }

                    let next_is_api = ui_messages.get(idx + 1).map_or(false, |next| match next {
                        KiloCodeUiMessage::Say { say, .. } => say == "api_req_started",
                        KiloCodeUiMessage::Ask { ask, .. } => ask == "api_req_failed",
                    });

                    let role = if pending_api_stats.is_some() {
                        MessageRole::Assistant
                    } else if next_is_api {
                        MessageRole::User
                    } else if matches!(last_role, Some(MessageRole::Assistant)) {
                        MessageRole::Assistant
                    } else if last_role.is_none() {
                        MessageRole::User
                    } else {
                        MessageRole::Assistant
                    };

                    let stats = if role == MessageRole::Assistant {
                        pending_api_stats.take().unwrap_or_default()
                    } else {
                        Stats::default()
                    };

                    let date = DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                    let local_hash = format!("{}-{}", conversation_hash, message_index);
                    let global_hash = hash_text(&format!(
                        "{}:{}:{}:{}",
                        project_hash, conversation_hash, message_index, ts
                    ));

                    entries.push(ConversationMessage {
                        application: Application::KiloCode,
                        date,
                        project_hash: project_hash.clone(),
                        conversation_hash: conversation_hash.clone(),
                        local_hash: Some(local_hash),
                        global_hash,
                        model: if role == MessageRole::Assistant {
                            current_model.clone()
                        } else {
                            None
                        },
                        stats,
                        role: role.clone(),
                        content: Some(cleaned),
                    });

                    message_index += 1;
                    last_role = Some(role);
                }
                "user_feedback" => {
                    let cleaned = clean_message_text(text);
                    if cleaned.is_empty() {
                        continue;
                    }
                    let date = DateTime::from_timestamp_millis(*ts).unwrap_or_else(Utc::now);
                    let local_hash = format!("{}-{}", conversation_hash, message_index);
                    let global_hash = hash_text(&format!(
                        "{}:{}:{}:{}",
                        project_hash, conversation_hash, message_index, ts
                    ));

                    entries.push(ConversationMessage {
                        application: Application::KiloCode,
                        date,
                        project_hash: project_hash.clone(),
                        conversation_hash: conversation_hash.clone(),
                        local_hash: Some(local_hash),
                        global_hash,
                        model: None,
                        stats: Stats::default(),
                        role: MessageRole::User,
                        content: Some(cleaned),
                    });

                    message_index += 1;
                    last_role = Some(MessageRole::User);
                }
                _ => {}
            },
            KiloCodeUiMessage::Ask { .. } => continue,
        }
    }

    Ok(entries)
}

#[async_trait]
impl Analyzer for KiloCodeAnalyzer {
    fn display_name(&self) -> &'static str {
        "Kilo Code"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        // VSCode forks that might have Kilo Code installed: Code, Cursor, Windsurf, VSCodium, Positron and Code - Insiders
        let vscode_gui_forks = [
            "Code",
            "Cursor",
            "Windsurf",
            "VSCodium",
            "Positron",
            "Code - Insiders",
        ];
        let vscode_cli_forks = ["vscode-server-insiders", "vscode-server"];

        if let Some(home_dir) = std::env::home_dir() {
            let home_str = home_dir.to_string_lossy();

            // Kilo Code CLI path
            patterns.push(format!(
                "{home_str}/.kilocode/cli/global/tasks/*/ui_messages.json"
            ));

            // Linux paths for all VSCode GUI forks
            for fork in &vscode_gui_forks {
                patterns.push(format!("{home_str}/.config/{fork}/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json"));
            }
            // Linux paths for all VSCode CLI (server) forks
            for fork in &vscode_cli_forks {
                patterns.push(format!("{home_str}/.{fork}/data/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json"));
            }

            // macOS paths for all VSCode GUI forks
            for fork in &vscode_gui_forks {
                patterns.push(format!("{home_str}/Library/Application Support/{fork}/User/globalStorage/kilocode.kilo-code/tasks/*/ui_messages.json"));
            }
        }
        // Windows paths for all VSCode GUI forks
        if let Ok(appdata) = std::env::var("APPDATA") {
            for fork in &vscode_gui_forks {
                patterns.push(format!("{appdata}\\{fork}\\User\\globalStorage\\kilocode.kilo-code\\tasks\\*\\ui_messages.json"));
            }
        }

        patterns
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        for pattern in patterns {
            for path in glob(&pattern)
                .unwrap_or_else(|_| glob("").unwrap())
                .flatten()
            {
                if path.is_file() {
                    // Store the parent directory (task directory) as the source
                    if let Some(parent) = path.parent() {
                        sources.push(DataSource {
                            path: parent.to_path_buf(),
                        });
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
        // Parse all task directories in parallel
        let all_entries: Vec<ConversationMessage> = sources
            .into_par_iter()
            .flat_map(
                |source| match parse_kilo_code_task_directory(&source.path) {
                    Ok(messages) => messages,
                    Err(e) => {
                        eprintln!(
                            "Failed to parse Kilo Code task directory {:?}: {}",
                            source.path, e
                        );
                        Vec::new()
                    }
                },
            )
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
        self.discover_data_sources()
            .is_ok_and(|sources| !sources.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_project_hash() {
        let path = PathBuf::from(
            "/home/user/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/9f365349-84f2-4a9a-b470-f94910583293/ui_messages.json",
        );
        let parent = path.parent().unwrap();
        let hash = extract_and_hash_project_id_kilo_code(parent);
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA256 hex length
    }
}

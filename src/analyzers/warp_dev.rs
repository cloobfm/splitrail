use crate::analyzer::{Analyzer, DataSource};
use crate::types::{
    AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats,
};
use crate::utils::hash_text;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use rayon::prelude::*;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct WarpDevAnalyzer;

impl WarpDevAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct WarpBlock {
    command: Option<String>,
    output: Option<String>,
    start_ts: String,
    pwd: String,
    was_autosuggestion_from_ai: Option<bool>,
}

fn get_warp_log_path() -> Option<PathBuf> {
    // Use dirs::home_dir() to get the home directory
    dirs::home_dir().map(|mut path| {
        path.push("Library/Application Support/dev.warp.Warp-Stable/warp_network.log");
        path
    })
}

fn parse_warp_log_file(file_path: &Path) -> Result<Vec<ConversationMessage>> {
    let content = std::fs::read_to_string(file_path)?;
    let mut interactions = Vec::new();
    let re = Regex::new(r"Body \{\s*(\{[\s\S]*?\})\s*\}")?;

    for cap in re.captures_iter(&content) {
        let json_body = &cap[1];
        if let Ok(block) = serde_json::from_str::<WarpBlock>(json_body) {
            if let Some(command) = block.command {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(&block.start_ts) {
                    let timestamp_utc: DateTime<Utc> = timestamp.into();
                    let is_ai = block.was_autosuggestion_from_ai.unwrap_or(false);

                    // Create a user message for the command
                    interactions.push(ConversationMessage {
                        date: timestamp_utc,
                        application: Application::Warp,
                        project_hash: hash_text(&block.pwd),
                        conversation_hash: hash_text(&block.start_ts), // Use timestamp to group command/output
                        global_hash: hash_text(&format!("{}_{}", block.start_ts, "user")),
                        role: MessageRole::User,
                        content: Some(command),
                        model: None,
                        stats: Stats::default(),
                        local_hash: None,
                    });

                    // Create an assistant message for the output
                    interactions.push(ConversationMessage {
                        date: timestamp_utc,
                        application: Application::Warp,
                        project_hash: hash_text(&block.pwd),
                        conversation_hash: hash_text(&block.start_ts),
                        global_hash: hash_text(&format!("{}_{}", block.start_ts, "assistant")),
                        role: MessageRole::Assistant,
                        content: block.output,
                        model: if is_ai { Some("Warp AI".to_string()) } else { None },
                        stats: Stats {
                            was_ai_suggestion: Some(is_ai),
                            ..Default::default()
                        },
                        local_hash: None,
                    });
                }
            }
        }
    }

    Ok(interactions)
}

#[async_trait]
impl Analyzer for WarpDevAnalyzer {
    fn display_name(&self) -> &'static str {
        "Warp"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        if let Some(path) = get_warp_log_path() {
            if path.exists() {
                return vec![path.to_string_lossy().into_owned()];
            }
        }
        vec![]
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        for pattern in patterns {
            for entry in glob(&pattern)? {
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
            .filter_map(|source| match parse_warp_log_file(&source.path) {
                Ok(messages) => Some(messages),
                Err(e) => {
                    eprintln!(
                        "Failed to parse Warp log file {}: {e:#}",
                        source.path.display(),
                    );
                    None
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
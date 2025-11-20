use crate::analyzer::{Analyzer, DataSource};
use crate::analyzers::amazon_q::parse_amazon_q_conversation;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

pub struct KiroCliAnalyzer;

impl KiroCliAnalyzer {
    pub fn new() -> Self {
        Self
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

        // Parse conversations in parallel using the Amazon Q parser
        // (Kiro CLI uses identical conversation structure)
        let all_entries: Vec<ConversationMessage> = conversations
            .into_par_iter()
            .flat_map(|(project_path, conversation_json)| {
                match parse_amazon_q_conversation(&project_path, &conversation_json) {
                    Ok(mut messages) => {
                        // Override application type to Kiro CLI
                        for msg in &mut messages {
                            msg.application = Application::KiroCli;
                        }
                        messages
                    }
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

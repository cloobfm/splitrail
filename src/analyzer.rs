use anyhow::Result;
use async_trait::async_trait;
use glob::Pattern;
use std::path::PathBuf;

use crate::types::{AgenticCodingToolStats, ConversationMessage};

#[derive(Clone, Debug)]
pub struct AnalyzerWatchDir {
    pub analyzer_name: String,
    pub match_dir: PathBuf,
    pub watch_dir: PathBuf,
    pub pattern: Pattern,
}

/// Represents a data source for an analyzer
#[derive(Debug, Clone)]
pub struct DataSource {
    pub path: PathBuf,
}

/// Main trait that all analyzers must implement
#[async_trait]
pub trait Analyzer: Send + Sync {
    /// Get the display name for this analyzer
    fn display_name(&self) -> &'static str;

    /// Get glob patterns for discovering data sources
    fn get_data_glob_patterns(&self) -> Vec<String>;

    /// Discover data sources for this analyzer
    fn discover_data_sources(&self) -> Result<Vec<DataSource>>;

    /// Parse conversations from data sources into normalized messages
    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>>;

    /// Get complete statistics for this analyzer
    async fn get_stats(&self) -> Result<AgenticCodingToolStats>;

    /// Check if this analyzer is available on the current system
    fn is_available(&self) -> bool;
}

/// Extract a pair of directories from a glob pattern for file watching.
/// `match_dir` represents the most specific path prefix before any wildcard.
/// `watch_dir` is the closest existing ancestor that `notify` can watch.
/// This allows us to watch parent directories even before the exact directory exists.
fn extract_watch_and_match_dirs(pattern: &str) -> Option<(PathBuf, PathBuf)> {
    fn trimmed_path(segment: &str) -> Option<PathBuf> {
        let trimmed = segment.trim_end_matches(['/', '\\']);
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    }

    fn existing_dir_or_ancestor(mut path: PathBuf) -> Option<PathBuf> {
        if path.as_os_str().is_empty() {
            return None;
        }

        loop {
            if path.exists() && path.is_dir() {
                return Some(path);
            }
            if !path.pop() {
                break;
            }
        }

        None
    }

    if let Some(pos) = pattern.find("**") {
        if let Some(match_dir) = trimmed_path(&pattern[..pos]) {
            if let Some(watch_dir) = existing_dir_or_ancestor(match_dir.clone()) {
                return Some((match_dir, watch_dir));
            }
        }
    }

    if let Some(pos) = pattern.find('*') {
        if let Some(match_dir) = trimmed_path(&pattern[..pos]) {
            if let Some(watch_dir) = existing_dir_or_ancestor(match_dir.clone()) {
                return Some((match_dir, watch_dir));
            }
        }
    }

    let pattern_path = PathBuf::from(pattern);
    if let Some(parent) = pattern_path.parent() {
        let match_dir = parent.to_path_buf();
        if let Some(watch_dir) = existing_dir_or_ancestor(match_dir.clone()) {
            return Some((match_dir, watch_dir));
        }
    }

    None
}

/// Registry for managing multiple analyzers
#[derive(Default)]
pub struct AnalyzerRegistry {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalyzerRegistry {
    /// Create a new analyzer registry
    pub fn new() -> Self {
        Self {
            analyzers: Vec::new(),
        }
    }

    /// Register an analyzer
    pub fn register<A: Analyzer + 'static>(&mut self, analyzer: A) {
        self.analyzers.push(Box::new(analyzer));
    }

    /// Get available analyzers (those that are present on the system)
    pub fn available_analyzers(&self) -> Vec<&dyn Analyzer> {
        self.analyzers
            .iter()
            .filter(|a| a.is_available())
            .map(|a| a.as_ref())
            .collect()
    }

    /// Get analyzer by display name  
    pub fn get_analyzer_by_display_name(&self, display_name: &str) -> Option<&dyn Analyzer> {
        self.analyzers
            .iter()
            .find(|a| a.display_name() == display_name)
            .map(|a| a.as_ref())
    }

    /// Load stats from all available analyzers
    pub async fn load_all_stats(&self) -> Result<crate::types::MultiAnalyzerStats> {
        let available_analyzers = self.available_analyzers();
        let mut all_stats = Vec::new();

        for analyzer in available_analyzers {
            match analyzer.get_stats().await {
                Ok(stats) => all_stats.push(stats),
                Err(e) => {
                    eprintln!(
                        "⚠️  Error analyzing {} data: {}",
                        analyzer.display_name(),
                        e
                    );
                }
            }
        }

        Ok(crate::types::MultiAnalyzerStats {
            analyzer_stats: all_stats,
        })
    }

    /// Get a mapping of data directories to analyzer names for file watching.
    /// Each entry records both the directory we can watch today and the more specific
    /// match prefix associated with the analyzer's data files.
    pub fn get_directory_to_analyzer_mapping(&self) -> Vec<AnalyzerWatchDir> {
        let mut dir_to_analyzer = Vec::new();

        for analyzer in &self.analyzers {
            let analyzer = analyzer.as_ref();
            let analyzer_name = analyzer.display_name().to_string();

            // Watch directories implied by glob patterns
            for pattern in analyzer.get_data_glob_patterns() {
                if let Some((match_dir, watch_dir)) = extract_watch_and_match_dirs(&pattern)
                    && let Ok(glob_pattern) = Pattern::new(&pattern)
                {
                    dir_to_analyzer.push(AnalyzerWatchDir {
                        analyzer_name: analyzer_name.clone(),
                        match_dir,
                        watch_dir,
                        pattern: glob_pattern,
                    });
                }
            }

            // NOTE: We used to also watch individual file parent directories, but this caused
            // massive overhead and conflicts. The recursive watch from glob patterns above
            // is sufficient to catch all file changes.
        }

        dir_to_analyzer
    }
}

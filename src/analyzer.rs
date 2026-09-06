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

/// Cheap change detector for a set of data sources: a hash of every file's path, size, and
/// modification time. Two calls return the same value if and only if no file was added,
/// removed, appended to, or rewritten in between. Costs one `stat` per file and no reads, so
/// it is safe to call on a timer where a full re-parse would not be.
pub fn sources_fingerprint(sources: &[DataSource]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut entries: Vec<(&std::path::Path, u64, Option<std::time::SystemTime>)> = sources
        .iter()
        .map(|source| {
            let meta = std::fs::metadata(&source.path).ok();
            (
                source.path.as_path(),
                meta.as_ref().map_or(0, |m| m.len()),
                meta.and_then(|m| m.modified().ok()),
            )
        })
        .collect();
    // Order-independent: discovery order can vary between globs.
    entries.sort();

    let mut hasher = DefaultHasher::new();
    entries.len().hash(&mut hasher);
    for (path, len, modified) in entries {
        path.hash(&mut hasher);
        len.hash(&mut hasher);
        modified.hash(&mut hasher);
    }
    hasher.finish()
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

#[cfg(test)]
mod fingerprint_tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn sources(dir: &std::path::Path) -> Vec<DataSource> {
        let mut paths: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        paths.into_iter().map(|path| DataSource { path }).collect()
    }

    #[test]
    fn fingerprint_is_stable_when_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jsonl"), "one\n").unwrap();
        let first = sources_fingerprint(&sources(dir.path()));
        let second = sources_fingerprint(&sources(dir.path()));
        assert_eq!(first, second);
    }

    #[test]
    fn fingerprint_changes_when_a_file_is_appended() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.jsonl");
        fs::write(&file, "one\n").unwrap();
        let before = sources_fingerprint(&sources(dir.path()));
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "two").unwrap();
        let after = sources_fingerprint(&sources(dir.path()));
        assert_ne!(before, after, "size change must be detected");
    }

    #[test]
    fn fingerprint_changes_when_a_file_is_added_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jsonl"), "one\n").unwrap();
        let one = sources_fingerprint(&sources(dir.path()));
        fs::write(dir.path().join("b.jsonl"), "two\n").unwrap();
        let two = sources_fingerprint(&sources(dir.path()));
        assert_ne!(one, two);
        fs::remove_file(dir.path().join("b.jsonl")).unwrap();
        assert_eq!(sources_fingerprint(&sources(dir.path())), one);
    }

    #[test]
    fn fingerprint_is_order_independent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jsonl"), "one\n").unwrap();
        fs::write(dir.path().join("b.jsonl"), "two\n").unwrap();
        let mut forward = sources(dir.path());
        let fp_forward = sources_fingerprint(&forward);
        forward.reverse();
        assert_eq!(sources_fingerprint(&forward), fp_forward);
    }

    #[test]
    fn fingerprint_of_no_sources_is_stable() {
        assert_eq!(sources_fingerprint(&[]), sources_fingerprint(&[]));
    }
}

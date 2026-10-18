//! Whole-file parse cache for analyzers whose sources are replaced rather than appended to.
//!
//! A reload re-runs `get_stats()` for the analyzer whose directory saw a write, and most analyzers
//! re-parsed every one of their sources to serve it. Watching a single file means watching its
//! whole parent directory (see `analyzer::extract_watch_and_match_dirs`), so an unrelated write
//! nearby triggers that too. This keeps each file's parsed messages against its size and mtime, so
//! a reload only pays for files that actually changed (BZL-74).
//!
//! For append-only JSONL, prefer [`crate::incremental::IncrementalJsonlCache`], which parses only
//! the new bytes instead of the whole file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Result;
use rayon::prelude::*;

use crate::analyzer::DataSource;
use crate::types::ConversationMessage;

/// One file's parsed messages, shared rather than copied out of the cache.
type Parsed = Arc<Vec<ConversationMessage>>;

/// Size and mtime of one file, or `None` where it does not exist.
type Stamp = Vec<(u64, Option<SystemTime>)>;

struct Entry {
    stamp: Stamp,
    messages: Parsed,
}

/// What a pass had to do. Useful for logging and for tests that want to prove the cache did not
/// re-read unchanged files.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StampReport {
    pub unchanged: usize,
    pub parsed: usize,
    pub removed: usize,
    pub failed: usize,
}

#[derive(Default)]
pub struct StampCache {
    files: Mutex<HashMap<PathBuf, Entry>>,
}

impl StampCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Entry>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Messages for every source, parsing only files whose size or mtime moved since last time.
    ///
    /// `parse` is called only for new or changed files. A file that fails to parse is reported and
    /// contributes nothing, and is retried next pass rather than cached as empty.
    pub fn parse_all<F>(
        &self,
        sources: &[DataSource],
        parse: F,
    ) -> (Vec<ConversationMessage>, StampReport)
    where
        F: Fn(&Path) -> Result<Vec<ConversationMessage>> + Sync,
    {
        self.parse_all_stamping(sources, |_| Vec::new(), parse)
    }

    /// Like [`Self::parse_all`], but also stamping files the parser reads *beside* each source.
    ///
    /// A source is often not the only file its parse depends on: a Kiro session's `.json` names
    /// the session while the sibling `.jsonl` carries the events, and a Droid session's
    /// `.settings.json` carries the authoritative token counts. Stamping only the discovered path
    /// would serve stale messages whenever the companion is the file that moved.
    pub fn parse_all_stamping<F, C>(
        &self,
        sources: &[DataSource],
        companions: C,
        parse: F,
    ) -> (Vec<ConversationMessage>, StampReport)
    where
        F: Fn(&Path) -> Result<Vec<ConversationMessage>> + Sync,
        C: Fn(&Path) -> Vec<PathBuf> + Sync,
    {
        // Overlapping glob patterns can list the same file twice; parse it once.
        let mut seen = HashSet::new();
        let paths: Vec<&Path> = sources
            .iter()
            .map(|s| s.path.as_path())
            .filter(|p| seen.insert(*p))
            .collect();

        let results: Vec<(Option<Parsed>, Outcome)> = paths
            .par_iter()
            .map(|path| self.parse_one(path, &companions, &parse))
            .collect();

        let mut report = StampReport::default();
        let mut chunks: Vec<Parsed> = Vec::with_capacity(results.len());
        for (parsed, outcome) in results {
            match outcome {
                Outcome::Unchanged => report.unchanged += 1,
                Outcome::Parsed => report.parsed += 1,
                Outcome::Failed => report.failed += 1,
            }
            if let Some(parsed) = parsed {
                chunks.push(parsed);
            }
        }

        // Forget files that are no longer discovered, so a deleted session stops counting.
        let live: HashSet<&Path> = paths.iter().copied().collect();
        let mut files = self.lock();
        let before = files.len();
        files.retain(|p, _| live.contains(p.as_path()));
        report.removed = before - files.len();
        drop(files);

        let total: usize = chunks.iter().map(|c| c.len()).sum();
        let mut messages = Vec::with_capacity(total);
        for chunk in &chunks {
            messages.extend(chunk.iter().cloned());
        }
        (messages, report)
    }

    fn parse_one<F, C>(&self, path: &Path, companions: &C, parse: &F) -> (Option<Parsed>, Outcome)
    where
        F: Fn(&Path) -> Result<Vec<ConversationMessage>>,
        C: Fn(&Path) -> Vec<PathBuf>,
    {
        // Stat before reading, so a write landing mid-parse shows up as a change next pass rather
        // than being silently absorbed into a stamp that claims to cover it.
        if std::fs::metadata(path).is_err() {
            self.lock().remove(path);
            return (None, Outcome::Failed);
        }
        let stamp: Stamp = std::iter::once(path.to_path_buf())
            .chain(companions(path))
            .map(|file| {
                std::fs::metadata(file)
                    .map(|m| (m.len(), m.modified().ok()))
                    .unwrap_or((0, None))
            })
            .collect();

        if let Some(entry) = self.lock().get(path) {
            if entry.stamp == stamp {
                return (Some(Arc::clone(&entry.messages)), Outcome::Unchanged);
            }
        }

        match parse(path) {
            Ok(messages) => {
                let messages: Parsed = Arc::new(messages);
                self.lock().insert(
                    path.to_path_buf(),
                    Entry {
                        stamp,
                        messages: Arc::clone(&messages),
                    },
                );
                (Some(messages), Outcome::Parsed)
            }
            Err(e) => {
                // Do not cache a failure: a half-written file should be retried, not remembered
                // as empty until it happens to change again.
                self.lock().remove(path);
                crate::utils::report(format!("Failed to parse {}: {e:#}", path.display()));
                (None, Outcome::Failed)
            }
        }
    }
}

enum Outcome {
    Unchanged,
    Parsed,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Application, MessageRole, Stats};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn msg(tag: &str) -> ConversationMessage {
        ConversationMessage {
            application: Application::QwenCode,
            date: chrono::Utc::now(),
            project_hash: "p".into(),
            conversation_hash: "c".into(),
            local_hash: None,
            global_hash: tag.to_string(),
            model: None,
            stats: Stats::default(),
            role: MessageRole::User,
            content: Some(tag.to_string()),
        }
    }

    /// Counts calls so a test can prove an unchanged file was not handed to the parser.
    fn counting_parser(
        calls: &AtomicUsize,
    ) -> impl Fn(&Path) -> Result<Vec<ConversationMessage>> + Sync + '_ {
        move |path| {
            calls.fetch_add(1, Ordering::SeqCst);
            let body = std::fs::read_to_string(path)?;
            Ok(body.lines().map(msg).collect())
        }
    }

    fn src(paths: &[&Path]) -> Vec<DataSource> {
        paths
            .iter()
            .map(|p| DataSource {
                path: p.to_path_buf(),
            })
            .collect()
    }

    fn tags(messages: &[ConversationMessage]) -> Vec<String> {
        let mut t: Vec<String> = messages.iter().map(|m| m.global_hash.clone()).collect();
        t.sort();
        t
    }

    /// The acceptance criterion for BZL-74: a second pass over an unchanged file must not read it.
    #[test]
    fn unchanged_file_is_not_parsed_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();

        let (first, report) = cache.parse_all(&src(&[&file]), counting_parser(&calls));
        assert_eq!(tags(&first), ["one", "two"]);
        assert_eq!(report.parsed, 1);

        let (second, report) = cache.parse_all(&src(&[&file]), counting_parser(&calls));
        assert_eq!(tags(&second), ["one", "two"], "same result from the cache");
        assert_eq!(report.unchanged, 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "an unchanged file must not be handed to the parser"
        );
    }

    #[test]
    fn changed_file_is_reparsed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "one\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();
        cache.parse_all(&src(&[&file]), counting_parser(&calls));

        // A different length is a change whatever the mtime granularity.
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        let (after, report) = cache.parse_all(&src(&[&file]), counting_parser(&calls));

        assert_eq!(tags(&after), ["one", "three", "two"]);
        assert_eq!(report.parsed, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// Only the file that changed is re-read; its neighbours stay cached.
    #[test]
    fn a_write_next_door_does_not_reparse_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            dir.path().join("a.json"),
            dir.path().join("b.json"),
            dir.path().join("c.json"),
        );
        for (p, body) in [(&a, "a1\n"), (&b, "b1\n"), (&c, "c1\n")] {
            std::fs::write(p, body).unwrap();
        }
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();
        let sources = src(&[&a, &b, &c]);
        cache.parse_all(&sources, counting_parser(&calls));
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        std::fs::write(&b, "b1\nb2\n").unwrap();
        let (all, report) = cache.parse_all(&sources, counting_parser(&calls));

        assert_eq!(report.parsed, 1, "only b changed");
        assert_eq!(report.unchanged, 2);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            4,
            "one extra parse, not three"
        );
        assert_eq!(tags(&all), ["a1", "b1", "b2", "c1"]);
    }

    #[test]
    fn files_that_disappear_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
        std::fs::write(&a, "a1\n").unwrap();
        std::fs::write(&b, "b1\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();
        cache.parse_all(&src(&[&a, &b]), counting_parser(&calls));

        // b is no longer discovered, so its messages must stop counting.
        let (only_a, report) = cache.parse_all(&src(&[&a]), counting_parser(&calls));
        assert_eq!(tags(&only_a), ["a1"]);
        assert_eq!(report.removed, 1);
    }

    /// A file that fails to parse (half-written, say) must be retried rather than remembered as
    /// empty until it next changes.
    #[test]
    fn a_parse_failure_is_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "one\n").unwrap();
        let attempts = AtomicUsize::new(0);
        let cache = StampCache::new();

        let flaky = |path: &Path| -> Result<Vec<ConversationMessage>> {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                anyhow::bail!("truncated");
            }
            Ok(std::fs::read_to_string(path)?.lines().map(msg).collect())
        };

        let (first, report) = cache.parse_all(&src(&[&file]), &flaky);
        assert!(first.is_empty());
        assert_eq!(report.failed, 1);

        // Same size and mtime, but the failure was not cached, so it is tried again.
        let (second, report) = cache.parse_all(&src(&[&file]), &flaky);
        assert_eq!(tags(&second), ["one"]);
        assert_eq!(report.parsed, 1);
    }

    /// A parse that reads a companion file must be invalidated when the companion moves, not
    /// only when the discovered path does. Kiro's session `.json` is static while the sibling
    /// `.jsonl` of events is the file that grows, so stamping the `.json` alone serves stale data.
    #[test]
    fn a_changed_companion_invalidates_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("s.json");
        let events = dir.path().join("s.jsonl");
        std::fs::write(&meta, "m1\n").unwrap();
        std::fs::write(&events, "e1\n").unwrap();

        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();
        let companions = |p: &Path| vec![p.with_extension("jsonl")];
        let parse = |p: &Path| -> Result<Vec<ConversationMessage>> {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut out: Vec<ConversationMessage> =
                std::fs::read_to_string(p)?.lines().map(msg).collect();
            out.extend(
                std::fs::read_to_string(p.with_extension("jsonl"))
                    .unwrap_or_default()
                    .lines()
                    .map(msg),
            );
            Ok(out)
        };

        let (first, _) = cache.parse_all_stamping(&src(&[&meta]), companions, &parse);
        assert_eq!(tags(&first), ["e1", "m1"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Only the companion grows; the discovered path is byte-identical.
        std::fs::write(&events, "e1\ne2\n").unwrap();
        let (after, report) = cache.parse_all_stamping(&src(&[&meta]), companions, &parse);

        assert_eq!(report.parsed, 1, "the companion moved, so re-parse");
        assert_eq!(tags(&after), ["e1", "e2", "m1"]);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// With the companion unchanged too, nothing is re-read.
    #[test]
    fn unchanged_companion_keeps_the_entry_cached() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("s.json");
        std::fs::write(&meta, "m1\n").unwrap();
        std::fs::write(dir.path().join("s.jsonl"), "e1\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();
        let companions = |p: &Path| vec![p.with_extension("jsonl")];

        cache.parse_all_stamping(&src(&[&meta]), companions, counting_parser(&calls));
        let (_, report) =
            cache.parse_all_stamping(&src(&[&meta]), companions, counting_parser(&calls));

        assert_eq!(report.unchanged, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_source_paths_are_parsed_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "one\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = StampCache::new();

        let (all, _) = cache.parse_all(&src(&[&file, &file]), counting_parser(&calls));
        assert_eq!(tags(&all), ["one"], "listed twice, counted once");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

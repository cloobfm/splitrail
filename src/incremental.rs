//! Incremental, per-file parse cache for append-only JSONL data sources.
//!
//! Analyzers used to re-parse every byte of every file on every reload. For Claude Code that
//! meant ~1.2 GB of JSON on all cores each time a session file grew by a few hundred bytes.
//! This cache remembers, per file, how far it has parsed and what it produced, so a reload
//! costs one `stat` per file plus parsing only the bytes appended since last time.
//!
//! Rules:
//! - A file whose size and mtime are unchanged is not opened; its cached messages are reused.
//! - A file that grew is parsed from the previous end offset with the parser state carried over.
//! - A file that shrank or is new is parsed from the beginning with fresh state.
//! - Only complete, newline-terminated lines are consumed. A partial trailing line (a writer
//!   mid-append) is left for the next reload, so it is never parsed twice or half-parsed.
//! - Files that disappear from the source list are dropped from the cache.
//!
//! The append fast path assumes files are append-only, which holds for every JSONL log this
//! tool reads. A file rewritten in place with a longer length would be parsed from a stale
//! offset; the worst case is a few "invalid entry" warnings until the file is next rewritten.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::analyzer::DataSource;
use crate::types::ConversationMessage;

/// What a chunk parser returns: the messages it produced and the number of bytes it consumed
/// from the reader. `bytes_consumed` must only count complete lines the parser accepted, so the
/// cache can resume exactly there next time.
pub type ChunkResult = (Vec<ConversationMessage>, u64);

/// One file's parsed messages, shared with the cache rather than copied out of it.
pub type MessageChunk = Arc<Vec<ConversationMessage>>;

struct CachedFile<S> {
    len: u64,
    modified: Option<SystemTime>,
    /// Byte offset just past the last complete line that has been parsed.
    parsed_bytes: u64,
    /// Parser state carried across appends (e.g. a project label learned from the first line).
    state: S,
}

/// Counts of what a refresh had to do. Useful for logging and for tests that want to prove the
/// cache did not re-read unchanged files.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    pub unchanged: usize,
    pub appended: usize,
    pub reparsed: usize,
    pub removed: usize,
    /// Bytes actually handed to the parser this refresh.
    pub bytes_parsed: u64,
    /// A file we had already consumed was rewritten, shrank, or vanished. Callers fold results
    /// across refreshes, and those events invalidate what was already folded, so the caller must
    /// discard its accumulator and start from a clean cache. A *first* parse never sets this.
    pub needs_rebuild: bool,
}

enum Outcome {
    Unchanged,
    Appended(u64),
    /// A file we had never seen: nothing folded from it can be stale.
    FirstParse(u64),
    /// A file we had already consumed, re-read from the top because it shrank or was rewritten.
    Reparsed(u64),
    Missing,
}

pub struct IncrementalJsonlCache<S> {
    files: Mutex<HashMap<PathBuf, CachedFile<S>>>,
}

impl<S> Default for IncrementalJsonlCache<S> {
    fn default() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
        }
    }
}

impl<S: Default + Send + 'static> IncrementalJsonlCache<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bring the cache up to date with `sources` and return every file's messages as a shared
    /// chunk, in source order. `parse` is called only for new, grown, or rewritten files, with a
    /// reader positioned where parsing should resume and the state from the previous chunk.
    ///
    /// Only the messages parsed *this* refresh come back: nothing for an unchanged file, just the
    /// appended lines for a grown one. The cache remembers cursors, not contents, so callers must
    /// fold what they are handed and check [`RefreshReport::needs_rebuild`].
    pub fn refresh<F>(&self, sources: &[DataSource], parse: F) -> (Vec<MessageChunk>, RefreshReport)
    where
        F: Fn(&Path, &mut BufReader<File>, &mut S) -> ChunkResult + Sync,
    {
        // Overlapping glob patterns can list a file twice; parse each path once.
        let mut seen = HashSet::new();
        let paths: Vec<&Path> = sources
            .iter()
            .map(|s| s.path.as_path())
            .filter(|p| seen.insert(*p))
            .collect();

        let results: Vec<(MessageChunk, Outcome)> = paths
            .par_iter()
            .map(|path| self.refresh_one(path, &parse))
            .collect();

        let mut report = RefreshReport::default();
        let mut invalidated = false;
        let mut chunks = Vec::with_capacity(results.len());
        for (msgs, outcome) in results {
            match outcome {
                Outcome::Unchanged => report.unchanged += 1,
                Outcome::Appended(n) => {
                    report.appended += 1;
                    report.bytes_parsed += n;
                }
                Outcome::FirstParse(n) => {
                    report.reparsed += 1;
                    report.bytes_parsed += n;
                }
                Outcome::Reparsed(n) => {
                    report.reparsed += 1;
                    report.bytes_parsed += n;
                    invalidated = true;
                }
                Outcome::Missing => {
                    report.removed += 1;
                    invalidated = true;
                }
            }
            // An empty chunk carries nothing downstream; skipping it keeps a missing or
            // still-empty file from padding the result.
            if !msgs.is_empty() {
                chunks.push(msgs);
            }
        }

        // Forget files that are no longer discovered.
        let live: HashSet<&Path> = paths.iter().copied().collect();
        let mut files = self.lock();
        let before = files.len();
        files.retain(|p, _| live.contains(p.as_path()));
        report.removed += before - files.len();
        // Set last: the retain above can also drop files, and a dropped file invalidates folded
        // state just as a rewrite does.
        report.needs_rebuild = invalidated || before != files.len();

        (chunks, report)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, CachedFile<S>>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn refresh_one<F>(&self, path: &Path, parse: &F) -> (MessageChunk, Outcome)
    where
        F: Fn(&Path, &mut BufReader<File>, &mut S) -> ChunkResult,
    {
        // Stat first, before reading, so a write that lands during parsing shows up as a size
        // change on the next refresh instead of being silently absorbed.
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                self.lock().remove(path);
                return (MessageChunk::default(), Outcome::Missing);
            }
        };
        let len = meta.len();
        let modified = meta.modified().ok();

        // Take ownership of the entry while we work on it; the lock is not held during I/O.
        let existing = self.lock().remove(path);

        match existing {
            Some(entry) if entry.len == len && entry.modified == modified => {
                self.lock().insert(path.to_path_buf(), entry);
                (MessageChunk::default(), Outcome::Unchanged)
            }
            Some(mut entry) if len >= entry.parsed_bytes => {
                let Ok(mut file) = File::open(path) else {
                    return (MessageChunk::default(), Outcome::Missing);
                };
                if file.seek(SeekFrom::Start(entry.parsed_bytes)).is_err() {
                    return self.reparse(path, len, modified, parse, true);
                }
                let mut reader = BufReader::new(file);
                let (new_messages, consumed) = parse(path, &mut reader, &mut entry.state);
                entry.parsed_bytes += consumed;
                entry.len = len;
                entry.modified = modified;
                self.lock().insert(path.to_path_buf(), entry);
                (Arc::new(new_messages), Outcome::Appended(consumed))
            }
            existing => {
                let was_known = existing.is_some();
                self.reparse(path, len, modified, parse, was_known)
            }
        }
    }

    /// Forget every cursor so the next refresh re-reads everything. Used when a caller's folded
    /// state has been invalidated (see [`RefreshReport::needs_rebuild`]).
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn reparse<F>(
        &self,
        path: &Path,
        len: u64,
        modified: Option<SystemTime>,
        parse: &F,
        was_known: bool,
    ) -> (MessageChunk, Outcome)
    where
        F: Fn(&Path, &mut BufReader<File>, &mut S) -> ChunkResult,
    {
        let Ok(file) = File::open(path) else {
            return (MessageChunk::default(), Outcome::Missing);
        };
        let mut reader = BufReader::new(file);
        let mut state = S::default();
        let (messages, consumed) = parse(path, &mut reader, &mut state);
        self.lock().insert(
            path.to_path_buf(),
            CachedFile {
                len,
                modified,
                parsed_bytes: consumed,
                state,
            },
        );
        let outcome = if was_known {
            Outcome::Reparsed(consumed)
        } else {
            Outcome::FirstParse(consumed)
        };
        (Arc::new(messages), outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Application, MessageRole, Stats};
    use std::fs;
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test parser: one message per complete line, content = "<running line number>:<text>".
    /// State is the running line number so tests can prove it carries across appends.
    fn line_parser(
        calls: &AtomicUsize,
    ) -> impl Fn(&Path, &mut BufReader<File>, &mut usize) -> ChunkResult + Sync + '_ {
        move |_path, reader, line_no| {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut out = Vec::new();
            let mut consumed = 0u64;
            let mut buf = Vec::new();
            loop {
                buf.clear();
                let n = reader.read_until(b'\n', &mut buf).unwrap();
                if n == 0 || buf.last() != Some(&b'\n') {
                    break; // EOF or an unterminated tail: leave it for next time
                }
                consumed += n as u64;
                *line_no += 1;
                let text = String::from_utf8_lossy(&buf[..n - 1]).to_string();
                out.push(ConversationMessage {
                    application: Application::ClaudeCode,
                    date: chrono::Utc::now(),
                    project_hash: "p".into(),
                    conversation_hash: "c".into(),
                    local_hash: None,
                    global_hash: format!("{line_no}"),
                    model: None,
                    stats: Stats::default(),
                    role: MessageRole::User,
                    content: Some(format!("{line_no}:{text}")),
                });
            }
            (out, consumed)
        }
    }

    /// Flatten the per-file chunks a refresh returns into their message contents, in order.
    fn contents(chunks: &[MessageChunk]) -> Vec<String> {
        chunks
            .iter()
            .flat_map(|c| c.iter())
            .map(|m| m.content.clone().unwrap())
            .collect()
    }

    fn append(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        f.sync_all().unwrap();
    }

    fn src(path: &Path) -> Vec<DataSource> {
        vec![DataSource {
            path: path.to_path_buf(),
        }]
    }

    /// The cache's job is to avoid re-*reading* unchanged files. Retaining every message it has
    /// ever parsed so it can hand them back each refresh is a different thing, and it is what kept
    /// the whole corpus resident (BZL-14). It must remember cursors, not contents.
    #[test]
    fn unchanged_files_yield_nothing_because_the_cache_holds_no_messages() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\nb\nc\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();

        let (first, _) = cache.refresh(&src(&file), line_parser(&calls));
        assert_eq!(contents(&first), ["1:a", "2:b", "3:c"]);

        let (second, _) = cache.refresh(&src(&file), line_parser(&calls));
        assert!(
            contents(&second).is_empty(),
            "the cache has nothing stored to hand back"
        );

        append(&file, "d\n");
        let (third, _) = cache.refresh(&src(&file), line_parser(&calls));
        assert_eq!(contents(&third), ["4:d"], "only the appended line is new");
    }

    #[test]
    fn first_refresh_parses_everything_and_second_refresh_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\nb\nc\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();

        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        assert_eq!(contents(&msgs), ["1:a", "2:b", "3:c"]);
        assert_eq!(report.reparsed, 1);
        assert_eq!(report.bytes_parsed, 6);

        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        assert!(
            contents(&msgs).is_empty(),
            "an unchanged file was not read, so it yields nothing new"
        );
        assert_eq!(report.unchanged, 1);
        assert_eq!(report.bytes_parsed, 0);
        assert!(!report.needs_rebuild, "a first parse invalidates nothing");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "unchanged file must not be re-read"
        );
    }

    #[test]
    fn appended_lines_are_parsed_from_the_old_offset_with_state_carried() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\nb\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();
        cache.refresh(&src(&file), line_parser(&calls));

        append(&file, "c\nd\n");
        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        // Line numbers 3 and 4 prove the parser state continued rather than restarting at 1,
        // and only the appended lines come back.
        assert_eq!(contents(&msgs), ["3:c", "4:d"]);
        assert_eq!(report.appended, 1);
        assert_eq!(report.bytes_parsed, 4, "only the appended bytes are parsed");
    }

    #[test]
    fn partial_trailing_line_waits_for_its_newline_and_is_parsed_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();
        cache.refresh(&src(&file), line_parser(&calls));

        append(&file, "part");
        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        assert!(
            contents(&msgs).is_empty(),
            "half-written line must not appear"
        );
        assert_eq!(report.appended, 1);
        assert_eq!(report.bytes_parsed, 0);

        append(&file, "ial\n");
        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        assert_eq!(contents(&msgs), ["2:partial"], "only the now-complete line");
        assert_eq!(
            report.bytes_parsed, 8,
            "the whole line is parsed exactly once"
        );
    }

    #[test]
    fn shrunk_file_is_reparsed_from_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\nb\nc\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();
        cache.refresh(&src(&file), line_parser(&calls));

        fs::write(&file, "x\n").unwrap();
        let (msgs, report) = cache.refresh(&src(&file), line_parser(&calls));
        assert_eq!(contents(&msgs), ["1:x"]);
        assert_eq!(report.reparsed, 1);
    }

    #[test]
    fn files_that_disappear_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.jsonl");
        let b = dir.path().join("b.jsonl");
        fs::write(&a, "a\n").unwrap();
        fs::write(&b, "b\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();
        let sources = vec![
            DataSource { path: a.clone() },
            DataSource { path: b.clone() },
        ];
        let (msgs, _) = cache.refresh(&sources, line_parser(&calls));
        assert_eq!(msgs.len(), 2);

        // b is no longer discovered. a is unchanged so yields nothing, but dropping b invalidates
        // anything a caller folded from it.
        let (msgs, report) = cache.refresh(&src(&a), line_parser(&calls));
        assert!(contents(&msgs).is_empty());
        assert_eq!(report.removed, 1);
        assert!(report.needs_rebuild, "a vanished file invalidates folded state");

        // a is deleted from disk while still listed
        fs::remove_file(&a).unwrap();
        let (msgs, report) = cache.refresh(&src(&a), line_parser(&calls));
        assert!(msgs.is_empty());
        assert_eq!(report.removed, 1);
    }

    #[test]
    fn duplicate_source_paths_are_parsed_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        fs::write(&file, "a\n").unwrap();
        let calls = AtomicUsize::new(0);
        let cache = IncrementalJsonlCache::<usize>::new();
        let sources = vec![
            DataSource { path: file.clone() },
            DataSource { path: file.clone() },
        ];
        let (msgs, report) = cache.refresh(&sources, line_parser(&calls));
        assert_eq!(msgs.len(), 1);
        assert_eq!(report.reparsed, 1);
    }
}

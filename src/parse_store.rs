//! What a headless publisher has parsed, saved to disk so a restart does not read every
//! transcript again.
//!
//! The parse caches (`incremental.rs` for Claude Code, Codex's own) live in memory, so each
//! start used to re-read every session file in the window: ~4 GB of small random reads on
//! devbox-a, whose disk sits on NAS hard drives beside a trading engine. With the store, a start
//! reads one file per analyzer, stats each source, and parses only bytes written since the save.
//!
//! Only the `mqtt` publisher turns it on ([`enable`]). Messages are saved without their text.
//! A store that is missing, unreadable or from another schema is ignored, and everything is
//! parsed as before. Costs are as priced when the line was parsed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::types::ConversationMessage;

/// Bump when the saved layout or the meaning of a parsed message changes.
const SCHEMA: u32 = 1;
/// Saving rewrites the whole store, so do it at most this often while files keep changing.
const SAVE_EVERY: Duration = Duration::from_secs(600);

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Turn the store on, kept in `dir`.
pub fn enable(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn enabled() -> bool {
    DIR.get().is_some()
}

/// `~/.cache/splitrail/parsed`.
pub fn default_dir() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".cache/splitrail/parsed"))
}

/// One source file as last parsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredFile<S> {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<SystemTime>,
    /// Bytes consumed so far; parsing resumes here if the file has grown.
    pub parsed_bytes: u64,
    pub state: S,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Serialize, Deserialize)]
struct Saved<S> {
    schema: u32,
    files: Vec<StoredFile<S>>,
}

fn file_for(dir: &Path, analyzer: &str) -> PathBuf {
    dir.join(format!("{}.json", crate::mqtt::slug(analyzer)))
}

/// The analyzer's saved files by path, or nothing if the store is off or unusable.
pub fn load<S: DeserializeOwned>(analyzer: &str) -> HashMap<PathBuf, StoredFile<S>> {
    DIR.get().map(|dir| load_from(dir, analyzer)).unwrap_or_default()
}

fn load_from<S: DeserializeOwned>(dir: &Path, analyzer: &str) -> HashMap<PathBuf, StoredFile<S>> {
    let Ok(file) = std::fs::File::open(file_for(dir, analyzer)) else {
        return HashMap::new();
    };
    match serde_json::from_reader::<_, Saved<S>>(std::io::BufReader::new(file)) {
        Ok(saved) if saved.schema == SCHEMA => {
            saved.files.into_iter().map(|f| (f.path.clone(), f)).collect()
        }
        _ => HashMap::new(),
    }
}

/// Write the analyzer's files, without message text, replacing the previous save atomically.
pub fn save<S: Serialize>(analyzer: &str, files: Vec<StoredFile<S>>) {
    if let Some(dir) = DIR.get() {
        if let Err(e) = save_to(dir, analyzer, files) {
            crate::utils::log_error(format!("Saving parsed {analyzer} files: {e:#}"));
        }
    }
}

fn save_to<S: Serialize>(dir: &Path, analyzer: &str, mut files: Vec<StoredFile<S>>) -> anyhow::Result<()> {
    use std::io::Write;
    for file in &mut files {
        for message in &mut file.messages {
            message.content = None;
        }
    }
    std::fs::create_dir_all(dir)?;
    let target = file_for(dir, analyzer);
    let tmp = target.with_extension("json.tmp");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    serde_json::to_writer(&mut out, &Saved { schema: SCHEMA, files })?;
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// When an analyzer should save next: soon after a parse that read anything, but at most every
/// [`SAVE_EVERY`] while files keep growing.
#[derive(Default)]
pub struct SaveSchedule {
    inner: Mutex<(bool, Option<Instant>)>,
}

impl SaveSchedule {
    /// Record a parse; `changed` when it read bytes or dropped files. True when a save is due.
    pub fn due(&self, changed: bool) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.0 |= changed;
        let (dirty, last) = *inner;
        if dirty && last.is_none_or(|at| at.elapsed() >= SAVE_EVERY) {
            *inner = (false, Some(Instant::now()));
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Application, MessageRole, Stats};

    fn message(content: &str) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date: chrono::Utc::now(),
            project_hash: "p".into(),
            conversation_hash: "c".into(),
            local_hash: None,
            global_hash: "g".into(),
            model: Some("m".into()),
            stats: Stats { output_tokens: 7, ..Stats::default() },
            role: MessageRole::Assistant,
            content: Some(content.into()),
        }
    }

    #[test]
    fn a_save_loads_back_without_message_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = StoredFile {
            path: PathBuf::from("/x/a.jsonl"),
            len: 10,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(5)),
            parsed_bytes: 9,
            state: 3u32,
            messages: vec![message("secret")],
        };
        save_to(dir.path(), "Claude (main)", vec![file]).unwrap();
        let loaded: HashMap<PathBuf, StoredFile<u32>> = load_from(dir.path(), "Claude (main)");
        let back = &loaded[Path::new("/x/a.jsonl")];
        assert_eq!((back.len, back.parsed_bytes, back.state), (10, 9, 3));
        assert_eq!(back.modified, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(5)));
        assert_eq!(back.messages[0].stats.output_tokens, 7);
        assert!(back.messages[0].content.is_none());
    }

    #[test]
    fn a_missing_or_foreign_store_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from::<u32>(dir.path(), "Codex").is_empty());
        std::fs::write(file_for(dir.path(), "Codex"), r#"{"schema":999,"files":[]}"#).unwrap();
        assert!(load_from::<u32>(dir.path(), "Codex").is_empty());
        std::fs::write(file_for(dir.path(), "Codex"), "not json").unwrap();
        assert!(load_from::<u32>(dir.path(), "Codex").is_empty());
    }

    #[test]
    fn saves_wait_for_changes_and_are_spaced_out() {
        let schedule = SaveSchedule::default();
        assert!(!schedule.due(false), "nothing read, nothing to save");
        assert!(schedule.due(true), "first change saves at once");
        assert!(!schedule.due(true), "the next waits SAVE_EVERY");
    }
}

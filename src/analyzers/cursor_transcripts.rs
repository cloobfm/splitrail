//! Cursor's own agent transcripts, for the transcript feed while a turn is still running.
//!
//! `cursor-agent` appends each step of a conversation to
//! `~/.cursor/projects/<cwd with / as ->/agent-transcripts/<conversation>/<conversation>.jsonl`
//! as it happens, one JSON object per line:
//!
//! ```json
//! {"role":"user","message":{"content":[{"type":"text","text":"<user_query>\nfix it\n</user_query>"}]}}
//! {"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}
//! {"role":"assistant","message":{"content":[{"type":"text","text":"Found the bug."}]}}
//! {"type":"turn_ended","status":"success"}
//! ```
//!
//! The hooks only fire when a turn starts and ends, so these files are the only sign of progress
//! in between. They carry no tokens, model or times: the tokens still come from the hooks and
//! billing (`cursor_agent`), and each line is stamped with the time splitrail first saw it, kept
//! in `~/.config/splitrail/cursor/transcript_times.json` so it does not move on the next read.
//! Subagents (a review, a search) write their own conversation next to their parent's.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One step of a transcript, as the feed shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// The user's (or, in a subagent, the parent agent's) request.
    Prompt(String),
    /// Text the agent wrote between tool calls.
    Text(String),
    /// Tool calls made in a row, by name.
    Tools(Vec<String>),
}

/// A transcript read from disk, with a time for every item (epoch seconds).
#[derive(Debug, Clone)]
pub struct Transcript {
    pub conversation_id: String,
    /// The `~/.cursor/projects/<slug>` folder name.
    pub project_slug: String,
    /// When the file was created (its first line), epoch seconds.
    pub started: f64,
    pub items: Vec<(f64, Item)>,
}

/// The `~/.cursor/projects` folder name Cursor uses for a working directory.
pub fn project_slug(cwd: &str) -> String {
    let mut slug = String::new();
    for c in cwd.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// The request inside a prompt line, without Cursor's `<timestamp>` and `<user_query>` wrapping.
fn user_query(text: &str) -> String {
    let inner = match (text.find("<user_query>"), text.rfind("</user_query>")) {
        (Some(start), Some(end)) if end > start => &text[start + "<user_query>".len()..end],
        _ => text,
    };
    let inner = match (inner.find("<timestamp>"), inner.find("</timestamp>")) {
        (Some(start), Some(end)) if end > start => {
            format!("{}{}", &inner[..start], &inner[end + "</timestamp>".len()..])
        }
        _ => inner.to_string(),
    };
    inner.trim().to_string()
}

/// The items of one transcript line. Hidden reasoning (`[REDACTED]`) and blank text are dropped;
/// a line that is not JSON gives none.
pub fn line_items(line: &str) -> Vec<Item> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    let user = value.get("role").and_then(|r| r.as_str()) == Some("user");
    let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    let mut items: Vec<Item> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
                if text.is_empty() || text == "[REDACTED]" {
                    continue;
                }
                items.push(if user { Item::Prompt(user_query(text)) } else { Item::Text(text.to_string()) });
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool").to_string();
                match items.last_mut() {
                    Some(Item::Tools(names)) => names.push(name),
                    _ => items.push(Item::Tools(vec![name])),
                }
            }
            _ => {}
        }
    }
    items
}

/// Times for a transcript's `total` complete lines. Lines stamped on an earlier read keep their
/// time; new ones are spread evenly from the last stamped line (or the file's creation) to its
/// last write, so a line that appears while splitrail watches gets about the time it was written.
pub fn stamp_lines(known: &[f64], total: usize, created: f64, modified: f64) -> Vec<f64> {
    let mut times: Vec<f64> = known.iter().copied().take(total).collect();
    let new = total - times.len();
    if new > 0 {
        let from = times.last().copied().unwrap_or(created).min(modified);
        for i in 1..=new {
            times.push(from + (modified - from) * i as f64 / new as f64);
        }
    }
    times
}

/// Turn a transcript's text into items with times, merging tool calls in a row across lines.
/// A last line without its newline is still being written and is left for the next read.
pub fn parse_transcript(content: &str, times: &[f64]) -> Vec<(f64, Item)> {
    let mut items: Vec<(f64, Item)> = Vec::new();
    for (line, &at) in complete_lines(content).zip(times) {
        for item in line_items(line) {
            match (items.last_mut(), item) {
                (Some((last_at, Item::Tools(names))), Item::Tools(more)) => {
                    names.extend(more);
                    *last_at = at;
                }
                (_, item) => items.push((at, item)),
            }
        }
    }
    items
}

fn complete_lines(content: &str) -> impl Iterator<Item = &str> {
    let complete = match content.rfind('\n') {
        Some(end) => &content[..=end],
        None => "",
    };
    complete.lines()
}

fn epoch(time: SystemTime) -> f64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Default, Serialize, Deserialize)]
struct TimesFile {
    /// Conversation id -> the time each line was first seen.
    #[serde(default)]
    lines: HashMap<String, Vec<f64>>,
}

/// Reads the transcripts and keeps their line times across reads and restarts.
pub struct TranscriptReader {
    projects_dir: Option<PathBuf>,
    times_file: Option<PathBuf>,
    /// Path -> (size, mtime) and the transcript read then.
    cache: std::sync::Mutex<HashMap<PathBuf, ((u64, f64), Transcript)>>,
    times: std::sync::Mutex<Option<TimesFile>>,
}

impl TranscriptReader {
    pub fn new(projects_dir: Option<PathBuf>, times_file: Option<PathBuf>) -> Self {
        Self {
            projects_dir,
            times_file,
            cache: Default::default(),
            times: Default::default(),
        }
    }

    pub fn for_home() -> Self {
        Self::new(
            std::env::home_dir().map(|home| home.join(".cursor").join("projects")),
            crate::cursor_limits::charges_file()
                .and_then(|path| path.parent().map(|dir| dir.join("transcript_times.json"))),
        )
    }

    /// The pattern to watch for new transcript lines.
    pub fn glob_pattern(&self) -> Option<String> {
        self.projects_dir
            .as_ref()
            .map(|dir| format!("{}/**/agent-transcripts/*/*.jsonl", dir.display()))
    }

    /// The project folders holding a transcript of any of `conversations`.
    pub fn folders_of(&self, conversations: &std::collections::HashSet<String>) -> std::collections::HashSet<String> {
        let Some(projects) = &self.projects_dir else {
            return Default::default();
        };
        let pattern = format!("{}/*/agent-transcripts/*", projects.display());
        glob::glob(&pattern)
            .map(|paths| {
                paths
                    .flatten()
                    .filter(|dir| {
                        dir.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| conversations.contains(n))
                    })
                    .filter_map(|dir| dir.parent()?.parent()?.file_name()?.to_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every transcript of the conversations `wanted` says to read (by project slug and
    /// conversation id), each read again only when its file changed.
    pub fn read(&self, wanted: impl Fn(&str, &str) -> bool) -> Vec<Transcript> {
        let Some(projects) = &self.projects_dir else {
            return Vec::new();
        };
        let pattern = format!("{}/*/agent-transcripts/*/*.jsonl", projects.display());
        let Ok(paths) = glob::glob(&pattern) else {
            return Vec::new();
        };
        let mut times = self.times.lock().unwrap_or_else(|e| e.into_inner());
        let times = times.get_or_insert_with(|| self.load_times());
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = false;
        let mut out = Vec::new();
        for path in paths.flatten() {
            let Some((slug, conversation)) = names(&path) else { continue };
            if !wanted(&slug, &conversation) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let modified = meta.modified().map(epoch).unwrap_or(0.0);
            let stamp = (meta.len(), modified);
            if let Some((cached, transcript)) = cache.get(&path) {
                if *cached == stamp {
                    out.push(transcript.clone());
                    continue;
                }
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let created = meta.created().map(epoch).unwrap_or(modified);
            let total = complete_lines(&content).count();
            let known = times.lines.get(&conversation).map(Vec::as_slice).unwrap_or(&[]);
            let stamped = stamp_lines(known, total, created, modified);
            if stamped.as_slice() != known {
                times.lines.insert(conversation.clone(), stamped.clone());
                changed = true;
            }
            let transcript = Transcript {
                conversation_id: conversation,
                project_slug: slug,
                started: stamped.first().copied().unwrap_or(created).min(created),
                items: parse_transcript(&content, &stamped),
            };
            cache.insert(path, (stamp, transcript.clone()));
            out.push(transcript);
        }
        if changed {
            self.save_times(times);
        }
        out
    }

    fn load_times(&self) -> TimesFile {
        self.times_file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    fn save_times(&self, times: &TimesFile) {
        let Some(path) = &self.times_file else { return };
        let Ok(json) = serde_json::to_string(times) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// The project slug and conversation id of `<projects>/<slug>/agent-transcripts/<id>/<id>.jsonl`.
fn names(path: &Path) -> Option<(String, String)> {
    let conversation = path.file_stem()?.to_str()?.to_string();
    let slug = path.parent()?.parent()?.parent()?.file_name()?.to_str()?.to_string();
    Some((slug, conversation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_lines_become_prompts_notes_and_runs_of_tool_calls() {
        let content = concat!(
            r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Wed, 12:14 AM</timestamp>\n<user_query>\nreview this project\n</user_query>"}]}}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"Looking at the manifest first."}]}}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"path":"a"}}]}}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Grep","input":{}}]}}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"[REDACTED]"}]}}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}"#,
            "\n",
            r#"{"type":"turn_ended","status":"success"}"#,
            "\n",
            r#"{"role":"assistant","message":{"content":[{"type":"te"#, // still being written
        );
        let times: Vec<f64> = (1..=7).map(f64::from).collect();
        let items = parse_transcript(content, &times);
        assert_eq!(
            items,
            vec![
                (1.0, Item::Prompt("review this project".into())),
                (2.0, Item::Text("Looking at the manifest first.".into())),
                // The hidden reasoning between them does not split the run.
                (6.0, Item::Tools(vec!["Read".into(), "Grep".into(), "Read".into()])),
            ]
        );
    }

    #[test]
    fn a_line_keeps_the_time_it_was_first_seen() {
        // First read: three lines written between the file's creation (100) and now (130).
        let first = stamp_lines(&[], 3, 100.0, 130.0);
        assert_eq!(first, vec![110.0, 120.0, 130.0]);
        // A minute later one more line: the earlier three do not move.
        let second = stamp_lines(&first, 4, 100.0, 190.0);
        assert_eq!(second, vec![110.0, 120.0, 130.0, 190.0]);
        // A rewritten (shorter) file keeps only what it still has.
        assert_eq!(stamp_lines(&second, 2, 100.0, 200.0), vec![110.0, 120.0]);
    }

    #[test]
    fn the_project_folder_is_the_working_directory_with_dashes() {
        assert_eq!(project_slug("/Users/bzl/Projects/tosmirror"), "Users-bzl-Projects-tosmirror");
        assert_eq!(project_slug("/private/tmp/claude-501/-Users-bzl-x/"), "private-tmp-claude-501-Users-bzl-x");
        assert_eq!(
            names(Path::new("/h/.cursor/projects/Users-x-p/agent-transcripts/c1/c1.jsonl")),
            Some(("Users-x-p".to_string(), "c1".to_string()))
        );
    }
}

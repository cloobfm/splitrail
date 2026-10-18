use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::analyzer::{Analyzer, DataSource};
use crate::models::calculate_total_cost;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

/// OpenCode conversation data structures
/// Based on actual OpenCode storage format found in ~/.local/share/opencode/

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSession {
    id: String,
    version: String,
    #[serde(rename = "projectID")]
    project_id: String,
    directory: String,
    title: String,
    time: OpenCodeTime,
    summary: OpenCodeSummary,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeTime {
    created: u64,
    updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSummary {
    additions: u64,
    deletions: u64,
    files: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodePart {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "messageID")]
    message_id: String,
    #[serde(rename = "type")]
    part_type: String,
    text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageTime {
    created: u64,
    completed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageData {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    role: String,
    time: OpenCodeMessageTime,
    #[serde(rename = "parentID")]
    parent_id: Option<String>,
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
    mode: Option<String>,
    path: Option<OpenCodePath>,
    cost: Option<f64>,
    tokens: Option<OpenCodeTokens>,
    finish: Option<String>,
    /// Unused, and its shape varies (an object, sometimes without a title, or a bare bool);
    /// typed strictly it dropped the whole message.
    summary: Option<serde_json::Value>,
    agent: Option<String>,
    model: Option<OpenCodeMessageModel>,
    tools: Option<OpenCodeMessageTools>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodePath {
    cwd: Option<String>,
    root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache: Option<OpenCodeCache>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeCache {
    read: u64,
    write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageModel {
    #[serde(rename = "providerID")]
    provider_id: String,
    #[serde(rename = "modelID")]
    model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeMessageTools {
    todowrite: Option<bool>,
    todoread: Option<bool>,
    task: Option<bool>,
}

/// Which OpenCode-format tool an analyzer reads: OpenCode itself, or Kilo CLI (a fork that
/// keeps the same storage under its own name).
#[derive(Debug, Clone)]
pub struct Flavor {
    pub display_name: &'static str,
    /// Under the home directory: `.local/share/opencode`.
    pub data_dir: &'static str,
    /// `opencode` reads `opencode.db` and channel databases like `opencode-canary.db`.
    pub db_stem: &'static str,
    pub application: Application,
    /// Prefixes the global hash, so the tools' message ids never collide.
    pub hash_prefix: &'static str,
}

pub const OPENCODE: Flavor = Flavor {
    display_name: "OpenCode",
    data_dir: ".local/share/opencode",
    db_stem: "opencode",
    application: Application::OpenCode,
    hash_prefix: "opencode",
};

pub const KILO_CLI: Flavor = Flavor {
    display_name: "Kilo CLI",
    data_dir: ".local/share/kilo",
    db_stem: "kilo",
    application: Application::KiloCli,
    hash_prefix: "kilo_cli",
};

pub struct OpenCodeAnalyzer {
    flavor: Flavor,
    /// Messages read from each database, kept between reloads. OpenCode writes logs and
    /// snapshots next to the database constantly, and every write there triggers a reload;
    /// re-reading 13k+ rows each time would pin a core.
    db_cache: std::sync::Mutex<std::collections::HashMap<PathBuf, DbCache>>,
}

#[derive(Default)]
struct DbCache {
    /// (len, mtime) of the database and its WAL when last read; unchanged means nothing new.
    stamp: Option<Vec<(u64, Option<std::time::SystemTime>)>>,
    /// Newest `time_updated` seen per table; only rows at or after it are read again.
    watermark_v2: i64,
    watermark_v1: i64,
    messages: std::collections::HashMap<String, ConversationMessage>,
}

impl Default for OpenCodeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenCodeAnalyzer {
    pub fn new() -> Self {
        Self::for_flavor(OPENCODE)
    }

    pub fn for_flavor(flavor: Flavor) -> Self {
        Self {
            flavor,
            db_cache: Default::default(),
        }
    }

    fn data_dir(&self) -> Option<PathBuf> {
        std::env::home_dir().map(|home| home.join(self.flavor.data_dir))
    }

    /// Whether `name` is one of this tool's databases: `opencode.db` or `opencode-<channel>.db`.
    fn is_database_name(&self, name: &str) -> bool {
        let stem = self.flavor.db_stem;
        name == format!("{stem}.db")
            || name
                .strip_prefix(stem)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|rest| {
                    rest.strip_suffix(".db")
                        .is_some_and(|channel| !channel.is_empty())
                })
    }

    /// The tool's SQLite databases, default channel first.
    fn databases(&self) -> Vec<PathBuf> {
        let Some(Ok(entries)) = self.data_dir().map(std::fs::read_dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| self.is_database_name(n))
            })
            .collect();
        let default = format!("{}.db", self.flavor.db_stem);
        found.sort_by_key(|p| {
            (
                p.file_name().and_then(|n| n.to_str()) != Some(default.as_str()),
                p.clone(),
            )
        });
        found
    }

    /// Messages in the database, reading only rows changed since the last call.
    fn read_database(&self, path: &Path) -> Result<Vec<ConversationMessage>> {
        let stamp: Vec<_> = ["", "-wal"]
            .iter()
            .map(|suffix| {
                let file = PathBuf::from(format!("{}{suffix}", path.display()));
                std::fs::metadata(file)
                    .map(|m| (m.len(), m.modified().ok()))
                    .unwrap_or((0, None))
            })
            .collect();
        let mut caches = self.db_cache.lock().unwrap_or_else(|e| e.into_inner());
        let cache = caches.entry(path.to_path_buf()).or_default();
        if cache.stamp.as_ref() != Some(&stamp) {
            let (rows, v2, v1) =
                parse_database(self, path, cache.watermark_v2, cache.watermark_v1)?;
            for (id, msg) in rows {
                cache.messages.insert(id, msg);
            }
            cache.watermark_v2 = cache.watermark_v2.max(v2);
            cache.watermark_v1 = cache.watermark_v1.max(v1);
            cache.stamp = Some(stamp);
        }
        let mut messages: Vec<_> = cache.messages.values().cloned().collect();
        messages.sort_by_key(|m| m.date);
        Ok(messages)
    }
}

#[async_trait]
impl Analyzer for OpenCodeAnalyzer {
    fn display_name(&self) -> &'static str {
        self.flavor.display_name
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        let mut patterns = Vec::new();

        if let Some(data_dir) = self.data_dir() {
            let home_str = data_dir.to_string_lossy();
            let stem = self.flavor.db_stem;

            // OPTIMIZED: Only read message files (contain all stats) and sessions (for project context)
            // Message files contain complete token counts, model info, and timing
            patterns.push(format!("{home_str}/storage/message/*/msg_*.json"));

            // Session metadata (read once for project context) - handle both directory structures
            patterns.push(format!("{home_str}/storage/session/*/ses_*.json"));
            patterns.push(format!("{home_str}/storage/message/ses_*.json"));

            // OpenCode 1.x+ keeps everything in SQLite; the WAL changes on every write, so watching
            // it surfaces new turns live.
            patterns.push(format!("{home_str}/{stem}.db"));
            patterns.push(format!("{home_str}/{stem}.db-wal"));
            patterns.push(format!("{home_str}/{stem}-*.db"));
            patterns.push(format!("{home_str}/{stem}-*.db-wal"));

            // REMOVED: Part files - these are individual message fragments, text content not needed for stats
            // Part files are only necessary if we need to display actual conversation content
        }

        patterns
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        // The database holds every message the legacy JSON files do (checked 2026-10-13: all
        // 13,564 present), so with it the JSON files are not read at all.
        let databases = self.databases();
        if !databases.is_empty() {
            return Ok(databases
                .into_iter()
                .map(|path| DataSource { path })
                .collect());
        }
        let patterns = self.get_data_glob_patterns();
        let mut sources = Vec::new();

        for pattern in patterns {
            for entry in glob::glob(&pattern)? {
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
        let mut messages = Vec::new();
        let mut seen_hashes = std::collections::HashSet::new();

        // SQLite databases first; their messages share ids with the legacy JSON files.
        let (databases, sources): (Vec<_>, Vec<_>) = sources
            .into_iter()
            .partition(|s| s.path.extension().is_some_and(|e| e == "db"));
        for db in databases {
            match self.read_database(&db.path) {
                Ok(parsed) => {
                    for msg in parsed {
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                }
                Err(e) => crate::utils::warn_once(format!(
                    "{}: could not read {}: {e:#}",
                    self.flavor.display_name,
                    db.path.display()
                )),
            }
        }

        // Separate message and session files
        let mut message_files = Vec::new();
        let mut session_files = Vec::new();
        let mut uncategorized_files = Vec::new(); // For test files and others

        for source in sources {
            // Check the actual filename, not the full path
            // Message files: msg_*.json
            // Session files: ses_*.json
            if let Some(filename) = source.path.file_name().and_then(|f| f.to_str()) {
                if filename.starts_with("ses_") {
                    session_files.push(source);
                } else if filename.starts_with("msg_") {
                    message_files.push(source);
                } else {
                    // For files that don't match expected patterns (like test temp files),
                    // add them to uncategorized so we can try to parse them as messages
                    uncategorized_files.push(source);
                }
            } else {
                // If we can't get filename, treat as potential message file
                uncategorized_files.push(source);
            }
        }

        // Load session metadata (read once, used for project context)
        let mut sessions = std::collections::HashMap::new();
        for session_file in session_files {
            if let Ok(content) = std::fs::read_to_string(&session_file.path)
                && let Ok(session) = serde_json::from_str::<OpenCodeSession>(&content)
            {
                sessions.insert(session.id.clone(), session);
            }
        }

        // Process message files
        for message_file in message_files {
            if let Ok(content) = std::fs::read_to_string(&message_file.path) {
                // Try to parse as single JSON object first, then as JSONL if that fails
                if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(&content) {
                    // Single JSON object format
                    // Get session info for directory path
                    let session_dir = sessions
                        .get(&opencode_msg.session_id)
                        .map(|s| s.directory.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    // Read actual message content from part files for both user and assistant
                    let message_content = self.read_message_text_content(&opencode_msg.id);

                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message_data(
                        opencode_msg,
                        &session_dir,
                        message_content,
                    ) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                } else {
                    // Try parsing as JSONL (multiple JSON objects separated by newlines)
                    for line in content.lines() {
                        let line = line.trim();
                        if !line.is_empty()
                            && let Ok(opencode_msg) =
                                serde_json::from_str::<OpenCodeMessageData>(line)
                        {
                            // Get session info for directory path
                            let session_dir = sessions
                                .get(&opencode_msg.session_id)
                                .map(|s| s.directory.clone())
                                .unwrap_or_else(|| "unknown".to_string());

                            // Read actual message content from part files for both user and assistant
                            let message_content = self.read_message_text_content(&opencode_msg.id);

                            // Convert to our internal format
                            if let Some(msg) = self.convert_opencode_message_data(
                                opencode_msg,
                                &session_dir,
                                message_content,
                            ) {
                                // Deduplicate by global hash
                                if seen_hashes.insert(msg.global_hash.clone()) {
                                    messages.push(msg);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process uncategorized files (for test files and any other valid message files)
        for message_file in uncategorized_files {
            if let Ok(content) = std::fs::read_to_string(&message_file.path) {
                // Try to parse as single JSON object first, then as JSONL if that fails
                if let Ok(opencode_msg) = serde_json::from_str::<OpenCodeMessageData>(&content) {
                    // Single JSON object format
                    // Get session info for directory path
                    let session_dir = sessions
                        .get(&opencode_msg.session_id)
                        .map(|s| s.directory.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    // Read actual message content from part files for both user and assistant
                    let message_content = self.read_message_text_content(&opencode_msg.id);

                    // Convert to our internal format
                    if let Some(msg) = self.convert_opencode_message_data(
                        opencode_msg,
                        &session_dir,
                        message_content,
                    ) {
                        // Deduplicate by global hash
                        if seen_hashes.insert(msg.global_hash.clone()) {
                            messages.push(msg);
                        }
                    }
                } else {
                    // Try parsing as JSONL (multiple JSON objects separated by newlines)
                    for line in content.lines() {
                        let line = line.trim();
                        if !line.is_empty()
                            && let Ok(opencode_msg) =
                                serde_json::from_str::<OpenCodeMessageData>(line)
                        {
                            // Get session info for directory path
                            let session_dir = sessions
                                .get(&opencode_msg.session_id)
                                .map(|s| s.directory.clone())
                                .unwrap_or_else(|| "unknown".to_string());

                            // Read actual message content from part files for both user and assistant
                            let message_content = self.read_message_text_content(&opencode_msg.id);

                            // Convert to our internal format
                            if let Some(msg) = self.convert_opencode_message_data(
                                opencode_msg,
                                &session_dir,
                                message_content,
                            ) {
                                // Deduplicate by global hash
                                if seen_hashes.insert(msg.global_hash.clone()) {
                                    messages.push(msg);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;

        let mut daily_stats = std::collections::BTreeMap::new();

        for message in &messages {
            // Convert UTC timestamp to local timezone for daily stats grouping
            let local_date = message.date.with_timezone(&chrono::Local);
            let date_str = local_date.format("%Y-%m-%d").to_string();
            let daily =
                daily_stats
                    .entry(date_str.clone())
                    .or_insert_with(|| crate::types::DailyStats {
                        date: date_str,
                        user_messages: 0,
                        ai_messages: 0,
                        conversations: 0,
                        active_seconds: 0,
                        models: std::collections::BTreeMap::new(),
                        stats: crate::types::Stats::default(),
                        slices: Vec::new(),
                    });

            match message.role {
                crate::types::MessageRole::User => daily.user_messages += 1,
                crate::types::MessageRole::Assistant => {
                    daily.ai_messages += 1;
                    if let Some(ref model) = message.model {
                        *daily.models.entry(model.clone()).or_insert(0) += 1;
                        let slice = crate::utils::slice_mut(
                            daily,
                            crate::utils::local_hour(&message.date),
                            model,
                            &message.project_hash,
                        );
                        crate::utils::add_to_slice(slice, &message.stats, message.stats.cost, 1);
                    }
                }
            }

            // Add stats
            daily.stats.input_tokens += message.stats.input_tokens;
            daily.stats.output_tokens += message.stats.output_tokens;
            daily.stats.reasoning_tokens += message.stats.reasoning_tokens;
            daily.stats.cache_creation_tokens += message.stats.cache_creation_tokens;
            daily.stats.cache_read_tokens += message.stats.cache_read_tokens;
            daily.stats.cached_tokens += message.stats.cached_tokens;
            daily.stats.cost += message.stats.cost;
            daily.stats.tool_calls += message.stats.tool_calls;
            daily.stats.terminal_commands += message.stats.terminal_commands;
            daily.stats.file_searches += message.stats.file_searches;
            daily.stats.file_content_searches += message.stats.file_content_searches;
            daily.stats.files_read += message.stats.files_read;
            daily.stats.files_added += message.stats.files_added;
            daily.stats.files_edited += message.stats.files_edited;
            daily.stats.files_deleted += message.stats.files_deleted;
            daily.stats.lines_read += message.stats.lines_read;
            daily.stats.lines_added += message.stats.lines_added;
            daily.stats.lines_edited += message.stats.lines_edited;
            daily.stats.lines_deleted += message.stats.lines_deleted;
            daily.stats.bytes_read += message.stats.bytes_read;
            daily.stats.bytes_added += message.stats.bytes_added;
            daily.stats.bytes_edited += message.stats.bytes_edited;
            daily.stats.bytes_deleted += message.stats.bytes_deleted;
        }

        // Count unique conversations
        let unique_conversations: std::collections::HashSet<_> =
            messages.iter().map(|m| &m.conversation_hash).collect();
        let num_conversations = unique_conversations.len() as u64;

        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        // Installed: its config or data directory exists.
        let config =
            std::env::home_dir().map(|home| home.join(".config").join(self.flavor.db_stem));
        config.is_some_and(|c| c.exists()) || self.data_dir().is_some_and(|d| d.exists())
    }
}

impl OpenCodeAnalyzer {
    /// Read text content from part files for a specific message
    fn read_message_text_content(&self, message_id: &str) -> Option<String> {
        // Find part directory for this message
        if let Some(data_dir) = self.data_dir() {
            let part_dir = data_dir.join("storage/part").join(message_id);

            if !part_dir.exists() {
                return None;
            }

            // Read all part files for this message and collect text
            let mut text_parts = Vec::new();

            if let Ok(entries) = std::fs::read_dir(&part_dir) {
                for entry in entries.flatten() {
                    if let Ok(content) = std::fs::read_to_string(entry.path())
                        && let Ok(part) = serde_json::from_str::<OpenCodePart>(&content)
                    {
                        // Only collect "text" type parts
                        if part.part_type == "text"
                            && let Some(text) = part.text
                            && !text.trim().is_empty()
                        {
                            text_parts.push(text);
                        }
                    }
                }
            }

            if text_parts.is_empty() {
                None
            } else {
                Some(text_parts.join(" "))
            }
        } else {
            None
        }
    }

    fn convert_opencode_message_data(
        &self,
        msg: OpenCodeMessageData,
        project_path: &str,
        content: Option<String>,
    ) -> Option<ConversationMessage> {
        // Determine role: user messages have actual user text in part files
        let role = match msg.role.to_lowercase().as_str() {
            "user" => MessageRole::User,
            "assistant" | "ai" => MessageRole::Assistant,
            _ => return None,
        };

        // Use cwd from message path if available, otherwise fall back to session directory
        let effective_project_path = msg
            .path
            .as_ref()
            .and_then(|p| p.cwd.as_ref())
            .filter(|cwd| !cwd.is_empty())
            .map(|cwd| {
                // Extract just the directory name from the full path
                Path::new(cwd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(cwd)
            })
            .unwrap_or_else(|| project_path);

        // Generate hashes - use the actual project name directly like other analyzers
        let project_hash = effective_project_path.to_string();
        let conversation_hash = hash_text(&msg.session_id);
        let local_hash = Some(hash_text(&format!("{}-{}", msg.id, msg.time.created)));
        let global_hash = hash_text(&format!(
            "{}-{}-{}",
            self.flavor.hash_prefix, msg.id, msg.time.created
        ));

        // Extract model information - FIXED: read from correct field
        let model = msg
            .model_id
            .clone()
            .or_else(|| msg.model.as_ref().map(|m| m.model_id.clone()))
            .or(Some("opencode-zen".to_string())); // fallback

        // Convert stats - FIXED: properly read token counts from message
        let mut stats = Stats::default();

        if let Some(tokens) = msg.tokens {
            stats.input_tokens = tokens.input;
            stats.output_tokens = tokens.output;
            stats.reasoning_tokens = tokens.reasoning;
            if let Some(cache) = tokens.cache {
                stats.cache_read_tokens = cache.read;
                stats.cache_creation_tokens = cache.write;
                stats.cached_tokens = cache.read;
            }
        }

        // Count tool calls based on finish type
        if let Some(finish) = &msg.finish
            && finish == "tool-calls"
        {
            stats.tool_calls = 1;
        }

        // Calculate cost using our pricing model (OpenCode stores cost=0 for free tier)
        if let Some(ref model_name) = model {
            stats.cost = calculate_total_cost(
                model_name,
                stats.input_tokens,
                stats.output_tokens,
                stats.cache_creation_tokens,
                stats.cache_read_tokens,
            );
        }

        // FIXED: Use actual message timestamp (milliseconds since epoch)
        let timestamp = DateTime::from_timestamp_millis(msg.time.created as i64)
            .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());

        Some(ConversationMessage {
            application: self.flavor.application.clone(),
            date: timestamp,
            project_hash,
            conversation_hash,
            local_hash,
            global_hash,
            model,
            stats,
            role,
            content, // Text content from part files
        })
    }
}

/// Whether `table` exists; the schema depends on the OpenCode version that last ran.
fn has_table(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}

/// Rows read from a database (by message id), and the v2 and v1 watermarks reached.
type DatabaseRead = (Vec<(String, ConversationMessage)>, i64, i64);

/// Messages from `opencode.db`, read-only. OpenCode 1.x stores them in `message` (data holds
/// the old JSON file's fields minus id/sessionID); 2.x in `session_message` (type
/// user/assistant/...; assistant data has `model.id`, `tokens`, `cost` and a `content` list of
/// text/reasoning/tool parts). 2.x copied part of the 1.x history over, so ids repeat across
/// the two and the caller deduplicates.
/// Rows whose `time_updated` is at or after the given watermarks, keyed by message id (a 2.x
/// row wins over its 1.x copy), with the newest `time_updated` seen in each table.
fn parse_database(
    analyzer: &OpenCodeAnalyzer,
    path: &Path,
    since_v2: i64,
    since_v1: i64,
) -> Result<DatabaseRead> {
    use anyhow::Context;
    use rusqlite::OpenFlags;
    let conn = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("open")?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;

    let mut directories = std::collections::HashMap::new();
    for table in ["session", "session_v2"] {
        if has_table(&conn, table) {
            let mut stmt = conn.prepare(&format!("SELECT id, directory FROM {table}"))?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows.flatten() {
                directories.insert(row.0, row.1);
            }
        }
    }
    // Project names are the folder name, as messages with a `path.cwd` already get.
    let directory = |session: &str| {
        directories
            .get(session)
            .map(|dir| {
                Path::new(dir)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(dir)
                    .to_string()
            })
            .unwrap_or_else(|| "unknown".to_string())
    };

    let mut messages: Vec<(String, ConversationMessage)> = Vec::new();
    let mut v2_ids = std::collections::HashSet::new();
    let (mut max_v2, mut max_v1) = (since_v2, since_v1);

    if has_table(&conn, "session_message") {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, type, time_created, data, time_updated FROM session_message \
             WHERE type IN ('user', 'assistant') AND time_updated >= ?1",
        )?;
        let rows = stmt.query_map([since_v2], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        for (id, session, role, created, data, updated) in rows.flatten() {
            max_v2 = max_v2.max(updated);
            if let Some(msg) = convert_v2_row(
                analyzer,
                &id,
                &session,
                &role,
                created,
                &data,
                &directory(&session),
            ) {
                v2_ids.insert(id.clone());
                messages.push((id, msg));
            }
        }
    }

    if has_table(&conn, "message") {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, data, time_updated FROM message WHERE time_updated >= ?1",
        )?;
        let rows = stmt.query_map([since_v1], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for (id, session, data, updated) in rows.flatten() {
            max_v1 = max_v1.max(updated);
            if v2_ids.contains(&id) {
                continue;
            }
            let Ok(serde_json::Value::Object(mut fields)) = serde_json::from_str(&data) else {
                continue;
            };
            fields.insert("id".into(), id.clone().into());
            fields.insert("sessionID".into(), session.clone().into());
            if let Ok(parsed) = serde_json::from_value::<OpenCodeMessageData>(fields.into())
                && let Some(msg) =
                    analyzer.convert_opencode_message_data(parsed, &directory(&session), None)
            {
                messages.push((id, msg));
            }
        }
    }
    Ok((messages, max_v2, max_v1))
}

/// One OpenCode 2.x `session_message` row, mapped onto the 1.x message shape.
fn convert_v2_row(
    analyzer: &OpenCodeAnalyzer,
    id: &str,
    session: &str,
    role: &str,
    row_created: i64,
    data: &str,
    directory: &str,
) -> Option<ConversationMessage> {
    let data: serde_json::Value = serde_json::from_str(data).ok()?;
    let created = data
        .pointer("/time/created")
        .and_then(|v| v.as_u64())
        .unwrap_or(row_created.max(0) as u64);
    let parts = data.get("content").and_then(|c| c.as_array());
    let tool_calls = parts
        .map(|p| p.iter().filter(|part| part["type"] == "tool").count() as u32)
        .unwrap_or(0);
    let text = match role {
        "user" => data
            .get("text")
            .and_then(|t| t.as_str())
            .map(str::to_string),
        _ => parts.map(|p| {
            p.iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        }),
    }
    .filter(|t| !t.trim().is_empty());
    let model = data.get("model");
    let shaped = serde_json::json!({
        "id": id,
        "sessionID": session,
        "role": role,
        "time": { "created": created },
        "modelID": model.and_then(|m| m.get("id")).cloned(),
        "providerID": model.and_then(|m| m.get("providerID")).cloned(),
        "tokens": data.get("tokens").cloned(),
        "cost": data.get("cost").cloned(),
    });
    let parsed: OpenCodeMessageData = serde_json::from_value(shaped).ok()?;
    let mut msg = analyzer.convert_opencode_message_data(parsed, directory, text)?;
    msg.stats.tool_calls = tool_calls;
    Some(msg)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn database_rows_from_both_schemas_are_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (id text, directory text);
            CREATE TABLE session_v2 (id text, directory text);
            CREATE TABLE message (id text, session_id text, time_created integer, time_updated integer, data text);
            CREATE TABLE session_message (id text, session_id text, type text, seq integer,
                time_created integer, time_updated integer, data text);
            INSERT INTO session VALUES ('s1', '/p/old');
            INSERT INTO session_v2 VALUES ('s2', '/p/new');
            -- 1.x: an assistant turn, plus one also copied into 2.x, plus one with a bool summary.
            INSERT INTO message VALUES ('m1', 's1', 0, 1, '{"role":"assistant","time":{"created":1760000000000},"modelID":"glm-4.6","providerID":"zai","tokens":{"input":10,"output":20,"reasoning":0,"cache":{"read":5,"write":0}},"cost":0}');
            INSERT INTO message VALUES ('m2', 's2', 0, 1, '{"role":"user","time":{"created":1760000001000},"summary":true}');
            INSERT INTO message VALUES ('m3', 's1', 0, 1, '{"role":"user","time":{"created":1760000002000},"summary":{"diffs":[]}}');
            -- 2.x
            INSERT INTO session_message VALUES ('m2', 's2', 'user', 1, 1760000001000, 1760000001000, '{"time":{"created":1760000001000},"text":"hi"}');
            INSERT INTO session_message VALUES ('m4', 's2', 'assistant', 2, 1760000002000, 1760000002000, '{"time":{"created":1760000002000},"model":{"id":"cat-jam-free","providerID":"opencode"},"tokens":{"input":7,"output":3,"reasoning":1,"cache":{"read":100,"write":0}},"cost":0,"content":[{"type":"text","text":"done"},{"type":"tool","name":"shell"},{"type":"tool","name":"read"}]}');
            INSERT INTO session_message VALUES ('i1', 's2', 'idle', 3, 1760000003000, 1760000003000, '{}');
            "#,
        )
        .unwrap();
        drop(conn);

        let analyzer = OpenCodeAnalyzer::new();
        let messages = analyzer
            .parse_conversations(vec![DataSource { path: path.clone() }])
            .await
            .unwrap();
        assert_eq!(messages.len(), 4); // m1, m2 once, m3, m4; the idle row is skipped
        let m4 = messages
            .iter()
            .find(|m| m.model.as_deref() == Some("cat-jam-free"))
            .unwrap();
        assert_eq!(m4.stats.output_tokens, 3);
        assert_eq!(m4.stats.cache_read_tokens, 100);
        assert_eq!(m4.stats.tool_calls, 2);
        assert_eq!(m4.project_hash, "new");
        assert_eq!(m4.content.as_deref(), Some("done"));
        let m1 = messages
            .iter()
            .find(|m| m.model.as_deref() == Some("glm-4.6"))
            .unwrap();
        assert_eq!(m1.project_hash, "old");

        // A turn streamed further: only that row is read again, and it replaces the old one.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE session_message SET time_updated = 1760000009000, data = json_set(data, '$.tokens.output', 30) WHERE id = 'm4'",
            [],
        )
        .unwrap();
        drop(conn);
        let messages = analyzer
            .parse_conversations(vec![DataSource { path }])
            .await
            .unwrap();
        assert_eq!(messages.len(), 4);
        let m4 = messages
            .iter()
            .find(|m| m.model.as_deref() == Some("cat-jam-free"))
            .unwrap();
        assert_eq!(m4.stats.output_tokens, 30);
    }

    /// Reads the real database: `cargo test opencode_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn opencode_live_database_matches_sqlite() {
        let analyzer = OpenCodeAnalyzer::new();
        let stats = analyzer.get_stats().await.unwrap();
        let assistant = stats
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let day = stats.daily_stats.get(&today);
        println!(
            "messages {} (assistant {assistant}); today output {:?} input {:?} cache read {:?} cost {:?}",
            stats.messages.len(),
            day.map(|d| d.stats.output_tokens),
            day.map(|d| d.stats.input_tokens),
            day.map(|d| d.stats.cache_read_tokens),
            day.map(|d| d.stats.cost),
        );
        assert!(assistant > 0);
    }

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_parse_opencode_conversations() {
        let analyzer = OpenCodeAnalyzer::new();

        // Create a temporary file with sample OpenCode data
        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000},"modelID":null,"tokens":null,"finish":null}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"opencode-zen","tokens":{"input":10,"output":5,"reasoning":0},"finish":"stop"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // Check the assistant message
        let assistant_msg = &messages[1];
        assert_eq!(assistant_msg.role, MessageRole::Assistant);
        assert_eq!(assistant_msg.model, Some("opencode-zen".to_string()));
        assert_eq!(assistant_msg.stats.input_tokens, 10);
        assert_eq!(assistant_msg.stats.output_tokens, 5);
        assert_eq!(assistant_msg.stats.tool_calls, 0); // finish is "stop", not "tool-calls"
    }

    #[tokio::test]
    async fn test_parse_opencode_with_missing_fields() {
        let analyzer = OpenCodeAnalyzer::new();

        // Test with missing optional fields
        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000}}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"test-model"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // Should handle missing tokens/tools/files gracefully
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].stats.input_tokens, 0); // Default when missing
        assert_eq!(messages[1].stats.output_tokens, 0);
    }

    #[tokio::test]
    async fn test_parse_opencode_with_different_roles() {
        let analyzer = OpenCodeAnalyzer::new();

        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"user","time":{"created":1704110400000}}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000}}
{"id":"msg3","sessionID":"test-session","role":"ai","time":{"created":1704110520000}}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 3);

        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[2].role, MessageRole::Assistant); // "ai" maps to Assistant
    }

    #[tokio::test]
    async fn test_parse_opencode_tool_call_detection() {
        let analyzer = OpenCodeAnalyzer::new();

        let mut temp_file = NamedTempFile::new().unwrap();
        let sample_data = r#"
{"id":"msg1","sessionID":"test-session","role":"assistant","time":{"created":1704110400000},"modelID":"test","tokens":{"input":5,"output":3,"reasoning":0},"finish":"tool-calls"}
{"id":"msg2","sessionID":"test-session","role":"assistant","time":{"created":1704110460000},"modelID":"test","tokens":{"input":5,"output":3,"reasoning":0},"finish":"stop"}
"#;
        temp_file.write_all(sample_data.as_bytes()).unwrap();

        let sources = vec![DataSource {
            path: temp_file.path().to_path_buf(),
        }];

        let messages = analyzer.parse_conversations(sources).await.unwrap();
        assert_eq!(messages.len(), 2);

        // First message has tool-calls finish reason
        assert_eq!(messages[0].stats.tool_calls, 1);
        // Second message has stop finish reason
        assert_eq!(messages[1].stats.tool_calls, 0);
    }

    #[test]
    fn test_file_categorization() {
        let _analyzer = OpenCodeAnalyzer::new();

        // Mock file paths that match the glob patterns
        let message_file = DataSource {
            path: std::path::PathBuf::from(
                "/home/user/.local/share/opencode/storage/message/session123/msg_001.json",
            ),
        };
        let session_file = DataSource {
            path: std::path::PathBuf::from(
                "/home/user/.local/share/opencode/storage/session/session123/ses_001.json",
            ),
        };
        let sources = vec![message_file, session_file];

        let (message_files, session_files) = {
            let mut message_files = Vec::new();
            let mut session_files = Vec::new();

            for source in sources {
                let path_str = source.path.to_string_lossy();
                if path_str.contains("message/") {
                    message_files.push(source);
                } else if path_str.contains("session/") {
                    session_files.push(source);
                }
            }
            (message_files, session_files)
        };

        assert_eq!(message_files.len(), 1);
        assert_eq!(session_files.len(), 1);
        assert!(
            message_files[0]
                .path
                .to_string_lossy()
                .contains("msg_001.json")
        );
        assert!(
            session_files[0]
                .path
                .to_string_lossy()
                .contains("ses_001.json")
        );
    }

    #[test]
    fn test_convert_opencode_message_data_comprehensive() {
        let analyzer = OpenCodeAnalyzer::new();

        // Test comprehensive message conversion
        let msg_data = OpenCodeMessageData {
            id: "test-msg".to_string(),
            session_id: "test-session".to_string(),
            role: "assistant".to_string(),
            time: OpenCodeMessageTime {
                created: 1704110400000, // 2024-01-01 12:00:00 UTC
                completed: Some(1704110460000),
            },
            model: Some(OpenCodeMessageModel {
                provider_id: "opencode".to_string(),
                model_id: "zen".to_string(),
            }),
            tokens: Some(OpenCodeTokens {
                input: 100,
                output: 50,
                reasoning: 10,
                cache: Some(OpenCodeCache { read: 20, write: 5 }),
            }),
            finish: Some("tool-calls".to_string()),
            tools: Some(OpenCodeMessageTools {
                todowrite: Some(true),
                todoread: Some(false),
                task: Some(true),
            }),
            parent_id: None,
            mode: None,
            path: None,
            cost: None,
            summary: None,
            agent: None,
            model_id: None,
            provider_id: None,
        };

        let result = analyzer.convert_opencode_message_data(msg_data, "test-project", None);

        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.model, Some("zen".to_string()));
        assert_eq!(msg.stats.input_tokens, 100);
        assert_eq!(msg.stats.output_tokens, 50);
        assert_eq!(msg.stats.reasoning_tokens, 10);
        assert_eq!(msg.stats.cache_read_tokens, 20);
        assert_eq!(msg.stats.cache_creation_tokens, 5);
        assert_eq!(msg.stats.tool_calls, 1);
        assert_eq!(msg.project_hash, "test-project");
    }

    #[test]
    fn kilo_cli_reads_its_own_databases_under_its_own_name() {
        let kilo = OpenCodeAnalyzer::for_flavor(KILO_CLI);
        assert!(kilo.is_database_name("kilo.db"));
        assert!(kilo.is_database_name("kilo-canary.db"));
        for other in [
            "kilo.db-wal",
            "kilo-.db",
            "kilo-canary.db-shm",
            "opencode.db",
            "kilocode.db",
        ] {
            assert!(!kilo.is_database_name(other), "{other}");
        }
        assert!(OpenCodeAnalyzer::new().is_database_name("opencode.db"));
        assert_eq!(kilo.display_name(), "Kilo CLI");

        let msg = |analyzer: &OpenCodeAnalyzer| {
            let data: OpenCodeMessageData = serde_json::from_value(serde_json::json!({
                "id": "msg1", "sessionID": "s1", "role": "assistant", "time": {"created": 1760000000000u64},
                "modelID": "kilo-auto/free", "tokens": {"input": 5, "output": 2, "reasoning": 0}
            }))
            .unwrap();
            analyzer
                .convert_opencode_message_data(data, "proj", None)
                .unwrap()
        };
        let (k, o) = (msg(&kilo), msg(&OpenCodeAnalyzer::new()));
        assert_eq!(k.application, Application::KiloCli);
        assert_ne!(
            k.global_hash, o.global_hash,
            "the same message id in both tools stays two messages"
        );
    }
}

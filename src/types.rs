use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Application {
    ClaudeCode,
    GeminiCli,
    QwenCode,
    CodexCli,
    Cline,
    RooCode,
    KiloCode,
    /// Kilo's OpenCode-based CLI (`~/.local/share/kilo`), as opposed to the extension and the
    /// older CLI under `~/.kilocode` (`KiloCode`).
    KiloCli,
    Copilot,
    AmazonQ,
    KiroCli,
    Warp,
    OpenCode,
    DroidCli,
    Antigravity,
    Ollama,
    CursorAgent,
    /// GitHub Copilot CLI (`copilot`), as opposed to Copilot Chat in VS Code (`Copilot`).
    CopilotCli,
    PiAgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub application: Application,
    #[serde(rename = "date")]
    pub date: DateTime<Utc>,
    pub project_hash: String,
    pub conversation_hash: String,
    /// The hash of this message, local to the application that we're gathering data from.  E.g.,
    /// in the Claude Code analyzer, this will be set to the message's hash within Claude Code.
    /// This is an Option because sometimes there's no way to generate, and so no need for,
    /// a local hash.
    pub local_hash: Option<String>,
    /// The hash of this message, global to the Splitrail instance.  This is used on the server to
    /// ensure that, in the event that messages that have been previously uploaded to the server
    /// are reuploaded, they are not redundantly inserted into the database and cause incorrectly
    /// inflated pre-aggregated metrics.
    pub global_hash: String,
    pub model: Option<String>, // None for user messages
    pub stats: Stats,
    pub role: MessageRole,
    /// The actual message content (text). Only populated for recent messages to save memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DailyStats {
    #[allow(dead_code)]
    pub date: String,
    pub user_messages: u32,
    pub ai_messages: u32,
    pub conversations: u32,
    /// Time spent actively working on this day: the sum of gaps under 15 minutes between
    /// consecutive messages. Derived here so the day drill-down does not need the day's raw
    /// messages, which is what forced the whole corpus to stay resident (BZL-14).
    #[serde(default)]
    pub active_seconds: u64,
    pub models: BTreeMap<String, u32>,
    pub stats: Stats,
    /// The day's tokens and cost by hour, model and project, for the hourly view and the
    /// model/project filters (BZL-138). Only combinations that occurred, a few dozen a day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
}

/// One hour's use of one model in one project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Slice {
    /// Local hour, 0-23.
    pub hour: u8,
    pub model: String,
    pub project: String,
    pub replies: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitInfo {
    pub used_percent: f64,
    pub window_minutes: u64,
    pub resets_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    // Token and cost stats
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_creation_tokens: u64,
    /// The portion of `cache_creation_tokens` written with Anthropic's 1-hour TTL, which bills at
    /// 2x base input instead of the 5-minute default's 1.25x (BZL-136). A subset, not an addition:
    /// never add this to `cache_creation_tokens`.
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    pub cache_read_tokens: u64,
    pub cached_tokens: u64,
    pub cost: f64,
    pub tool_calls: u32,

    // File operation stats
    pub terminal_commands: u64,
    pub file_searches: u64,
    pub file_content_searches: u64,
    pub files_read: u64,
    pub files_added: u64,
    pub files_edited: u64,
    pub files_deleted: u64,
    pub lines_read: u64,
    pub lines_added: u64,
    pub lines_edited: u64,
    pub lines_deleted: u64,
    pub bytes_read: u64,
    pub bytes_added: u64,
    pub bytes_edited: u64,
    pub bytes_deleted: u64,

    // Todo stats
    pub todos_created: u64,
    pub todos_completed: u64,
    pub todos_in_progress: u64,
    pub todo_writes: u64,
    pub todo_reads: u64,

    // Composition stats
    pub code_lines: u64,
    pub docs_lines: u64,
    pub data_lines: u64,
    pub media_lines: u64,
    pub config_lines: u64,
    pub other_lines: u64,

    // Rate limits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<std::collections::HashMap<String, RateLimitInfo>>,

    // Local inference (Ollama): energy used and what that electricity cost. `cost` stays the
    // street price, like every other tool, so these are kept beside it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub energy_wh: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub energy_cost: f64,
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

#[derive(Debug, Clone, Copy)]
pub enum FileCategory {
    SourceCode,
    Data,
    Documentation,
    Media,
    Config,
    Other,
}

impl FileCategory {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "java" | "cpp" | "c" | "h" | "hpp"
            | "cs" | "go" | "php" | "rb" | "swift" | "kt" | "scala" | "clj" | "hs" | "ml"
            | "fs" | "elm" | "dart" | "lua" | "r" | "jl" | "nim" | "zig" | "v" | "odin" => {
                FileCategory::SourceCode
            }
            "json" | "xml" | "yaml" | "yml" | "toml" | "ini" | "csv" | "tsv" | "sql" | "db"
            | "sqlite" | "sqlite3" => FileCategory::Data,
            "md" | "txt" | "rst" | "adoc" | "tex" | "rtf" | "doc" | "docx" | "pdf" | "html"
            | "htm" => FileCategory::Documentation,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "ico" | "webp" | "tiff" | "mp3"
            | "wav" | "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => FileCategory::Media,
            "config" | "conf" | "cfg" | "env" | "properties" | "plist" | "reg" | "desktop"
            | "service" => FileCategory::Config,
            _ => FileCategory::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticCodingToolStats {
    pub daily_stats: BTreeMap<String, DailyStats>,
    pub num_conversations: u64,
    pub messages: Vec<ConversationMessage>,
    pub analyzer_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiAnalyzerStats {
    /// Shared, not cloned: a reload replaces one analyzer's entry and every consumer (watch
    /// channel, TUI, notifier, uploader) holds the same allocation. Cloning this struct is a
    /// handful of refcount bumps rather than a copy of ~100K messages.
    pub analyzer_stats: Vec<std::sync::Arc<AgenticCodingToolStats>>,
}

#[derive(Debug, Deserialize)]
pub struct UploadResponse {
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

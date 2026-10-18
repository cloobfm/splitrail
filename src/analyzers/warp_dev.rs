//! Warp's AI agent, read from the Warp app's own database:
//! `~/Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support/
//! dev.warp.Warp-Stable/warp.sqlite`.
//!
//! * `agent_conversations`: one row per conversation, `conversation_data` JSON whose
//!   `conversation_usage_metadata` holds the credits spent, tokens per model
//!   (`token_usage[].warp_tokens`, with BYOK and custom-endpoint tokens beside them), and tool
//!   counts (commands run, files read, diffs applied with lines added and removed).
//!   `last_modified_at` is UTC.
//! * `ai_queries`: one row per prompt, `input` a JSON list whose `Query.text` is the prompt, with
//!   `start_ts` (UTC) and `working_directory`.
//!
//! Warp records totals per conversation, not per reply, so each conversation becomes one assistant
//! message per model, dated when the conversation last changed. Since October 2026 Warp also
//! records `total_charged_usage` (input, output, cache read and cache write tokens for the whole
//! conversation), split here across the models by their share of tokens; older conversations have
//! only a total per model, counted as input. Cost is the credits spent at what a credit costs
//! on the plan (`warp_limits::cents_per_credit`), split across models by tokens.
//!
//! Runs of `warp agent run` (the Oz CLI) are not in that database at all. `warp-run`
//! (`scripts/headless_run.py`) saves each one's `warp run get` record to
//! `~/.config/splitrail/warp/runs/<run id>.json`: credits spent and cost per model, no tokens.
//! Each becomes the prompt plus one assistant message per model, dated when the run last changed,
//! with input and output tokens estimated from what each cost at the model's list price.
//!
//! The database is several hundred MB with a busy WAL (it also holds terminal history), so it is
//! read in place, read-only, rather than copied; Warp's `-wal` and `-shm` already exist beside
//! it, so opening it creates nothing. With no WAL (Warp not running) it is opened immutable.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;

pub const DISPLAY_NAME: &str = "Warp";

fn database_path() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(
        "Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support/dev.warp.Warp-Stable/warp.sqlite",
    ))
}

/// Where `warp-run` saves CLI runs.
fn runs_dir() -> Option<PathBuf> {
    Some(std::env::home_dir()?.join(".config/splitrail/warp/runs"))
}

fn rfc3339(value: Option<&Value>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Sum of every number under `value` whose key ends in `_cents` (a model's input, cache, output
/// and web costs).
fn cents(value: &Value) -> f64 {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, v)| match v {
                Value::Number(n) if key.ends_with("_cents") => n.as_f64().unwrap_or(0.0),
                _ => cents(v),
            })
            .sum(),
        _ => 0.0,
    }
}

/// Messages for one saved `warp run get` record. `cents_per_credit` prices the credits, as for
/// interactive conversations; the models split it by what each cost.
pub fn run_messages(run: &Value, cents_per_credit: f64) -> Vec<ConversationMessage> {
    let Some(run_id) = run.get("run_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let usage = run.get("request_usage");
    let credits = usage
        .and_then(|u| u.get("inference_cost"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let started = rfc3339(run.get("started_at")).or_else(|| rfc3339(run.get("created_at")));
    let Some(started) = started else {
        return Vec::new();
    };
    // A run still going (`warp-run` saves it every 30 s) is dated when it was last saved, so the
    // column shows it active; a finished one when it finished.
    let finished = run
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|s| ["SUCCEEDED", "FAILED", "ERROR", "CANCELLED", "BLOCKED"].contains(&s));
    let saved = run
        .get("recorded_at")
        .and_then(Value::as_f64)
        .and_then(|t| DateTime::from_timestamp(t as i64, 0));
    let ended = rfc3339(run.get("finished_at"))
        .or_else(|| {
            if finished {
                rfc3339(run.get("updated_at"))
            } else {
                saved.or_else(|| rfc3339(run.get("updated_at")))
            }
        })
        .unwrap_or(started)
        .max(started);
    let conversation = run
        .get("conversation_id")
        .and_then(Value::as_str)
        .unwrap_or(run_id);
    let project = project_name(run.get("cwd").and_then(Value::as_str));
    // (model, cents in total, cents for input, cents for output)
    let mut by_model: Vec<(String, f64, f64, f64)> = Vec::new();
    for category in usage
        .and_then(|u| u.get("usage_by_category"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|c| c.values())
    {
        for (model, spend) in category
            .get("direct_api_inference_usage")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let model = crate::analyzers::antigravity::normalize_model(model);
            let part = |key: &str| {
                spend
                    .get("cost_in_cents")
                    .and_then(|c| c.get(key))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            let (input, output) = (part("input_cost_in_cents"), part("output_cost_in_cents"));
            match by_model.iter_mut().find(|(m, ..)| *m == model) {
                Some((_, total, i, o)) => {
                    *total += cents(spend);
                    *i += input;
                    *o += output;
                }
                None => by_model.push((model, cents(spend), input, output)),
            }
        }
    }
    by_model.retain(|(model, ..)| !model.is_empty());
    if by_model.is_empty() {
        by_model.push(("warp".to_string(), 0.0, 0.0, 0.0));
    }
    let total: f64 = by_model.iter().map(|(_, c, ..)| c).sum();
    let cost = credits * cents_per_credit / 100.0;
    let mut messages = vec![ConversationMessage {
        application: Application::Warp,
        date: started,
        project_hash: project.clone(),
        conversation_hash: hash_text(conversation),
        local_hash: None,
        global_hash: hash_text(&format!("warp:run:{run_id}")),
        model: None,
        stats: Stats::default(),
        role: MessageRole::User,
        content: run
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::to_string),
    }];
    for (i, (model, spend, input_cents, output_cents)) in by_model.iter().enumerate() {
        let share = if total > 0.0 {
            spend / total
        } else {
            1.0 / by_model.len() as f64
        };
        messages.push(ConversationMessage {
            application: Application::Warp,
            date: ended,
            project_hash: project.clone(),
            conversation_hash: hash_text(conversation),
            local_hash: None,
            global_hash: hash_text(&format!("warp:run:{run_id}:{model}")),
            model: Some(model.clone()),
            stats: Stats {
                cost: cost * share,
                input_tokens: tokens_for(
                    *input_cents,
                    crate::models::calculate_input_cost(model, 1_000_000),
                ),
                output_tokens: tokens_for(
                    *output_cents,
                    crate::models::calculate_output_cost(model, 1_000_000),
                ),
                ..Default::default()
            },
            role: MessageRole::Assistant,
            content: if i == 0 {
                run.get("title").and_then(Value::as_str).map(str::to_string)
            } else {
                None
            },
        });
    }
    messages
}

/// Tokens that cost `cents` at `dollars_per_million`, or 0 for a model with no known price.
fn tokens_for(cents: f64, dollars_per_million: f64) -> u64 {
    if dollars_per_million <= 0.0 || cents <= 0.0 {
        return 0;
    }
    (cents / 100.0 / dollars_per_million * 1_000_000.0).round() as u64
}

fn read_runs(dir: &Path, cents_per_credit: f64) -> Vec<ConversationMessage> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| serde_json::from_slice::<Value>(&std::fs::read(p).ok()?).ok())
        .flat_map(|run| run_messages(&run, cents_per_credit))
        .collect()
}

fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

/// Open Warp's database without writing anything beside it.
fn open(db: &Path) -> Result<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let mode = if wal_path(db).is_file() {
        "ro"
    } else {
        "ro&immutable=1"
    };
    let uri = format!(
        "file:{}?mode={mode}",
        db.to_string_lossy().replace(' ', "%20")
    );
    let conn = rusqlite::Connection::open_with_flags(uri, flags).context("open Warp database")?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    Ok(conn)
}

/// The account Warp is signed in to, if any.
pub fn signed_in_email() -> Option<String> {
    let conn = open(&database_path()?).ok()?;
    conn.query_row(
        "SELECT email FROM current_user_information LIMIT 1",
        [],
        |r| r.get(0),
    )
    .ok()
}

fn utc(text: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .map(|t| t.and_utc())
}

/// The folder name of a working directory, as the other columns show projects.
fn project_name(dir: Option<&str>) -> String {
    dir.filter(|d| !d.is_empty())
        .map(|d| {
            Path::new(d)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(d)
                .to_string()
        })
        .unwrap_or_default()
}

/// One prompt from `ai_queries`.
pub struct Query {
    pub conversation_id: String,
    pub exchange_id: String,
    pub at: DateTime<Utc>,
    pub input: String,
    pub working_directory: Option<String>,
}

/// One row of `agent_conversations`.
pub struct Conversation {
    pub conversation_id: String,
    pub last_modified_at: DateTime<Utc>,
    pub data: String,
    pub summary: Option<String>,
}

/// The prompt text from an `ai_queries.input` list.
fn prompt_text(input: &str) -> Option<String> {
    let value: Value = serde_json::from_str(input).ok()?;
    value
        .as_array()?
        .iter()
        .find_map(|item| item.get("Query")?.get("text")?.as_str())
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
}

fn count(stats: &Value, key: &str, field: &str) -> u64 {
    stats
        .get(key)
        .and_then(|s| s.get(field))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// Messages for Warp's conversations and prompts. `cents_per_credit` prices the credits.
pub fn messages_from(
    conversations: &[Conversation],
    queries: &[Query],
    cents_per_credit: f64,
) -> Vec<ConversationMessage> {
    let mut messages = Vec::new();
    let mut last_dir: HashMap<&str, &str> = HashMap::new();
    for query in queries {
        if let Some(dir) = query.working_directory.as_deref().filter(|d| !d.is_empty()) {
            last_dir.insert(&query.conversation_id, dir);
        }
        messages.push(ConversationMessage {
            application: Application::Warp,
            date: query.at,
            project_hash: project_name(query.working_directory.as_deref()),
            conversation_hash: hash_text(&query.conversation_id),
            local_hash: None,
            global_hash: hash_text(&format!("warp:query:{}", query.exchange_id)),
            model: None,
            stats: Stats::default(),
            role: MessageRole::User,
            content: prompt_text(&query.input),
        });
    }
    for conversation in conversations {
        let Ok(data) = serde_json::from_str::<Value>(&conversation.data) else {
            continue;
        };
        let Some(usage) = data.get("conversation_usage_metadata") else {
            continue;
        };
        let credits = usage
            .get("credits_spent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let models: Vec<(String, u64)> = usage
            .get("token_usage")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let model = entry.get("model_id")?.as_str()?;
                let tokens = ["warp_tokens", "byok_tokens", "custom_endpoint_tokens"]
                    .iter()
                    .filter_map(|k| entry.get(*k).and_then(Value::as_u64))
                    .sum();
                Some((
                    crate::analyzers::antigravity::normalize_model(model),
                    tokens,
                ))
            })
            .filter(|(model, _)| !model.is_empty())
            .collect();
        if models.is_empty() && credits <= 0.0 {
            continue;
        }
        let models = if models.is_empty() {
            vec![("warp".to_string(), 0)]
        } else {
            models
        };
        let total_tokens: u64 = models.iter().map(|(_, t)| t).sum();
        let tools = usage
            .get("tool_usage_metadata")
            .cloned()
            .unwrap_or(Value::Null);
        let tool_calls: u64 = tools
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, s)| s.get("count").and_then(Value::as_u64))
            .sum();
        let project = project_name(last_dir.get(conversation.conversation_id.as_str()).copied());
        let cost = credits * cents_per_credit / 100.0;
        // Input, output and cache tokens for the whole conversation, when Warp recorded them.
        let charged = usage
            .get("total_charged_usage")
            .filter(|c| c.get("output_tokens").is_some());
        let charged_tokens = |key: &str| {
            charged
                .and_then(|c| c.get(key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let split = [
            charged_tokens("input_tokens"),
            charged_tokens("output_tokens"),
            charged_tokens("input_cache_read_tokens"),
            charged_tokens("input_cache_write_tokens"),
        ];
        let mut given = [0u64; 4];
        for (i, (model, tokens)) in models.iter().enumerate() {
            let share = if total_tokens > 0 {
                *tokens as f64 / total_tokens as f64
            } else {
                1.0 / models.len() as f64
            };
            let mut stats = Stats {
                input_tokens: *tokens,
                cost: cost * share,
                ..Default::default()
            };
            if charged.is_some() {
                // By share of tokens; the last model takes the rounding, so the totals are exact.
                let last = i + 1 == models.len();
                let part: Vec<u64> = (0..4)
                    .map(|k| {
                        if last {
                            split[k].saturating_sub(given[k])
                        } else {
                            (split[k] as f64 * share).round() as u64
                        }
                    })
                    .collect();
                for k in 0..4 {
                    given[k] += part[k];
                }
                stats.input_tokens = part[0];
                stats.output_tokens = part[1];
                stats.cache_read_tokens = part[2];
                stats.cache_creation_tokens = part[3];
            }
            // Tool activity is per conversation; it goes on the first model's message.
            if i == 0 {
                stats.tool_calls = u32::try_from(tool_calls).unwrap_or(u32::MAX);
                stats.terminal_commands = count(&tools, "run_command_stats", "commands_executed");
                stats.files_read = count(&tools, "read_files_stats", "count");
                stats.file_content_searches = count(&tools, "grep_stats", "count");
                stats.file_searches = count(&tools, "file_glob_stats", "count")
                    + count(&tools, "search_codebase_stats", "count");
                stats.files_edited = count(&tools, "apply_file_diff_stats", "files_changed");
                stats.lines_added = count(&tools, "apply_file_diff_stats", "lines_added");
                stats.lines_deleted = count(&tools, "apply_file_diff_stats", "lines_removed");
            }
            messages.push(ConversationMessage {
                application: Application::Warp,
                date: conversation.last_modified_at,
                project_hash: project.clone(),
                conversation_hash: hash_text(&conversation.conversation_id),
                local_hash: None,
                global_hash: hash_text(&format!("warp:{}:{model}", conversation.conversation_id)),
                model: Some(model.clone()),
                stats,
                role: MessageRole::Assistant,
                content: if i == 0 {
                    conversation.summary.clone()
                } else {
                    None
                },
            });
        }
    }
    messages
}

fn read_database(db: &Path) -> Result<(Vec<Conversation>, Vec<Query>)> {
    let conn = open(db)?;
    let mut stmt = conn.prepare(
        "SELECT conversation_id, last_modified_at, \
         json_object('conversation_usage_metadata', json(json_extract(conversation_data, '$.conversation_usage_metadata'))), \
         summary FROM agent_conversations",
    )?;
    let conversations = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .flatten()
        .filter_map(|(conversation_id, modified, data, summary)| {
            Some(Conversation {
                conversation_id,
                last_modified_at: utc(&modified)?,
                data,
                summary,
            })
        })
        .collect();
    let mut stmt = conn.prepare(
        // `input` also carries the prompt's whole context; only its text is needed.
        "SELECT conversation_id, exchange_id, start_ts, \
         json_array(json_object('Query', json_object('text', json_extract(input, '$[0].Query.text')))), \
         working_directory FROM ai_queries ORDER BY start_ts",
    )?;
    let queries = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .flatten()
        .filter_map(
            |(conversation_id, exchange_id, start, input, working_directory)| {
                Some(Query {
                    conversation_id,
                    exchange_id,
                    at: utc(&start)?,
                    input,
                    working_directory,
                })
            },
        )
        .collect();
    Ok((conversations, queries))
}

/// Changes when the AI tables do. Warp writes terminal history to the same database all the
/// time; this one-row query is what keeps those writes from re-reading every conversation.
fn fingerprint(db: &Path) -> Result<String> {
    Ok(open(db)?.query_row(
        "SELECT (SELECT count(*) || ':' || ifnull(max(last_modified_at), '') FROM agent_conversations) \
         || '|' || (SELECT count(*) || ':' || ifnull(max(id), '') FROM ai_queries)",
        [],
        |r| r.get(0),
    )?)
}

type Stamp = [(u64, Option<SystemTime>); 2];

fn stamp(db: &Path) -> Stamp {
    let of =
        |path: &Path| std::fs::metadata(path).map_or((0, None), |m| (m.len(), m.modified().ok()));
    [of(db), of(&wal_path(db))]
}

pub struct WarpDevAnalyzer {
    database: Option<PathBuf>,
    runs: Option<PathBuf>,
    /// The last read, kept until the database or its WAL changes. Warp writes terminal history
    /// to the same database constantly, so most changes leave the AI tables as they were; the
    /// read is two small queries.
    cache: Mutex<Option<(Stamp, String, Arc<Vec<ConversationMessage>>)>>,
}

impl WarpDevAnalyzer {
    pub fn new() -> Self {
        Self {
            database: database_path(),
            runs: runs_dir(),
            cache: Mutex::new(None),
        }
    }

    fn messages(&self, db: &Path) -> Result<Arc<Vec<ConversationMessage>>> {
        let current = stamp(db);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("Warp cache poisoned"))?;
        if let Some((seen, _, messages)) = cache.as_ref()
            && *seen == current
        {
            return Ok(messages.clone());
        }
        let print = fingerprint(db);
        if let (Ok(print), Some((seen, seen_print, messages))) = (&print, cache.as_mut())
            && print == seen_print
        {
            *seen = current;
            return Ok(messages.clone());
        }
        match read_database(db) {
            Ok((conversations, queries)) => {
                let messages = Arc::new(messages_from(
                    &conversations,
                    &queries,
                    crate::warp_limits::cents_per_credit(),
                ));
                *cache = Some((current, print.unwrap_or_default(), messages.clone()));
                Ok(messages)
            }
            // Busy mid-write: keep the previous read.
            Err(e) => cache.as_ref().map(|(_, _, m)| m.clone()).ok_or(e),
        }
    }
}

impl Default for WarpDevAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Analyzer for WarpDevAnalyzer {
    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        self.database
            .iter()
            .flat_map(|db| [db.clone(), wal_path(db)])
            .chain(self.runs.iter().map(|dir| dir.join("*.json")))
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    /// The database, and its WAL so a write that only reaches the WAL still counts as a change,
    /// and the saved CLI runs.
    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        let runs = self
            .runs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"));
        Ok(self
            .database
            .iter()
            .flat_map(|db| [db.clone(), wal_path(db)])
            .filter(|p| p.is_file())
            .chain(runs)
            .map(|path| DataSource { path })
            .collect())
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let mut messages = match sources
            .iter()
            .map(|s| &s.path)
            .find(|p| Some(*p) == self.database.as_ref())
        {
            Some(db) => self.messages(db)?.as_ref().clone(),
            None => Vec::new(),
        };
        if let Some(dir) = &self.runs
            && sources.iter().any(|s| s.path.starts_with(dir))
        {
            messages.extend(read_runs(dir, crate::warp_limits::cents_per_credit()));
        }
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations = daily_stats.values().map(|s| s.conversations as u64).sum();
        Ok(AgenticCodingToolStats {
            daily_stats,
            num_conversations,
            messages,
            analyzer_name: self.display_name().to_string(),
        })
    }

    fn is_available(&self) -> bool {
        self.database.as_ref().is_some_and(|db| db.is_file())
            || self.runs.as_ref().is_some_and(|d| d.is_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        utc(s).unwrap()
    }

    #[test]
    fn conversations_become_one_message_per_model_priced_by_credits() {
        let data = r#"{"conversation_usage_metadata": {"credits_spent": 100.0,
            "token_usage": [
                {"model_id": "Claude Sonnet 5.5 (high)", "warp_tokens": 750, "byok_tokens": 0, "custom_endpoint_tokens": 0},
                {"model_id": "GPT-5 Nano", "warp_tokens": 250, "byok_tokens": 0, "custom_endpoint_tokens": 0}],
            "tool_usage_metadata": {"run_command_stats": {"count": 3, "commands_executed": 4},
                "read_files_stats": {"count": 2},
                "apply_file_diff_stats": {"count": 1, "lines_added": 61, "lines_removed": 10, "files_changed": 6}}}}"#;
        let conversations = [Conversation {
            conversation_id: "c1".into(),
            last_modified_at: at("2026-10-03 07:06:41"),
            data: data.into(),
            summary: Some("Set up devbox-b".into()),
        }];
        let queries = [Query {
            conversation_id: "c1".into(),
            exchange_id: "e1".into(),
            at: at("2026-10-03 07:00:00.5"),
            input: r#"[{"Query":{"text":"make a devbox-b","context":[]}}]"#.into(),
            working_directory: Some("/Users/x/nas-devbox".into()),
        }];
        let messages = messages_from(&conversations, &queries, 1.8);
        assert_eq!(messages.len(), 3);

        let prompt = &messages[0];
        assert_eq!(prompt.role, MessageRole::User);
        assert_eq!(prompt.content.as_deref(), Some("make a devbox-b"));
        assert_eq!(prompt.project_hash, "nas-devbox");

        let (sonnet, nano) = (&messages[1], &messages[2]);
        assert_eq!(sonnet.model.as_deref(), Some("claude-sonnet-5.5"));
        assert_eq!(nano.model.as_deref(), Some("gpt-5-nano"));
        // 100 credits at 1.8 cents is $1.80, split 3:1 by tokens.
        assert!((sonnet.stats.cost - 1.35).abs() < 1e-9 && (nano.stats.cost - 0.45).abs() < 1e-9);
        assert_eq!(sonnet.stats.input_tokens, 750);
        assert_eq!(
            (sonnet.stats.tool_calls, sonnet.stats.terminal_commands),
            (6, 4)
        );
        assert_eq!(
            (sonnet.stats.lines_added, sonnet.stats.lines_deleted),
            (61, 10)
        );
        assert_eq!(nano.stats.tool_calls, 0, "tool activity counted once");
        assert_eq!(sonnet.project_hash, "nas-devbox");
        assert_eq!(sonnet.content.as_deref(), Some("Set up devbox-b"));
        assert_eq!(sonnet.conversation_hash, prompt.conversation_hash);
        assert_eq!(
            sonnet.stats.output_tokens, 0,
            "no split recorded: all input"
        );
    }

    #[test]
    fn recorded_token_split_is_shared_across_models() {
        let data = r#"{"conversation_usage_metadata": {"credits_spent": 10.0,
            "total_charged_usage": {"input_tokens": 198, "output_tokens": 9063,
                "input_cache_read_tokens": 939077, "input_cache_write_tokens": 174330},
            "token_usage": [
                {"model_id": "GPT-6.1 Sol (medium reasoning)", "warp_tokens": 828086},
                {"model_id": "GPT-6 Luna (low reasoning)", "warp_tokens": 34807},
                {"model_id": "Claude Haiku 4.5", "warp_tokens": 259775}]}}"#;
        let conversations = [Conversation {
            conversation_id: "c2".into(),
            last_modified_at: at("2026-10-17 01:58:19"),
            data: data.into(),
            summary: None,
        }];
        let messages = messages_from(&conversations, &[], 1.8);
        assert_eq!(messages.len(), 3);
        let sum = |f: fn(&Stats) -> u64| messages.iter().map(|m| f(&m.stats)).sum::<u64>();
        assert_eq!(sum(|s| s.output_tokens), 9063);
        assert_eq!(sum(|s| s.input_tokens), 198);
        assert_eq!(sum(|s| s.cache_read_tokens), 939_077);
        assert_eq!(sum(|s| s.cache_creation_tokens), 174_330);
        // Sol had 828,086 of the 1,122,668 tokens.
        assert_eq!(
            messages[0].stats.output_tokens,
            (9063.0 * 828_086.0 / 1_122_668.0_f64).round() as u64
        );
    }

    #[test]
    fn a_saved_cli_run_is_its_prompt_and_its_cost_per_model() {
        let mut run: Value =
            serde_json::from_str(include_str!("../../scripts/testdata/warp_run_get.json")).unwrap();
        run["cwd"] = "/Users/x/Courtside".into();
        let messages = run_messages(&run, 1.8);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[0].project_hash, "Courtside");
        assert_eq!(
            messages[0].content.as_deref(),
            Some("Reply with just the word OK. Do not run any tools.")
        );
        let reply = &messages[1];
        assert_eq!(reply.date, at("2026-10-17 02:04:45.046175"));
        assert!((reply.stats.cost - 4.98725 * 0.018).abs() < 1e-9);
        // Output tokens from the output cost at the model's list price, when it has one.
        assert_eq!(tokens_for(31.2455, 10.0), 31_246);
        assert_eq!(tokens_for(1.0, 0.0), 0);
        // Two models: the cost splits by what each cost.
        run["request_usage"]["usage_by_category"]["tool_summarization"] = serde_json::json!({
            "direct_api_inference_usage": {"claude-haiku-4-5": {"cost_in_cents": {"output_cost_in_cents": 1.0}}}
        });
        let messages = run_messages(&run, 1.8);
        assert_eq!(messages.len(), 3);
        // Still running: dated when warp-run last saved it.
        run["state"] = "IN_PROGRESS".into();
        run["recorded_at"] = 1_792_210_000.0.into();
        let messages = run_messages(&run, 1.8);
        assert_eq!(messages[1].date.timestamp(), 1_792_210_000);
        let total: f64 = messages.iter().map(|m| m.stats.cost).sum();
        assert!((total - 4.98725 * 0.018).abs() < 1e-9);
    }

    #[test]
    fn empty_or_unreadable_conversations_are_skipped() {
        let conversation = |data: &str| Conversation {
            conversation_id: "c".into(),
            last_modified_at: at("2026-10-03 07:06:41"),
            data: data.into(),
            summary: None,
        };
        let empty = r#"{"conversation_usage_metadata": {"credits_spent": 0.0, "token_usage": []}}"#;
        let credits_only =
            r#"{"conversation_usage_metadata": {"credits_spent": 2.0, "token_usage": []}}"#;
        assert!(messages_from(&[conversation(empty), conversation("{")], &[], 1.8).is_empty());
        let messages = messages_from(&[conversation(credits_only)], &[], 1.8);
        assert_eq!(messages.len(), 1);
        assert!((messages[0].stats.cost - 0.036).abs() < 1e-9);
    }

    #[test]
    fn prompt_text_comes_from_the_query_entry() {
        assert_eq!(
            prompt_text(r#"[{"Query":{"text":"hi"}}]"#).as_deref(),
            Some("hi")
        );
        assert_eq!(prompt_text(r#"[{"Other":{}}]"#), None);
        assert_eq!(prompt_text("not json"), None);
    }

    /// Reads the real database: `cargo test warp_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn warp_live_database_has_data() {
        let started = std::time::Instant::now();
        let (conversations, queries) = read_database(&database_path().unwrap()).unwrap();
        println!(
            "read {} conversations, {} prompts in {:?}",
            conversations.len(),
            queries.len(),
            started.elapsed()
        );
        let started = std::time::Instant::now();
        let _ = fingerprint(&database_path().unwrap()).unwrap();
        println!("fingerprint in {:?}", started.elapsed());
        let stats = WarpDevAnalyzer::new().get_stats().await.unwrap();
        let cost: f64 = stats.daily_stats.values().map(|d| d.stats.cost).sum();
        let models: std::collections::BTreeSet<_> = stats
            .messages
            .iter()
            .filter_map(|m| m.model.clone())
            .collect();
        println!(
            "messages {} conversations {} cost ${cost:.2} models {models:?}",
            stats.messages.len(),
            stats.num_conversations
        );
        crate::price_feed::init(Default::default(), true); // As the dashboard does at start.
        let runs = read_runs(&runs_dir().unwrap(), crate::warp_limits::cents_per_credit());
        println!(
            "CLI runs: {} messages ${:.2}",
            runs.len(),
            runs.iter().map(|m| m.stats.cost).sum::<f64>()
        );
        for m in runs.iter().filter(|m| m.role == MessageRole::Assistant) {
            println!(
                "  {:?} in {} out {}",
                m.model, m.stats.input_tokens, m.stats.output_tokens
            );
        }
        let output: u64 = stats.messages.iter().map(|m| m.stats.output_tokens).sum();
        println!("output tokens, all Warp: {output}");
        println!("signed in: {}", signed_in_email().is_some());
        assert!(!stats.messages.is_empty());
    }
}

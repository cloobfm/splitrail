use crate::analyzer::{Analyzer, DataSource};
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, Stats};
use crate::utils::hash_text;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use serde::Deserialize;
use std::path::Path;

pub struct OllamaAnalyzer {
    usage_file: Option<std::path::PathBuf>,
}

impl OllamaAnalyzer {
    pub fn new() -> Self {
        Self {
            usage_file: std::env::home_dir().map(|home| {
                home.join(".config")
                    .join("splitrail")
                    .join("ollama")
                    .join("usage.jsonl")
            }),
        }
    }
}

impl Default for OllamaAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Clone)]
struct UsageEvent {
    ts: f64,
    model: String,
    endpoint: String,
    prompt_tokens: u64,
    cached_tokens: u64,
    completion_tokens: u64,
    duration_ms: u64,
    client: String,
}

/// Normalise model name from the proxy event to the price feed slug.
/// If the model contains a namespace (e.g. `ollama/qwen3-coder:30b`), keep it.
fn normalize_model(name: &str) -> String {
    name.trim().to_string()
}

/// Parse the JSONL written by `ollama_usage_proxy.py` into messages.
/// Each line = one assistant message (local inference, no conversation grouping).
/// Malformed lines are skipped silently.
pub fn parse_usage_log(content: &str, source: &Path) -> Vec<ConversationMessage> {
    let source_str = source.to_string_lossy();
    content
        .lines()
        .enumerate()
        .filter_map(|(line_no, line)| {
            let event: UsageEvent = serde_json::from_str(line.trim()).ok()?;
            let date = DateTime::<Utc>::from_timestamp_millis((event.ts * 1000.0) as i64)
                .unwrap_or_else(Utc::now);
            let model = normalize_model(&event.model);
            // Local inference: cost is $0 directly (free).
            let cost = 0.0;
            // Input tokens = prompt - cached (cached are read, not billed as fresh input).
            let input_tokens = event.prompt_tokens.saturating_sub(event.cached_tokens);
            let output_tokens = event.completion_tokens;
            Some(ConversationMessage {
                application: Application::Ollama,
                date,
                project_hash: hash_text(&std::env::var("PWD").unwrap_or_else(|_| ".".to_string())),
                conversation_hash: hash_text(&format!("{}:{}:{}", source_str, line_no, event.ts)),
                local_hash: None,
                global_hash: hash_text(&format!("ollama_{}_{}", source_str, event.ts)),
                model: Some(model.clone()),
                stats: Stats {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: event.cached_tokens,
                    cached_tokens: event.cached_tokens,
                    cost,
                    ..Default::default()
                },
                role: MessageRole::Assistant,
                content: None,
            })
        })
        .collect()
}

#[async_trait]
impl Analyzer for OllamaAnalyzer {
    fn display_name(&self) -> &'static str {
        "Ollama"
    }

    fn get_data_glob_patterns(&self) -> Vec<String> {
        self.usage_file
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
        Ok(self
            .usage_file
            .iter()
            .filter(|path| path.is_file())
            .map(|path| DataSource { path: path.clone() })
            .collect())
    }

    async fn parse_conversations(
        &self,
        sources: Vec<DataSource>,
    ) -> Result<Vec<ConversationMessage>> {
        let mut messages = Vec::new();
        for source in sources {
            let content = std::fs::read_to_string(&source.path)?;
            messages.extend(parse_usage_log(&content, &source.path));
        }
        Ok(messages)
    }

    async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
        let sources = self.discover_data_sources()?;
        let messages = self.parse_conversations(sources).await?;
        let daily_stats = crate::utils::aggregate_by_date(&messages);
        let num_conversations = daily_stats.values().map(|s| s.conversations as u64).sum();
        Ok(AgenticCodingToolStats {
            analyzer_name: self.display_name().to_string(),
            daily_stats,
            messages,
            num_conversations,
        })
    }

    fn is_available(&self) -> bool {
        self.discover_data_sources()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_line_is_one_assistant_message() {
        let log = concat!(
            r#"{"ts": 1790880000.5, "model": "qwen3-coder:30b", "endpoint": "/api/generate", "prompt_tokens": 100, "cached_tokens": 20, "completion_tokens": 30, "duration_ms": 500, "client": "127.0.0.1:11434"}"#,
            "\n",
            "bad line\n",
            r#"{"ts": 1790880100, "model": "hermes3:8b", "endpoint": "/api/chat", "prompt_tokens": 50, "cached_tokens": 0, "completion_tokens": 10, "duration_ms": 200, "client": "127.0.0.1:11434"}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/test.jsonl"));
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.application, Application::Ollama);
        assert_eq!(first.role, MessageRole::Assistant);
        assert_eq!(first.model.as_deref(), Some("qwen3-coder:30b"));
        assert_eq!(first.stats.input_tokens, 80); // 100 - 20 cached
        assert_eq!(first.stats.output_tokens, 30);
        assert_eq!(first.stats.cache_read_tokens, 20);
        assert_eq!(first.stats.cost, 0.0);
        assert_ne!(first.conversation_hash, messages[1].conversation_hash);
    }

    #[test]
    fn cost_is_zero_for_local_inference() {
        let log = concat!(
            r#"{"ts": 1790880000, "model": "local-llama", "endpoint": "/api/generate", "prompt_tokens": 1, "cached_tokens": 0, "completion_tokens": 1, "duration_ms": 100, "client": "127.0.0.1:11434"}"#,
            "\n",
        );
        let messages = parse_usage_log(log, Path::new("/test.jsonl"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].stats.cost, 0.0);
    }
}

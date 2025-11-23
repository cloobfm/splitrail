use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tokio::sync::watch;

use crate::config::SlackConfig;
use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, MessageRole, MultiAnalyzerStats};

pub struct NotificationManager {
    settings: NotificationSettings,
    slack: Option<SlackNotifier>,
    notified_messages: HashSet<String>,
}

#[derive(Clone)]
struct NotificationSettings {
    waiting_seconds: i64,
    stale_seconds: i64,
    sample_messages: usize,
}

impl NotificationManager {
    pub fn from_config(config: &crate::config::Config) -> Option<Self> {
        if !config.notifications.enabled {
            return None;
        }

        let slack = if config.notifications.slack.enabled {
            SlackNotifier::from_config(&config.notifications.slack)
        } else {
            None
        };

        Some(Self {
            settings: NotificationSettings {
                waiting_seconds: config.notifications.waiting_seconds as i64,
                stale_seconds: (config.notifications.stale_minutes * 60) as i64,
                sample_messages: config.notifications.sample_messages.max(1),
            },
            slack,
            notified_messages: HashSet::new(),
        })
    }

    pub async fn run(mut self, mut stats_rx: watch::Receiver<MultiAnalyzerStats>) -> Result<()> {
        let mut latest_stats = stats_rx.borrow().clone();
        // Check every 15 seconds for stalled sessions
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        // Skip first tick to avoid startup notifications
        ticker.tick().await;

        loop {
            tokio::select! {
                changed = stats_rx.changed() => {
                    if changed.is_ok() {
                        latest_stats = stats_rx.borrow().clone();
                    } else {
                        break; // sender dropped
                    }
                }
                _ = ticker.tick() => {
                    self.check_and_notify(&latest_stats).await?;
                }
            }
        }

        Ok(())
    }

    async fn check_and_notify(&mut self, stats: &MultiAnalyzerStats) -> Result<()> {
        let now = Utc::now();

        // Avoid unbounded growth if running for a long time
        if self.notified_messages.len() > 10_000 {
            self.notified_messages.clear();
        }

        for analyzer_stats in &stats.analyzer_stats {
            // Only alert on the freshest conversation that just crossed the threshold,
            // and ignore ones that have been idle far beyond it (to avoid noisy catch-up).
            const RECENT_WINDOW_SECS: i64 = 15 * 60; // 15 minutes after threshold
            let mut waiting: Vec<_> =
                find_waiting_conversations(analyzer_stats, &self.settings, now)
                    .into_iter()
                    .filter(|alert| {
                        let over_threshold = alert
                            .idle_seconds
                            .saturating_sub(self.settings.waiting_seconds);
                        over_threshold <= RECENT_WINDOW_SECS
                    })
                    .collect();

            if let Some(alert) = waiting
                .iter()
                .min_by_key(|a| a.idle_seconds)
                .cloned()
            {
                // Skip if we've already notified on this exact assistant message
                if self
                    .notified_messages
                    .insert(alert.last_global_hash.clone())
                {
                    let text = build_notification_text(&alert);

                    // Only send one notification per analyzer per tick
                    if let Some(slack) = &self.slack {
                        if let Err(e) = slack.send(&text).await {
                            eprintln!("⚠️ Failed to deliver Slack notification: {e:#}");
                        } else {
                            mark_notification_sent();
                        }
                    } else {
                        // Surface locally so the user still sees the alert
                        eprintln!("{text}");
                        mark_notification_sent();
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
struct WaitingAlert {
    analyzer_name: String,
    application: Application,
    last_global_hash: String,
    conversation_hash: String,
    project_hash: String,
    model: Option<String>,
    idle_seconds: i64,
    last_message_time: DateTime<Utc>,
    recent_messages: Vec<ConversationMessage>,
}

fn find_waiting_conversations(
    stats: &AgenticCodingToolStats,
    settings: &NotificationSettings,
    now: DateTime<Utc>,
) -> Vec<WaitingAlert> {
    let mut conversations: HashMap<String, Vec<&ConversationMessage>> = HashMap::new();

    for msg in &stats.messages {
        conversations
            .entry(msg.conversation_hash.clone())
            .or_default()
            .push(msg);
    }

    let mut alerts = Vec::new();

    for messages in conversations.values_mut() {
        messages.sort_by_key(|m| m.date);

        let Some(last) = messages.last() else {
            continue;
        };

        // Only alert if the assistant spoke last and we've been idle long enough
        if !matches!(last.role, MessageRole::Assistant) {
            continue;
        }

        let idle = now
            .signed_duration_since(last.date)
            .num_seconds()
            .max(0);
        if idle < settings.waiting_seconds || idle > settings.stale_seconds {
            continue;
        }

        // Require that the prior human message was reasonably recent to reduce false positives
        let human_before = messages
            .iter()
            .rev()
            .skip(1)
            .find(|m| matches!(m.role, MessageRole::User));

        if let Some(user_msg) = human_before {
            let gap = last.date.signed_duration_since(user_msg.date).num_minutes();
            if gap > 60 {
                // Too much time between human and assistant, likely stale session
                continue;
            }
        } else {
            // No human message at all, skip
            continue;
        }

        // Build recent message preview (oldest -> newest)
        let mut recent: Vec<ConversationMessage> = messages
            .iter()
            .rev()
            .take(settings.sample_messages)
            .map(|m| (*m).clone())
            .collect();
        recent.reverse();

        alerts.push(WaitingAlert {
            analyzer_name: stats.analyzer_name.clone(),
            application: last.application.clone(),
            last_global_hash: last.global_hash.clone(),
            conversation_hash: last.conversation_hash.clone(),
            project_hash: last.project_hash.clone(),
            model: last.model.clone(),
            idle_seconds: idle,
            last_message_time: last.date,
            recent_messages: recent,
        });
    }

    alerts
}

fn build_notification_text(alert: &WaitingAlert) -> String {
    let model = alert.model.as_deref().unwrap_or("unknown");
    let app = format_application(&alert.application);
    
    // Ultra-compact header (no "ago" since notification timing implies recency)
    let header = format!(
        "🟡 {} {} | {} | {}",
        app,
        format_duration(alert.idle_seconds),
        abbreviate(&alert.project_hash, 6),
        model
    );

    let mut lines = vec![header];

    // Get last 2 messages to show interaction pace and who spoke last
    let mut messages = Vec::new();
    for msg in alert.recent_messages.iter().rev().take(2) {
        if let Some(content_raw) = msg.content.as_deref() {
            let cleaned = clean_message(content_raw);
            if !cleaned.is_empty() {
                let (emoji, max_len) = match msg.role {
                    MessageRole::User => ("👤", 50),
                    MessageRole::Assistant => ("💬", 70),
                };
                let trimmed = truncate_content(&cleaned, max_len);
                let timestamp = format_timestamp(msg.date);
                messages.push(format!("[{}] {} {}", timestamp, emoji, trimmed));
            }
        }
    }

    // If we have less than 3 messages, add more to reach minimum 3 total
    if messages.len() < 3 {
        for msg in alert.recent_messages.iter().rev().skip(2).take(3 - messages.len()) {
            if let Some(content_raw) = msg.content.as_deref() {
                let cleaned = clean_message(content_raw);
                if !cleaned.is_empty() {
                    let (emoji, max_len) = match msg.role {
                        MessageRole::User => ("👤", 40),
                        MessageRole::Assistant => ("💬", 60),
                    };
                    let trimmed = truncate_content(&cleaned, max_len);
                    let timestamp = format_timestamp(msg.date);
                    messages.push(format!("[{}] {} {}", timestamp, emoji, trimmed));
                }
            }
        }
    }

    lines.extend(messages);
    lines.join("\n")
}

fn clean_message(msg: &str) -> String {
    msg.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_content(msg: &str, max_len: usize) -> String {
    if msg.len() <= max_len {
        msg.to_string()
    } else {
        format!("{}…", msg.chars().take(max_len).collect::<String>())
    }
}

fn abbreviate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

fn format_timestamp(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local).format("%H:%M").to_string()
}

fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{}h{}m", hours, minutes)
        }
    }
}

fn format_application(app: &Application) -> &'static str {
    match app {
        Application::ClaudeCode => "Claude",
        Application::GeminiCli => "Gemini",
        Application::QwenCode => "Qwen",
        Application::CodexCli => "Codex",
        Application::Cline => "Cline",
        Application::RooCode => "Roo",
        Application::KiloCode => "Kilo",
        Application::Copilot => "Copilot",
        Application::AmazonQ => "Amazon Q",
        Application::KiroCli => "Kiro",
        Application::Warp => "Warp",
        Application::OpenCode => "OpenCode",
    }
}

struct SlackNotifier {
    client: reqwest::Client,
    webhook_url: String,
    channel: Option<String>,
    username: Option<String>,
}

static SLACK_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static LAST_SENT: OnceLock<std::sync::Mutex<Option<DateTime<Utc>>>> = OnceLock::new();

impl SlackNotifier {
    fn from_config(config: &SlackConfig) -> Option<Self> {
        let webhook_url = config.webhook_url.clone()?;

        let client = SLACK_CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .danger_accept_invalid_certs(true)
                .build()
                .expect("Failed to build Slack HTTP client")
        });

        Some(Self {
            client: client.clone(),
            webhook_url,
            channel: config.channel.clone(),
            username: config.username.clone(),
        })
    }

    async fn send(&self, text: &str) -> Result<()> {
        #[derive(Serialize)]
        struct SlackPayload<'a> {
            text: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            channel: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            username: Option<&'a str>,
        }

        let payload = SlackPayload {
            text,
            channel: self.channel.as_deref(),
            username: self.username.as_deref(),
        };

        let body = serde_json::to_string(&payload)?;

        let response = self
            .client
            .post(&self.webhook_url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .context("Slack webhook request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Slack webhook error ({status}): {body}");
        }

        Ok(())
    }
}

/// Record that a notification was sent (used by the TUI to show a short-lived checkmark)
pub fn mark_notification_sent() {
    let guard = LAST_SENT.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut last) = guard.lock() {
        *last = Some(Utc::now());
    }
}

/// How many seconds ago the last notification was sent (if within the supplied window)
pub fn last_notification_age_seconds(max_age_secs: i64) -> Option<i64> {
    let guard = LAST_SENT.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(last) = guard.lock() {
        if let Some(ts) = *last {
            let age = Utc::now()
                .signed_duration_since(ts)
                .num_seconds()
                .max(0);
            if age <= max_age_secs {
                return Some(age);
            }
        }
    }
    None
}

/// Calculate the soonest time (in seconds) until a waiting-notification would fire.
/// Returns None if no conversations are in a "waiting" pre-notification state.
pub fn next_notification_eta_seconds(
    stats: &[&AgenticCodingToolStats],
    waiting_threshold_secs: i64,
    stale_secs: i64,
) -> Option<i64> {
    let now = Utc::now();
    let mut best: Option<i64> = None;

    for analyzer in stats {
        // Group by conversation hash, take last message
        let mut by_conv: HashMap<&str, &ConversationMessage> = HashMap::new();
        for msg in &analyzer.messages {
            by_conv
                .entry(&msg.conversation_hash)
                .and_modify(|existing| {
                    if msg.date > existing.date {
                        *existing = msg;
                    }
                })
                .or_insert(msg);
        }

        for last in by_conv.values() {
            if !matches!(last.role, MessageRole::Assistant) {
                continue;
            }
            let idle = now.signed_duration_since(last.date).num_seconds().max(0);

            if idle >= waiting_threshold_secs || idle > stale_secs {
                continue;
            }

            let remaining = waiting_threshold_secs.saturating_sub(idle);
            best = match best {
                Some(current) => Some(current.min(remaining)),
                None => Some(remaining),
            };
        }
    }

    best
}

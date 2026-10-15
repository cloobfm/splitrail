//! Live stats on an MQTT broker, for Home Assistant and anything else subscribed to it.
//!
//! * `splitrail/<host>/state`: retained JSON with output tokens per second (the dashboard's
//!   15-minute average), today's cost and output tokens, overall and per tool, and each tool's
//!   plan limits (used %, `null` when unknown).
//! * `splitrail/<host>/status`: `online` / `offline`, retained; `offline` is also the broker's
//!   last will, so a machine that sleeps or drops off shows as unavailable instead of frozen.
//! * `homeassistant/sensor/splitrail_<host>_<key>/config`: retained discovery configs, one
//!   per sensor, all under one device per machine. Tools get sensors once they have recent use
//!   or limits; every tool is always in the state JSON so a template never misses its key.
//!
//! Publishing never blocks the dashboard: while the broker is unreachable messages are dropped,
//! and on reconnect discovery and status are sent again.

use crate::config::MqttConfig;
use crate::plan_limits::{self, AccountLimits};
use crate::types::{MessageRole, MultiAnalyzerStats};
use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::{Value, json};
use crate::rollup;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::watch;

const DEFAULT_PORT: u16 = 1883;
const TOPIC_ROOT: &str = "splitrail";
const DISCOVERY_ROOT: &str = "homeassistant";
const PUBLISH_INTERVAL: Duration = Duration::from_secs(15);
const ROLLUP_REFRESH: Duration = Duration::from_secs(300);
const ROLLUP_MIN_GAP: Duration = Duration::from_secs(60);
/// The window the dashboard's tk/s averages over.
const RATE_WINDOW_SECS: i64 = 15 * 60;
/// Tools used within this long get their own sensors.
const ACTIVE_DAYS: i64 = 30;

static CLIENT: OnceLock<(AsyncClient, String)> = OnceLock::new();
static LINK: AtomicU8 = AtomicU8::new(Link::Off as u8);

/// The broker connection, for the dashboard's indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Link {
    /// No broker configured.
    Off = 0,
    Connecting = 1,
    Connected = 2,
    /// Configured but the last attempt failed (unreachable, or login refused).
    Failing = 3,
}

pub fn link() -> Link {
    match LINK.load(Ordering::Relaxed) {
        1 => Link::Connecting,
        2 => Link::Connected,
        3 => Link::Failing,
        _ => Link::Off,
    }
}

fn set_link(link: Link) {
    LINK.store(link as u8, Ordering::Relaxed);
}

/// One tool's numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolState {
    pub name: String,
    pub slug: String,
    pub tks: f64,
    pub cost_today: f64,
    pub output_tokens_today: u64,
    pub limits: Option<AccountLimits>,
    /// Used within `ACTIVE_DAYS`, or has limits: worth sensors of its own.
    pub active: bool,
}

/// Lowercase ASCII letters and digits joined by single underscores: `Claude Code (claude-v)`
/// becomes `claude_code_claude_v`.
pub fn slug(name: &str) -> String {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("_")
}

pub fn tool_states(
    stats: &MultiAnalyzerStats,
    now: DateTime<Utc>,
    limits_for: impl Fn(&str) -> Option<AccountLimits>,
) -> Vec<ToolState> {
    let rate_start = now - ChronoDuration::seconds(RATE_WINDOW_SECS);
    let active_start = now - ChronoDuration::days(ACTIVE_DAYS);
    let today = now.with_timezone(&Local).date_naive();
    stats
        .analyzer_stats
        .iter()
        .map(|tool| {
            let mut window_output = 0u64;
            let mut cost_today = 0.0;
            let mut output_tokens_today = 0u64;
            let mut recent = false;
            for message in &tool.messages {
                if message.date > now {
                    continue;
                }
                recent |= message.date >= active_start;
                if message.date.with_timezone(&Local).date_naive() == today {
                    cost_today += message.stats.cost;
                    if message.role == MessageRole::Assistant {
                        output_tokens_today += message.stats.output_tokens;
                    }
                }
                if message.role == MessageRole::Assistant && message.date >= rate_start {
                    window_output += message.stats.output_tokens;
                }
            }
            let limits = limits_for(&tool.analyzer_name);
            ToolState {
                name: tool.analyzer_name.clone(),
                slug: slug(&tool.analyzer_name),
                tks: window_output as f64 / RATE_WINDOW_SECS as f64,
                cost_today,
                output_tokens_today,
                active: recent || limits.is_some(),
                limits,
            }
        })
        .collect()
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

fn window_used(window: Option<plan_limits::WindowStatus>) -> Value {
    window.map_or(Value::Null, |w| json!(round(w.used_percentage, 1)))
}

pub fn state_payload(tools: &[ToolState], now: DateTime<Utc>) -> Value {
    let mut by_tool = serde_json::Map::new();
    for tool in tools {
        let limits = tool.limits;
        by_tool.insert(
            tool.slug.clone(),
            json!({
                "name": tool.name,
                "tks": round(tool.tks, 1),
                "cost_today": round(tool.cost_today, 2),
                "output_tokens_today": tool.output_tokens_today,
                "five_hour": window_used(limits.and_then(|l| l.five_hour)),
                "seven_day": window_used(limits.and_then(|l| l.seven_day)),
                "monthly": window_used(limits.and_then(|l| l.monthly)),
            }),
        );
    }
    json!({
        "tks": round(tools.iter().map(|t| t.tks).sum(), 1),
        "cost_today": round(tools.iter().map(|t| t.cost_today).sum(), 2),
        "output_tokens_today": tools.iter().map(|t| t.output_tokens_today).sum::<u64>(),
        "updated": now.to_rfc3339(),
        "tools": by_tool,
    })
}

/// A Home Assistant sensor: its key (unique within the machine), name, value path in the state
/// JSON, and unit/class/icon.
#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    pub key: String,
    pub name: String,
    pub path: String,
    pub kind: SensorKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SensorKind {
    Rate,
    Cost,
    Tokens,
    LimitUsed,
}

pub fn sensors(tools: &[ToolState]) -> Vec<Sensor> {
    let sensor = |key: String, name: String, path: String, kind| Sensor { key, name, path, kind };
    let mut out = vec![
        sensor("tks".into(), "Output tk/s".into(), "value_json.tks".into(), SensorKind::Rate),
        sensor("cost_today".into(), "Cost today".into(), "value_json.cost_today".into(), SensorKind::Cost),
        sensor(
            "output_tokens_today".into(),
            "Output tokens today".into(),
            "value_json.output_tokens_today".into(),
            SensorKind::Tokens,
        ),
    ];
    for tool in tools.iter().filter(|t| t.active) {
        let base = format!("value_json.tools['{}']", tool.slug);
        out.push(sensor(
            format!("{}_tks", tool.slug),
            format!("{} tk/s", tool.name),
            format!("{base}.tks"),
            SensorKind::Rate,
        ));
        out.push(sensor(
            format!("{}_cost_today", tool.slug),
            format!("{} cost today", tool.name),
            format!("{base}.cost_today"),
            SensorKind::Cost,
        ));
        let Some(limits) = tool.limits else { continue };
        for (window, label, present) in [
            ("five_hour", "5h limit", limits.five_hour.is_some()),
            ("seven_day", "weekly limit", limits.seven_day.is_some()),
            ("monthly", "monthly limit", limits.monthly.is_some()),
        ] {
            if present {
                out.push(sensor(
                    format!("{}_{window}", tool.slug),
                    format!("{} {label}", tool.name),
                    format!("{base}.{window}"),
                    SensorKind::LimitUsed,
                ));
            }
        }
    }
    out
}

pub struct Topics {
    pub host: String,
    pub host_slug: String,
}

impl Topics {
    pub fn new(host: &str) -> Self {
        let host_slug = match slug(host) {
            s if s.is_empty() => "splitrail".to_string(),
            s => s,
        };
        Self { host: host.to_string(), host_slug }
    }
    pub fn state(&self) -> String {
        format!("{TOPIC_ROOT}/{}/state", self.host_slug)
    }
    pub fn status(&self) -> String {
        format!("{TOPIC_ROOT}/{}/status", self.host_slug)
    }
    /// `daily`, `recent` or `limits` (see `rollup`).
    pub fn rollup(&self, kind: &str) -> String {
        format!("{TOPIC_ROOT}/{}/{kind}", self.host_slug)
    }
    pub fn discovery(&self, sensor: &Sensor) -> String {
        format!("{DISCOVERY_ROOT}/sensor/splitrail_{}_{}/config", self.host_slug, sensor.key)
    }

    pub fn discovery_payload(&self, sensor: &Sensor) -> Value {
        let mut config = json!({
            "name": sensor.name,
            "unique_id": format!("splitrail_{}_{}", self.host_slug, sensor.key),
            "object_id": format!("splitrail_{}_{}", self.host_slug, sensor.key),
            "state_topic": self.state(),
            "value_template": format!("{{{{ {} }}}}", sensor.path),
            "availability_topic": self.status(),
            "device": {
                "identifiers": [format!("splitrail_{}", self.host_slug)],
                "name": format!("Splitrail {}", self.host),
                "manufacturer": "Splitrail",
                "model": "splitrail-dashboard",
                "sw_version": env!("CARGO_PKG_VERSION"),
            },
        });
        let extra = match sensor.kind {
            SensorKind::Rate => json!({
                "unit_of_measurement": "tk/s", "state_class": "measurement", "icon": "mdi:speedometer",
            }),
            SensorKind::Cost => json!({
                "unit_of_measurement": "USD", "device_class": "monetary", "state_class": "total",
            }),
            SensorKind::Tokens => json!({
                "unit_of_measurement": "tokens", "state_class": "total_increasing", "icon": "mdi:counter",
            }),
            SensorKind::LimitUsed => json!({
                "unit_of_measurement": "%", "state_class": "measurement", "icon": "mdi:gauge",
            }),
        };
        if let (Some(config), Some(extra)) = (config.as_object_mut(), extra.as_object()) {
            config.extend(extra.clone());
        }
        config
    }
}

/// This machine's id in topics.
pub fn host_slug() -> String {
    Topics::new(&short_hostname()).host_slug
}

fn short_hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "splitrail".to_string())
}

pub fn password(config: &MqttConfig) -> Option<String> {
    let path = match &config.password_file {
        Some(path) => match path.strip_prefix("~/") {
            Some(rest) => std::env::home_dir()?.join(rest),
            None => path.into(),
        },
        None => std::env::home_dir()?.join(".config/splitrail/mqtt_password"),
    };
    let password = std::fs::read_to_string(path).ok()?.trim_end_matches(['\n', '\r']).to_string();
    (!password.is_empty()).then_some(password)
}

/// Publish `stats` to the broker until the stats channel closes. Returns at once when no broker
/// is configured.
pub async fn run(config: MqttConfig, mut stats_rx: watch::Receiver<MultiAnalyzerStats>) {
    let Some(host) = config.host.clone().filter(|h| !h.trim().is_empty()) else {
        return;
    };
    let topics = Topics::new(&short_hostname());
    let mut options = MqttOptions::new(
        format!("splitrail-{}-{}", topics.host_slug, std::process::id()),
        host,
        config.port.unwrap_or(DEFAULT_PORT),
    );
    options.set_keep_alive(Duration::from_secs(30));
    // The roll-up's daily totals run to tens of KB; rumqttc's default cap is 10 KB.
    options.set_max_packet_size(4 * 1024 * 1024, 4 * 1024 * 1024);
    options.set_last_will(LastWill::new(topics.status(), "offline", QoS::AtLeastOnce, true));
    if let Some(username) = &config.username {
        options.set_credentials(username, password(&config).unwrap_or_default());
    }
    let (client, mut eventloop) = AsyncClient::new(options, 256);
    set_link(Link::Connecting);
    let _ = CLIENT.set((client.clone(), topics.status()));

    // Set on every (re)connect: the broker may have lost retained messages, and the session
    // started without them.
    static CONNECTED_AFRESH: AtomicBool = AtomicBool::new(false);
    tokio::spawn(async move {
        let mut failing = false;
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    failing = false;
                    set_link(Link::Connected);
                    CONNECTED_AFRESH.store(true, Ordering::SeqCst);
                }
                Ok(_) => {}
                Err(e) => {
                    set_link(Link::Failing);
                    if !failing {
                        crate::utils::log_error(format!("MQTT: {e}"));
                        failing = true;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    });

    let mut discovered: HashSet<String> = HashSet::new();
    // Roll-up payloads are sent only when they change (they are retained, so a dashboard that
    // starts later still gets them), and at least every ROLLUP_REFRESH in case one was lost.
    let mut last_sent: HashMap<&str, (String, std::time::Instant)> = HashMap::new();
    let mut ticker = tokio::time::interval(PUBLISH_INTERVAL);
    loop {
        tokio::select! {
            changed = stats_rx.changed() => if changed.is_err() { break },
            _ = ticker.tick() => {}
        }
        if CONNECTED_AFRESH.swap(false, Ordering::SeqCst) {
            discovered.clear();
            last_sent.clear();
        }
        // Every time, not just on connect: another instance on this machine (the dashboard
        // beside `splitrail mqtt`) shares the topic, and its exit marks the machine offline.
        let _ = client.try_publish(topics.status(), QoS::AtLeastOnce, true, "online");
        let now = Utc::now();
        let stats = stats_rx.borrow().clone();
        let tools = tool_states(&stats, now, plan_limits::limits_for_analyzer);
        for sensor in sensors(&tools) {
            if discovered.contains(&sensor.key) {
                continue;
            }
            let payload = topics.discovery_payload(&sensor).to_string();
            if client.try_publish(topics.discovery(&sensor), QoS::AtLeastOnce, true, payload).is_ok() {
                discovered.insert(sensor.key);
            }
        }
        let state = state_payload(&tools, now).to_string();
        let _ = client.try_publish(topics.state(), QoS::AtLeastOnce, true, state);

        let rollups = [
            ("daily", serde_json::to_string(&rollup::daily_payload(&topics.host, &stats, now))),
            (
                "recent",
                serde_json::to_string(&rollup::recent_payload(
                    &topics.host,
                    &stats,
                    now,
                    config.share_previews,
                )),
            ),
            ("limits", serde_json::to_string(&rollup::limits_payload(now.timestamp() as f64))),
        ];
        for (kind, payload) in rollups {
            let Ok(payload) = payload else { continue };
            // Daily totals change with every message but are large: at most once a minute.
            let min_gap = if kind == "recent" { Duration::ZERO } else { ROLLUP_MIN_GAP };
            let stale = last_sent.get(kind).is_none_or(|(sent, at)| {
                (*sent != payload && at.elapsed() >= min_gap) || at.elapsed() >= ROLLUP_REFRESH
            });
            if stale
                && client
                    .try_publish(topics.rollup(kind), QoS::AtLeastOnce, true, payload.clone())
                    .is_ok()
            {
                last_sent.insert(kind, (payload, std::time::Instant::now()));
            }
        }
    }
}

/// Mark this machine offline before exiting (the last will covers a crash or lost network).
pub async fn shutdown() {
    if let Some((client, status)) = CLIENT.get() {
        let _ = client.try_publish(status, QoS::AtLeastOnce, true, "offline");
        let _ = client.try_disconnect();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_limits::WindowStatus;
    use crate::types::{AgenticCodingToolStats, Application, ConversationMessage, Stats};
    use std::sync::Arc;

    fn message(date: DateTime<Utc>, role: MessageRole, output: u64, cost: f64) -> ConversationMessage {
        ConversationMessage {
            application: Application::ClaudeCode,
            date,
            project_hash: String::new(),
            conversation_hash: "c".into(),
            local_hash: None,
            global_hash: format!("{date}{output}"),
            model: Some("m".into()),
            stats: Stats { output_tokens: output, cost, ..Default::default() },
            role,
            content: None,
        }
    }

    fn tool(name: &str, messages: Vec<ConversationMessage>) -> Arc<AgenticCodingToolStats> {
        Arc::new(AgenticCodingToolStats {
            daily_stats: Default::default(),
            num_conversations: 1,
            messages,
            analyzer_name: name.into(),
        })
    }

    #[test]
    fn slugs_are_topic_and_id_safe() {
        assert_eq!(slug("Claude Code (claude-v)"), "claude_code_claude_v");
        assert_eq!(slug("Amberts-MacBook-Pro"), "amberts_macbook_pro");
        assert_eq!(slug("  "), "");
    }

    #[test]
    fn rate_cost_and_activity() {
        let now = Utc::now();
        let stats = MultiAnalyzerStats {
            analyzer_stats: vec![
                tool(
                    "Claude Code",
                    vec![
                        message(now - ChronoDuration::minutes(5), MessageRole::Assistant, 9000, 1.5),
                        message(now - ChronoDuration::minutes(20), MessageRole::Assistant, 500, 0.0),
                        message(now - ChronoDuration::minutes(1), MessageRole::User, 0, 0.0),
                    ],
                ),
                tool("Cline", vec![message(now - ChronoDuration::days(90), MessageRole::Assistant, 10, 1.0)]),
                tool("Cursor Agent", vec![]),
            ],
        };
        let limits = |name: &str| {
            (name == "Cursor Agent").then_some(AccountLimits {
                five_hour: None,
                seven_day: None,
                monthly: Some(WindowStatus { used_percentage: 9.04, resets_in_secs: Some(10) }),
            })
        };
        let tools = tool_states(&stats, now, limits);
        assert_eq!(tools[0].tks, 10.0); // 9000 tokens over 900s; the 20-minute-old one is out.
        assert!(tools[0].active);
        assert!(!tools[1].active, "90 days idle and no limits");
        assert!(tools[2].active, "limits alone make a tool worth showing");

        let state = state_payload(&tools, now);
        assert_eq!(state["tks"], 10.0);
        assert_eq!(state["tools"]["cursor_agent"]["monthly"], 9.0);
        assert_eq!(state["tools"]["cursor_agent"]["five_hour"], Value::Null);
        assert!(state["tools"]["cline"].is_object(), "every tool stays in the state");

        let keys: Vec<_> = sensors(&tools).into_iter().map(|s| s.key).collect();
        assert!(keys.contains(&"claude_code_tks".to_string()));
        assert!(keys.contains(&"cursor_agent_monthly".to_string()));
        assert!(!keys.iter().any(|k| k.starts_with("cline")));
        assert!(!keys.contains(&"cursor_agent_five_hour".to_string()));
    }

    #[test]
    fn discovery_points_at_the_state_topic() {
        let topics = Topics::new("Amberts-MacBook-Pro");
        let sensor = Sensor {
            key: "tks".into(),
            name: "Output tk/s".into(),
            path: "value_json.tks".into(),
            kind: SensorKind::Rate,
        };
        assert_eq!(
            topics.discovery(&sensor),
            "homeassistant/sensor/splitrail_amberts_macbook_pro_tks/config"
        );
        let payload = topics.discovery_payload(&sensor);
        assert_eq!(payload["state_topic"], "splitrail/amberts_macbook_pro/state");
        assert_eq!(payload["value_template"], "{{ value_json.tks }}");
        assert_eq!(payload["availability_topic"], "splitrail/amberts_macbook_pro/status");
        assert_eq!(payload["unit_of_measurement"], "tk/s");
    }
}

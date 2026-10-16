use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub upload: UploadConfig,
    pub formatting: FormattingConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub warp: WarpConfig,
    #[serde(default)]
    pub pricing: PricingConfig,
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub goal: GoalConfig,
}

/// The output rate to average over each calendar month, every machine together; the menu bar's
/// dot shows whether the month is on track and whether the current speed keeps it there.
///
/// ```toml
/// [goal]
/// monthly_tks = 150
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GoalConfig {
    #[serde(default = "default_monthly_tks")]
    pub monthly_tks: f64,
}

impl Default for GoalConfig {
    fn default() -> Self {
        Self { monthly_tks: default_monthly_tks() }
    }
}

fn default_monthly_tks() -> f64 {
    150.0
}

/// Live stats published to an MQTT broker, with Home Assistant discovery. Off until `host` is
/// set. The password is read from `password_file` so it stays out of this file.
///
/// ```toml
/// [mqtt]
/// host = "192.168.163.163"
/// username = "splitrail"
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MqttConfig {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    /// Defaults to `~/.config/splitrail/mqtt_password`.
    #[serde(default)]
    pub password_file: Option<String>,
    /// Publish each tool's newest message text (one line, truncated, secrets redacted) so
    /// other machines' dashboards can show it. Off by default.
    #[serde(default)]
    pub share_previews: bool,
    /// Show other machines on the broker as extra columns in the dashboard. On by default.
    #[serde(default)]
    pub roll_up: Option<bool>,
}

/// Local inference is costed as the electricity it uses, at the household rate. The proxy
/// measures each request's energy with `macmon` when it can; `power_watts` × duration is the
/// fallback for requests logged without a measurement. Both default to 0, so Ollama stays at $0
/// until they are set.
///
/// ```toml
/// [ollama]
/// power_watts = 60          # extra draw while generating (M1 Max GPU-bound inference)
/// electricity_per_kwh = 0.24
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct OllamaConfig {
    #[serde(default)]
    pub power_watts: f64,
    #[serde(default)]
    pub electricity_per_kwh: f64,
}

impl OllamaConfig {
    /// Dollars of electricity for a measured amount of energy.
    pub fn joules_cost(&self, joules: f64) -> f64 {
        joules / 3_600_000.0 * self.electricity_per_kwh
    }
}

/// Claude Code accounts to read, one config directory each. Empty means auto-detect: `~/.claude`
/// plus any `~/.claude-<label>` directory that holds both `projects/` and `.claude.json`.
///
/// ```toml
/// [[claude_code.accounts]]
/// label = "main"
/// dir = "~/.claude"
///
/// [[claude_code.accounts]]
/// label = "v"
/// dir = "~/.claude-v"
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ClaudeCodeConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<ClaudeAccount>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ClaudeAccount {
    pub label: String,
    pub dir: String,
}

impl ClaudeCodeConfig {
    /// The accounts to read, as (label, directory), with `~` expanded.
    pub fn resolved_accounts(&self) -> Vec<(String, PathBuf)> {
        let Some(home) = std::env::home_dir() else {
            return Vec::new();
        };
        if !self.accounts.is_empty() {
            return self
                .accounts
                .iter()
                .map(|a| (a.label.clone(), expand_home(&a.dir, &home)))
                .collect();
        }
        detect_claude_accounts(&home)
    }
}

fn expand_home(dir: &str, home: &std::path::Path) -> PathBuf {
    match dir.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if dir == "~" => home.to_path_buf(),
        None => PathBuf::from(dir),
    }
}

/// `~/.claude` as "main", plus each `~/.claude-<label>` that looks like a Claude Code config
/// directory. Siblings such as `~/.claude-worktrees` have no `.claude.json` and are skipped.
pub fn detect_claude_accounts(home: &std::path::Path) -> Vec<(String, PathBuf)> {
    let mut accounts = vec![("main".to_string(), home.join(".claude"))];
    let Ok(entries) = fs::read_dir(home) else {
        return accounts;
    };
    let mut extra: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let label = name.strip_prefix(".claude-")?.to_string();
            let path = entry.path();
            (path.join("projects").is_dir() && path.join(".claude.json").is_file())
                .then_some((label, path))
        })
        .collect();
    extra.sort();
    accounts.extend(extra);
    accounts
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub url: String,
    pub api_token: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadConfig {
    pub auto_upload: bool,
    pub upload_today_only: bool,
    pub retry_attempts: u32,
    pub last_date_uploaded: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FormattingConfig {
    pub number_comma: bool,
    pub number_human: bool,
    pub locale: String,
    pub decimal_places: usize,
    #[serde(default = "default_health_display_style")]
    pub health_display_style: String, // "text" or "braille"
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct NotificationConfig {
    #[serde(default = "default_notifications_enabled")]
    pub enabled: bool,
    #[serde(default = "default_waiting_seconds")]
    pub waiting_seconds: u64,
    #[serde(default = "default_stale_minutes")]
    pub stale_minutes: u64,
    #[serde(default = "default_sample_messages")]
    pub sample_messages: usize,
    #[serde(default)]
    pub slack: SlackConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SlackConfig {
    #[serde(default = "default_slack_enabled")]
    pub enabled: bool,
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WarpConfig {
    pub auth_token: Option<String>,
    #[serde(default = "default_warp_auto_sync")]
    pub auto_sync: bool,
    #[serde(default = "default_warp_sync_interval_hours")]
    pub sync_interval_hours: u64,
}

/// Model pricing. Prices come from the live feed (see `price_feed`) unless the feed is disabled,
/// with `overrides` taking precedence over both the feed and the built-in table:
///
/// ```toml
/// [pricing.overrides."claude-opus-5-5"]
/// input_per_1m = 4.0
/// output_per_1m = 20.0
/// cache_write_per_1m = 5.0
/// cache_read_per_1m = 0.2
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PricingConfig {
    #[serde(default = "default_pricing_feed_enabled")]
    pub feed_enabled: bool,
    #[serde(default)]
    pub overrides: std::collections::HashMap<String, crate::price_feed::ModelPrice>,
}

impl Default for PricingConfig {
    fn default() -> Self {
        Self {
            feed_enabled: default_pricing_feed_enabled(),
            overrides: Default::default(),
        }
    }
}

fn default_pricing_feed_enabled() -> bool {
    true
}

fn default_warp_auto_sync() -> bool {
    true
}

fn default_warp_sync_interval_hours() -> u64 {
    6
}

fn default_health_display_style() -> String {
    "text".to_string()
}

fn default_notifications_enabled() -> bool {
    false
}

fn default_waiting_seconds() -> u64 {
    180
}

fn default_stale_minutes() -> u64 {
    180
}

fn default_sample_messages() -> usize {
    5
}

fn default_slack_enabled() -> bool {
    false
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                url: "https://splitrail.dev".to_string(),
                api_token: "".to_string(),
            },
            upload: UploadConfig {
                auto_upload: false,
                upload_today_only: false,
                retry_attempts: 3,
                last_date_uploaded: 0,
            },
            formatting: FormattingConfig {
                number_comma: false,
                number_human: false,
                locale: "en".to_string(),
                decimal_places: 2,
                health_display_style: "text".to_string(),
            },
            notifications: NotificationConfig {
                enabled: default_notifications_enabled(),
                waiting_seconds: default_waiting_seconds(),
                stale_minutes: default_stale_minutes(),
                sample_messages: default_sample_messages(),
                slack: SlackConfig::default(),
            },
            warp: WarpConfig::default(),
            pricing: PricingConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            ollama: OllamaConfig::default(),
            mqtt: MqttConfig::default(),
            goal: GoalConfig::default(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        Ok(std::env::home_dir()
            .context("Could not find home directory")?
            .join(".splitrail.toml"))
    }

    pub fn load() -> Result<Option<Config>> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&config_path).context("Failed to read config file")?;

        let config: Config = toml::from_str(&content).context("Failed to parse config file")?;

        Ok(Some(config))
    }

    pub fn save(&self, silent: bool) -> Result<()> {
        let config_path = Self::config_path()?;
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(&config_path, content).context("Failed to write config file")?;

        if !silent {
            println!("✅ Configuration saved to: {}", config_path.display());
        }

        Ok(())
    }

    pub fn set_api_token(&mut self, token: String) {
        self.server.api_token = token;
    }

    pub fn set_auto_upload(&mut self, enabled: bool) {
        self.upload.auto_upload = enabled;
    }

    pub fn set_upload_today_only(&mut self, enabled: bool) {
        self.upload.upload_today_only = enabled;
    }

    pub fn is_configured(&self) -> bool {
        !self.server.api_token.is_empty() && !self.server.url.is_empty()
    }

    pub fn is_api_token_missing(&self) -> bool {
        self.server.api_token.is_empty()
    }

    pub fn is_server_url_missing(&self) -> bool {
        self.server.url.is_empty()
    }

    pub fn set_last_date_uploaded(&mut self, date: i64) {
        self.upload.last_date_uploaded = date;
    }
}

// CLI helper functions
pub fn create_default_config(overwrite: bool) -> Result<()> {
    let config = Config::default();
    if !std::fs::exists(Config::config_path()?)? || overwrite {
        config.save(true)?;

        println!("📝 Created default configuration file.");
        println!("📍 Edit it with your Splitrail Cloud API token:");
        println!("   splitrail config set api-token ...");
        println!("or");
        println!("   {}", Config::config_path()?.display());
    } else {
        println!("Configuration already exists.  Pass `--overwrite` to overwrite.");
    }

    Ok(())
}

pub fn show_config() -> Result<()> {
    match Config::load()? {
        Some(config) => {
            println!("🔧 Current configuration:");
            println!(
                "   API Token: {}",
                if config.server.api_token.is_empty() {
                    "Not set"
                } else {
                    "Set"
                }
            );
            println!("   Auto Upload: {}", config.upload.auto_upload);
            println!("   Upload Today Only: {}", config.upload.upload_today_only);
            println!("   Number Comma: {}", config.formatting.number_comma);
            println!("   Number Human: {}", config.formatting.number_human);
            println!("   Locale: {}", config.formatting.locale);
            println!("   Decimal Places: {}", config.formatting.decimal_places);
            println!(
                "   Health Display Style: {}",
                config.formatting.health_display_style
            );
            println!("   Notifications Enabled: {}", config.notifications.enabled);
            println!(
                "   Notify After (sec): {}",
                config.notifications.waiting_seconds
            );
            println!(
                "   Stale After (min): {}",
                config.notifications.stale_minutes
            );
            println!(
                "   Sample Messages: {}",
                config.notifications.sample_messages
            );
            println!(
                "   Slack: {}",
                if config.notifications.slack.enabled {
                    if config.notifications.slack.webhook_url.is_some() {
                        "Enabled (webhook set)"
                    } else {
                        "Enabled (no webhook configured)"
                    }
                } else {
                    "Disabled"
                }
            );
            println!("   Pricing Feed: {}", config.pricing.feed_enabled);
            println!("   Price Overrides: {}", config.pricing.overrides.len());
            for (label, dir) in config.claude_code.resolved_accounts() {
                println!("   Claude Account: {label} → {}", dir.display());
            }
        }
        None => {
            println!("❌ No configuration file found.");
            println!("   Run 'splitrail config init' to create one.");
        }
    }
    Ok(())
}

pub fn set_config_value(key: &str, value: &str) -> Result<()> {
    let mut config = Config::load()?.unwrap_or_default();

    match key {
        "api-token" => config.set_api_token(value.to_string()),
        "auto-upload" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.set_auto_upload(enabled);
        }
        "upload-today-only" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.set_upload_today_only(enabled);
        }
        "number-comma" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.formatting.number_comma = enabled;
        }
        "number-human" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.formatting.number_human = enabled;
        }
        "locale" => {
            config.formatting.locale = value.to_string();
        }
        "decimal-places" => {
            let places = value.parse::<usize>().context("Invalid number value")?;
            config.formatting.decimal_places = places;
        }
        "health-display-style" => {
            if value == "text" || value == "braille" {
                config.formatting.health_display_style = value.to_string();
            } else {
                anyhow::bail!("Invalid health display style. Use 'text' or 'braille'");
            }
        }
        "notifications-enabled" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.notifications.enabled = enabled;
        }
        "notifications-wait-seconds" => {
            let seconds = value
                .parse::<u64>()
                .context("Invalid number value for wait seconds")?;
            config.notifications.waiting_seconds = seconds;
        }
        "notifications-stale-minutes" => {
            let minutes = value
                .parse::<u64>()
                .context("Invalid number value for stale minutes")?;
            config.notifications.stale_minutes = minutes;
        }
        "notifications-sample-messages" => {
            let count = value
                .parse::<usize>()
                .context("Invalid number value for sample messages")?;
            config.notifications.sample_messages = count.max(1);
        }
        "slack-enabled" => {
            let enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
            config.notifications.slack.enabled = enabled;
        }
        "slack-webhook-url" => {
            config.notifications.slack.webhook_url = Some(value.to_string());
        }
        "slack-channel" => {
            config.notifications.slack.channel = Some(value.to_string());
        }
        "slack-username" => {
            config.notifications.slack.username = Some(value.to_string());
        }
        "pricing-feed-enabled" => {
            config.pricing.feed_enabled = value
                .parse::<bool>()
                .context("Invalid boolean value. Use 'true' or 'false'")?;
        }
        _ => anyhow::bail!("Unknown config key: {}", key),
    }

    config.save(false)?;
    Ok(())
}

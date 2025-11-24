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
        _ => anyhow::bail!("Unknown config key: {}", key),
    }

    config.save(false)?;
    Ok(())
}

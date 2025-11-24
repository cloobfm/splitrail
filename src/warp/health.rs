//! WARP token health checking and management
//!
//! This module provides functionality to:
//! - Check if WARP auth token is valid
//! - Extract tokens from config or WARP installation
//! - Monitor token health with daily checks
//! - Provide user-friendly error messages

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Status of the WARP authentication token
#[derive(Debug, Clone, PartialEq)]
pub enum WarpTokenStatus {
    /// Token is valid and working
    Valid,
    /// Token not found in config
    NotConfigured,
    /// Token exists but is expired/invalid
    Expired,
    /// Error checking token (network issue, etc.)
    Error(String),
}

/// Health check cache to avoid repeated API calls
#[derive(Debug, Serialize, Deserialize)]
struct WarpHealthCache {
    last_check: DateTime<Utc>,
    status: String,
    last_sync: Option<DateTime<Utc>>,
    conversations_synced: Option<usize>,
}

impl WarpHealthCache {
    fn cache_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("splitrail")
            .join("warp_health.json")
    }

    fn load() -> Option<Self> {
        let path = Self::cache_path();
        let content = fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save(&self) -> Result<()> {
        let path = Self::cache_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn should_check(&self) -> bool {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.last_check);
        // Check once per day (86400 seconds)
        duration.num_seconds() > 86400
    }
}

/// Check if we're running inside WARP terminal
pub fn is_running_in_warp() -> bool {
    std::env::var("TERM_PROGRAM")
        .ok()
        .map(|v| v.contains("WarpTerminal") || v == "WarpTerminal")
        .unwrap_or(false)
        || std::env::var("WARP_USE_SSH_WRAPPER").is_ok()
}

/// Get WARP auth token from config
fn get_warp_token() -> Result<String> {
    let config_path = dirs::config_dir()
        .context("Could not find config directory")?
        .join("splitrail")
        .join("config.toml");

    let content = fs::read_to_string(&config_path)
        .context("Could not read config file. Run 'splitrail config init' first.")?;

    let config: toml::Value = toml::from_str(&content)
        .context("Invalid TOML in config file")?;

    config
        .get("warp")
        .and_then(|w| w.get("auth_token"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .context("WARP auth_token not found in config. Run 'splitrail warp setup'")
}

/// Check if WARP token health should be checked (daily limit)
pub fn should_check_warp_token() -> bool {
    match WarpHealthCache::load() {
        Some(cache) => cache.should_check(),
        None => true, // No cache, should check
    }
}

/// Perform a lightweight API call to check if token is valid
async fn test_token(token: &str) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let response = client
        .post("https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo")
        .header("Authorization", token)
        .header("Content-Type", "application/json")
        .send()
        .await?;

    Ok(response.status().is_success())
}

/// Check WARP token health with caching
pub async fn check_warp_token_health() -> WarpTokenStatus {
    // Check cache first
    if let Some(cache) = WarpHealthCache::load() {
        if !cache.should_check() {
            // Return cached status if checked recently
            return match cache.status.as_str() {
                "valid" => WarpTokenStatus::Valid,
                "expired" => WarpTokenStatus::Expired,
                "not_configured" => WarpTokenStatus::NotConfigured,
                _ => WarpTokenStatus::Error("Unknown cached status".to_string()),
            };
        }
    }

    // Get token from config
    let token = match get_warp_token() {
        Ok(t) => t,
        Err(_) => {
            // Save "not configured" status to cache
            let cache = WarpHealthCache {
                last_check: Utc::now(),
                status: "not_configured".to_string(),
                last_sync: None,
                conversations_synced: None,
            };
            let _ = cache.save();
            return WarpTokenStatus::NotConfigured;
        }
    };

    // Test the token
    let status = match test_token(&token).await {
        Ok(true) => {
            // Token is valid
            let cache = WarpHealthCache {
                last_check: Utc::now(),
                status: "valid".to_string(),
                last_sync: None,
                conversations_synced: None,
            };
            let _ = cache.save();
            WarpTokenStatus::Valid
        }
        Ok(false) => {
            // Token exists but is invalid/expired
            let cache = WarpHealthCache {
                last_check: Utc::now(),
                status: "expired".to_string(),
                last_sync: None,
                conversations_synced: None,
            };
            let _ = cache.save();
            WarpTokenStatus::Expired
        }
        Err(e) => {
            // Network error or other issue - don't cache
            WarpTokenStatus::Error(e.to_string())
        }
    };

    status
}

/// Display status message to user based on token health
pub fn display_warp_status(status: &WarpTokenStatus) {
    match status {
        WarpTokenStatus::Valid => {
            // Silent success - no message needed
        }
        WarpTokenStatus::NotConfigured => {
            if is_running_in_warp() {
                println!("ℹ️  WARP analytics not configured");
                println!("   Enable with: splitrail warp setup");
            }
        }
        WarpTokenStatus::Expired => {
            println!("⚠️  WARP token expired");
            println!("   Refresh with: splitrail warp refresh");
        }
        WarpTokenStatus::Error(e) => {
            eprintln!("Note: Could not check WARP token status: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_running_in_warp() {
        // Save original env
        let original = std::env::var("TERM_PROGRAM").ok();

        // Test with WARP env var
        std::env::set_var("TERM_PROGRAM", "WarpTerminal");
        assert!(is_running_in_warp());

        // Test without WARP env var
        std::env::remove_var("TERM_PROGRAM");
        std::env::remove_var("WARP_USE_SSH_WRAPPER");
        assert!(!is_running_in_warp());

        // Restore original
        if let Some(val) = original {
            std::env::set_var("TERM_PROGRAM", val);
        }
    }
}

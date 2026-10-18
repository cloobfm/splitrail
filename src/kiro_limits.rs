//! Kiro's monthly credits, from Kiro itself: `kiro-cli chat --no-interactive /usage` prints the
//! plan, credits used of the allowance and the reset date, using Kiro's own sign-in (nothing of
//! it is read here). Kiro has no weekly window.
//!
//! ```text
//! Estimated Usage | resets on 2026-11-01 | KIRO FREE
//! Credits (0.55 of 50 covered in plan)
//! ████████ 1%
//! Overages: Disabled
//! ```
//!
//! Every ten minutes this writes `~/.config/splitrail/limits/kiro.json` in the shape the other
//! monthly allowances use (`plan_limits`), the plan as the login name.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde_json::{Value, json};
use std::time::Duration;

pub const SNAPSHOT_LABEL: &str = "kiro";
const POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(60);

/// Terminal colour and cursor codes out of `text`.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Parameters, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// `Kiro Free` from `KIRO FREE`.
fn plan_name(raw: &str) -> String {
    raw.split_whitespace()
        .map(|word| {
            let lower = word.to_lowercase();
            let mut chars = lower.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// The snapshot for `/usage` output captured at `captured_at` (epoch seconds), or `None` if it
/// has no credit line.
pub fn snapshot_from_usage(output: &str, captured_at: f64) -> Option<Value> {
    let text = strip_ansi(output);
    let header = text.lines().find(|l| l.contains("resets on"))?;
    let mut parts = header.split('|').map(str::trim);
    let _ = parts.next();
    let resets_at = parts
        .next()
        .and_then(|p| p.strip_prefix("resets on"))
        .and_then(|d| NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|t| t.and_utc().timestamp() as f64);
    let plan = parts.next().map(plan_name).filter(|p| !p.is_empty());
    // `Credits (0.55 of 50 covered in plan)`
    let credits = text
        .lines()
        .find(|l| l.trim_start().starts_with("Credits"))?;
    let inside = credits.split('(').nth(1)?.split(')').next()?;
    let mut numbers = inside
        .split_whitespace()
        .filter_map(|w| w.replace(',', "").parse::<f64>().ok());
    let used = numbers.next()?;
    let limit = numbers.next().filter(|l| *l > 0.0)?;
    let overages = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Overages:"))
        .map(|v| v.trim().eq_ignore_ascii_case("enabled"));
    Some(json!({
        "account": SNAPSHOT_LABEL,
        "captured_at": captured_at,
        "monthly": { "used_percentage": used / limit * 100.0, "resets_at": resets_at },
        "credits_used": used,
        "credits_limit": limit,
        "overages_enabled": overages,
        "login_name": plan,
    }))
}

fn kiro_cli() -> Option<std::path::PathBuf> {
    let home = std::env::home_dir()?;
    [
        home.join(".local/bin/kiro-cli"),
        "/Applications/Kiro CLI.app/Contents/MacOS/kiro-cli".into(),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

async fn poll_once(cli: &std::path::Path) -> Result<()> {
    let output = tokio::time::timeout(
        TIMEOUT,
        tokio::process::Command::new(cli)
            .args(["chat", "--no-interactive", "/usage"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Kiro took too long to report usage")?
    .context("Could not run kiro-cli")?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    let snapshot = snapshot_from_usage(&text, now)
        .context("Kiro's /usage had no credit line (signed out?)")?;
    let dir = std::env::home_dir()
        .context("No home directory")?
        .join(".config/splitrail/limits");
    std::fs::create_dir_all(&dir)?;
    // Write-then-rename so the dashboard never reads a half-written file.
    let tmp = dir.join(format!(".{SNAPSHOT_LABEL}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&snapshot)?)?;
    std::fs::rename(&tmp, dir.join(format!("{SNAPSHOT_LABEL}.json")))?;
    Ok(())
}

/// Ask Kiro for its usage every ten minutes while the dashboard runs. Does nothing without it.
pub async fn run() {
    let Some(cli) = kiro_cli() else {
        return;
    };
    let mut reported = false;
    loop {
        match poll_once(&cli).await {
            Ok(()) => reported = false,
            Err(e) if !reported => {
                crate::utils::log_error(format!("Kiro credits: {e:#}"));
                reported = true; // Once per failure streak, not every poll.
            }
            Err(_) => {}
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "\n\u{1b}[1mEstimated Usage\u{1b}[0m | resets on 2026-11-01 | \u{1b}[38;5;141mKIRO FREE\u{1b}[0m\n\u{1b}[1mCredits\u{1b}[0m (12.5 of 50 covered in plan)\n\u{1b}[38;5;141m████\u{1b}[0m 25%\n\nOverages: \u{1b}[1mDisabled\u{1b}[0m\n";

    #[test]
    fn usage_output_becomes_a_monthly_snapshot() {
        let snapshot = snapshot_from_usage(OUTPUT, 1000.0).unwrap();
        assert_eq!(snapshot["monthly"]["used_percentage"], 25.0);
        assert_eq!(snapshot["monthly"]["resets_at"], 1_793_491_200.0);
        assert_eq!(snapshot["login_name"], "Kiro Free");
        assert_eq!(snapshot["overages_enabled"], false);
        let limits = crate::plan_limits::parse_snapshot(&snapshot.to_string(), 1001.0).unwrap();
        assert_eq!(limits.monthly.unwrap().used_percentage, 25.0);
    }

    #[test]
    fn output_without_credits_gives_nothing() {
        assert!(snapshot_from_usage("Not logged in", 0.0).is_none());
        assert!(
            snapshot_from_usage("Estimated Usage | resets on 2026-11-01 | KIRO PRO\n", 0.0)
                .is_none()
        );
    }

    /// Asks the real Kiro: `cargo test kiro_limits_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn kiro_limits_live_writes_the_snapshot() {
        poll_once(&kiro_cli().unwrap()).await.unwrap();
        let limits = crate::plan_limits::limits_for_analyzer("Kiro CLI").unwrap();
        println!(
            "monthly {:?} plan {:?}",
            limits.monthly,
            crate::plan_limits::login_for_analyzer("Kiro CLI")
        );
    }
}

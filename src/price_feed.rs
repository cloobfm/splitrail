//! Live model pricing from LiteLLM's public price list.
//!
//! Vendors do not publish prices through their APIs, so the hardcoded table in `models.rs` goes
//! stale whenever a model ships. This module fetches LiteLLM's community-maintained price file at
//! most once a day, caches it on disk, and puts it in front of that table. Lookup order:
//!
//! 1. `[pricing.overrides]` in the config file
//! 2. the feed (the on-disk cache, refreshed when older than a day)
//! 3. the hardcoded table in `models.rs`
//!
//! A refresh only changes costs computed after it lands; history already aggregated or uploaded
//! is not re-priced.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::utils::warn_once;

pub const FEED_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Refresh the cached feed once it is older than this.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// A refreshed price more than this many times higher or lower than the one it replaces is
/// treated as a bad upstream commit and ignored in favour of the previous price.
const MAX_JUMP: f64 = 10.0;

/// A price above a token threshold, for models that charge more for long prompts. Applied the
/// same progressive way as `models::PricingTier`: tokens up to the threshold at the base rate,
/// the remainder at this rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceTier {
    pub above_tokens: u64,
    pub input_per_1m: f64,
    pub output_per_1m: f64,
}

/// Prices for one model, per 1M tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input_per_1m: f64,
    pub output_per_1m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<PriceTier>,
}

impl ModelPrice {
    pub fn input_cost(&self, tokens: u64) -> f64 {
        self.tiered_cost(tokens, self.input_per_1m, |t| t.input_per_1m)
    }

    pub fn output_cost(&self, tokens: u64) -> f64 {
        self.tiered_cost(tokens, self.output_per_1m, |t| t.output_per_1m)
    }

    /// A model with no cache-write price (OpenAI, Google) charges nothing for creation, matching
    /// how `models::CachingSupport` treats those vendors.
    pub fn cache_cost(&self, creation_tokens: u64, read_tokens: u64) -> f64 {
        per_1m(creation_tokens, self.cache_write_per_1m.unwrap_or(0.0))
            + per_1m(read_tokens, self.cache_read_per_1m.unwrap_or(0.0))
    }

    fn tiered_cost(&self, tokens: u64, base_rate: f64, rate: impl Fn(&PriceTier) -> f64) -> f64 {
        let mut cost = 0.0;
        let mut floor = 0u64;
        let mut current_rate = base_rate;
        for tier in &self.tiers {
            if tokens <= tier.above_tokens {
                break;
            }
            cost += per_1m(tier.above_tokens - floor, current_rate);
            floor = tier.above_tokens;
            current_rate = rate(tier);
        }
        cost + per_1m(tokens - floor, current_rate)
    }
}

fn per_1m(tokens: u64, rate: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * rate
}

/// Where a resolved price came from, for `splitrail pricing show`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    Override,
    Feed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    fetched_at: u64,
    source: String,
    models: HashMap<String, ModelPrice>,
}

static FEED: RwLock<Option<Arc<HashMap<String, ModelPrice>>>> = RwLock::new(None);
static OVERRIDES: OnceLock<HashMap<String, ModelPrice>> = OnceLock::new();

/// Load the on-disk cache and the config overrides. Cheap and offline; call before any parsing.
pub fn init(overrides: HashMap<String, ModelPrice>, feed_enabled: bool) {
    let _ = OVERRIDES.set(overrides);
    if feed_enabled && let Ok(Some(cache)) = read_cache() {
        install(cache.models);
    }
}

/// Resolve a model's price from the overrides or the feed. `None` means fall through to the
/// hardcoded table.
pub fn lookup(model_name: &str) -> Option<(ModelPrice, PriceSource)> {
    if let Some(price) = OVERRIDES.get().and_then(|o| o.get(model_name)) {
        return Some((price.clone(), PriceSource::Override));
    }
    let feed = FEED.read().ok()?;
    feed.as_ref()?
        .get(model_name)
        .map(|price| (price.clone(), PriceSource::Feed))
}

/// Whether the cached feed is missing or older than a day.
pub fn is_stale() -> bool {
    match read_cache() {
        Ok(Some(cache)) => now_secs().saturating_sub(cache.fetched_at) > MAX_AGE.as_secs(),
        _ => true,
    }
}

/// Whether any feed is cached on disk, however old.
pub fn has_cache() -> bool {
    matches!(read_cache(), Ok(Some(_)))
}

/// Summary of a refresh, for the CLI.
pub struct RefreshReport {
    pub models: usize,
    pub rejected: Vec<String>,
}

/// Fetch the feed, sanity-check it against the previous prices, write the cache, and install it.
pub async fn refresh() -> Result<RefreshReport> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("Failed to build HTTP client")?;
    let body = client
        .get(FEED_URL)
        .send()
        .await
        .context("Failed to fetch price feed")?
        .error_for_status()
        .context("Price feed returned an error")?
        .text()
        .await
        .context("Failed to read price feed")?;

    let fetched = parse_litellm(&body)?;
    let previous = read_cache()
        .ok()
        .flatten()
        .map(|c| c.models)
        .unwrap_or_default();
    let (models, rejected) = guard_jumps(fetched, &previous, |name| {
        crate::models::builtin_flat_price(name)
    });
    for name in &rejected {
        warn_once(format!(
            "WARNING: Price feed changed {name} by more than {MAX_JUMP}x; keeping the previous price."
        ));
    }

    let report = RefreshReport {
        models: models.len(),
        rejected,
    };
    write_cache(&CacheFile {
        fetched_at: now_secs(),
        source: FEED_URL.to_string(),
        models: models.clone(),
    })?;
    install(models);
    Ok(report)
}

/// Refresh if stale, swallowing errors: a failed refresh leaves the cached or built-in prices in
/// place, which is the right degradation for a background task.
pub async fn refresh_if_stale(feed_enabled: bool) {
    if feed_enabled
        && is_stale()
        && let Err(e) = refresh().await
    {
        warn_once(format!("WARNING: Could not refresh model prices: {e:#}"));
    }
}

fn install(models: HashMap<String, ModelPrice>) {
    if let Ok(mut feed) = FEED.write() {
        *feed = Some(Arc::new(models));
    }
}

/// Parse LiteLLM's price file. Entries without both an input and an output price (embeddings,
/// image models, the `sample_spec` entry) are skipped.
pub fn parse_litellm(body: &str) -> Result<HashMap<String, ModelPrice>> {
    let raw: HashMap<String, serde_json::Value> =
        serde_json::from_str(body).context("Price feed is not valid JSON")?;

    let mut models = HashMap::new();
    for (name, entry) in raw {
        let Some(fields) = entry.as_object() else {
            continue;
        };
        let rate = |key: &str| fields.get(key).and_then(|v| v.as_f64()).map(|v| v * 1e6);
        let (Some(input_per_1m), Some(output_per_1m)) =
            (rate("input_cost_per_token"), rate("output_cost_per_token"))
        else {
            continue;
        };

        // Long-context surcharges appear as `input_cost_per_token_above_{N}k_tokens`.
        let mut tiers: Vec<PriceTier> = fields
            .keys()
            .filter_map(|key| {
                let thousands: u64 = key
                    .strip_prefix("input_cost_per_token_above_")?
                    .strip_suffix("k_tokens")?
                    .parse()
                    .ok()?;
                let suffix = format!("above_{thousands}k_tokens");
                Some(PriceTier {
                    above_tokens: thousands * 1_000,
                    input_per_1m: rate(key)?,
                    output_per_1m: rate(&format!("output_cost_per_token_{suffix}"))
                        .unwrap_or(output_per_1m),
                })
            })
            .collect();
        tiers.sort_by_key(|t| t.above_tokens);

        models.insert(
            name,
            ModelPrice {
                input_per_1m,
                output_per_1m,
                cache_write_per_1m: rate("cache_creation_input_token_cost"),
                cache_read_per_1m: rate("cache_read_input_token_cost"),
                tiers,
            },
        );
    }

    if models.is_empty() {
        anyhow::bail!("Price feed contained no priced models");
    }
    Ok(models)
}

/// Drop fetched prices that moved more than `MAX_JUMP`x from the previous feed (or, for a model
/// the previous feed lacked, from the built-in table), keeping the previous price instead.
/// Returns the merged map and the names whose update was rejected.
fn guard_jumps(
    mut fetched: HashMap<String, ModelPrice>,
    previous: &HashMap<String, ModelPrice>,
    builtin: impl Fn(&str) -> Option<(f64, f64)>,
) -> (HashMap<String, ModelPrice>, Vec<String>) {
    let jumped = |old: f64, new: f64| {
        old > 0.0 && new > 0.0 && (new / old > MAX_JUMP || old / new > MAX_JUMP)
    };

    let mut rejected: Vec<String> = fetched
        .iter()
        .filter(|(name, price)| {
            let baseline = previous
                .get(*name)
                .map(|p| (p.input_per_1m, p.output_per_1m))
                .or_else(|| builtin(name));
            baseline.is_some_and(|(old_in, old_out)| {
                jumped(old_in, price.input_per_1m) || jumped(old_out, price.output_per_1m)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    rejected.sort();

    for name in &rejected {
        match previous.get(name) {
            Some(prev) => {
                fetched.insert(name.clone(), prev.clone());
            }
            // No previous feed price to keep: leave the name out so lookups fall through to
            // the built-in table.
            None => {
                fetched.remove(name);
            }
        }
    }
    (fetched, rejected)
}

fn cache_path() -> Result<PathBuf> {
    Ok(std::env::home_dir()
        .context("Could not find home directory")?
        .join(".config")
        .join("splitrail")
        .join("prices.json"))
}

fn read_cache() -> Result<Option<CacheFile>> {
    let path = cache_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).context("Failed to read price cache")?;
    Ok(Some(
        serde_json::from_str(&content).context("Failed to parse price cache")?,
    ))
}

/// Write via a temp file and rename, so a crash mid-write never leaves a truncated cache.
fn write_cache(cache: &CacheFile) -> Result<()> {
    let path = cache_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).context("Failed to create config directory")?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(cache)?).context("Failed to write price cache")?;
    std::fs::rename(&tmp, &path).context("Failed to replace price cache")?;
    Ok(())
}

/// Age of the cached feed, for `splitrail pricing show`.
pub fn cache_age() -> Option<Duration> {
    let cache = read_cache().ok().flatten()?;
    Some(Duration::from_secs(
        now_secs().saturating_sub(cache.fetched_at),
    ))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "sample_spec": {"max_tokens": "set to max"},
        "claude-opus-5-5": {
            "input_cost_per_token": 4e-06,
            "output_cost_per_token": 2e-05,
            "cache_creation_input_token_cost": 5e-06,
            "cache_read_input_token_cost": 2e-07
        },
        "gemini-long": {
            "input_cost_per_token": 1e-06,
            "output_cost_per_token": 4e-06,
            "input_cost_per_token_above_200k_tokens": 2e-06,
            "output_cost_per_token_above_200k_tokens": 8e-06
        },
        "text-embedding-3-small": {"input_cost_per_token": 2e-08}
    }"#;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn parses_chat_models_and_skips_the_rest() {
        let models = parse_litellm(SAMPLE).unwrap();
        assert_eq!(models.len(), 2);
        let opus = &models["claude-opus-5-5"];
        assert!(close(opus.input_per_1m, 4.0));
        assert!(close(opus.output_per_1m, 20.0));
        assert!(close(opus.cache_write_per_1m.unwrap(), 5.0));
        assert!(close(opus.cache_read_per_1m.unwrap(), 0.2));
    }

    #[test]
    fn long_context_tier_is_progressive() {
        let models = parse_litellm(SAMPLE).unwrap();
        let gemini = &models["gemini-long"];
        assert_eq!(gemini.tiers.len(), 1);
        // 200K at $1/M + 100K at $2/M
        assert!(close(gemini.input_cost(300_000), 0.4));
        assert!(close(gemini.input_cost(100_000), 0.1));
        assert!(close(gemini.output_cost(300_000), 1.6));
    }

    #[test]
    fn cache_cost_without_write_price_charges_reads_only() {
        let price = ModelPrice {
            input_per_1m: 1.0,
            output_per_1m: 2.0,
            cache_write_per_1m: None,
            cache_read_per_1m: Some(0.5),
            tiers: vec![],
        };
        assert!(close(price.cache_cost(1_000_000, 1_000_000), 0.5));
    }

    #[test]
    fn jump_guard_keeps_previous_price() {
        let fetched = parse_litellm(SAMPLE).unwrap();
        let mut previous = HashMap::new();
        // Previous feed had Opus 5.5 at $0.30 input: the new $4 is a >10x jump.
        previous.insert(
            "claude-opus-5-5".to_string(),
            ModelPrice {
                input_per_1m: 0.3,
                output_per_1m: 20.0,
                cache_write_per_1m: None,
                cache_read_per_1m: None,
                tiers: vec![],
            },
        );
        let (merged, rejected) = guard_jumps(fetched, &previous, |_| None);
        assert_eq!(rejected, vec!["claude-opus-5-5".to_string()]);
        assert!(close(merged["claude-opus-5-5"].input_per_1m, 0.3));
        assert!(close(merged["gemini-long"].input_per_1m, 1.0));
    }

    #[test]
    fn jump_guard_against_builtin_drops_to_fallback() {
        let fetched = parse_litellm(SAMPLE).unwrap();
        let (merged, rejected) = guard_jumps(fetched, &HashMap::new(), |name| {
            (name == "gemini-long").then_some((50.0, 4.0))
        });
        assert_eq!(rejected, vec!["gemini-long".to_string()]);
        assert!(!merged.contains_key("gemini-long"));
        assert!(merged.contains_key("claude-opus-5-5"));
    }

    #[test]
    fn small_moves_pass_the_guard() {
        let fetched = parse_litellm(SAMPLE).unwrap();
        let (merged, rejected) = guard_jumps(fetched, &HashMap::new(), |_| Some((5.0, 25.0)));
        assert!(rejected.is_empty());
        assert_eq!(merged.len(), 2);
    }
}

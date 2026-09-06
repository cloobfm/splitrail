use phf::phf_map;
use serde::{Deserialize, Serialize};

use crate::utils::warn_once;

/// Represents different pricing tier structures for various models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingTier {
    /// Maximum tokens for this tier (None means unlimited - highest tier)
    pub max_tokens: Option<u64>,
    /// Input cost per 1M tokens
    pub input_per_1m: f64,
    /// Output cost per 1M tokens
    pub output_per_1m: f64,
}

/// Different pricing structures supported by various model providers
#[derive(Debug, Clone)]
pub enum PricingStructure {
    /// Flat rate pricing (same cost regardless of token count)
    Flat {
        input_per_1m: f64,
        output_per_1m: f64,
    },
    /// Tiered pricing (different costs based on token thresholds)
    Tiered { tiers: &'static [PricingTier] },
}

/// Caching tier for models with tiered cache pricing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachingTier {
    /// Maximum tokens for this caching tier (None means unlimited)
    pub max_tokens: Option<u64>,
    /// Cached input cost per 1M tokens
    pub cached_input_per_1m: f64,
}

/// Different caching support models
#[derive(Debug, Clone)]
pub enum CachingSupport {
    /// Model does not support caching
    None,
    /// OpenAI-style caching (simple cached input pricing)
    OpenAI { cached_input_per_1m: f64 },
    /// Anthropic-style caching (separate write and read costs)
    Anthropic {
        cache_write_per_1m: f64,
        cache_read_per_1m: f64,
    },
    /// Google-style caching (may have tiers like input/output)
    Google { tiers: &'static [CachingTier] },
}

/// Complete model information with all pricing details
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Pricing structure (flat or tiered)
    pub pricing: PricingStructure,
    /// Caching support and pricing
    pub caching: CachingSupport,
}

static MODEL_INDEX: phf::Map<&'static str, ModelInfo> = phf_map! {
    // OpenAI Models
    "o4-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.1,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.275,
        },
    },
    "o3" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.5,
        },
    },
    "o3-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 20.0,
            output_per_1m: 80.0,
        },
        caching: CachingSupport::None,
    },
    "o3-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.1,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.55,
        },
    },
    "o1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 15.0,
            output_per_1m: 60.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 7.5,
        },
    },
    "o1-preview" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 15.0,
            output_per_1m: 60.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 7.5,
        },
    },
    "o1-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.1,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.55,
        },
    },
    "o1-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 150.0,
            output_per_1m: 600.0,
        },
        caching: CachingSupport::None,
    },
    "gpt-4.1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.5,
        },
    },
    "gpt-4o" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.5,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 1.25,
        },
    },
    "gpt-4o-2024-05-13" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::None,
    },
    "gpt-4.1-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.4,
            output_per_1m: 1.6,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.1,
        },
    },
    "gpt-4.1-nano" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.1,
            output_per_1m: 0.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.025,
        },
    },
    "gpt-4o-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.15,
            output_per_1m: 0.6,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.075,
        },
    },
    "codex-mini-latest" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.5,
            output_per_1m: 6.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.375,
        },
    },
    "gpt-4-turbo" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 30.0,
        },
        caching: CachingSupport::None,
    },
    "gpt-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.125,
        },
    },
    "gpt-5-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 2.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.025,
        },
    },
    "gpt-5-nano" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.05,
            output_per_1m: 0.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.005,
        },
    },
    "gpt-5-codex-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 2.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.025,
        },
    },
    "gpt-5.1-codex" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.125,
        },
    },
    "gpt-5.1-codex-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 2.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.025,
        },
    },
    "gpt-5.1-codex-max" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.125,
        },
    },
    "gpt-5.2-codex" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.175,
        },
    },
    "coder-alpha2" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,
            output_per_1m: 0.0,
        },
        caching: CachingSupport::None,
    },

    // OpenAI 2026 models (GPT-5.1 through GPT-6)
    "gpt-6-astra" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 50.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 1.0,
        },
    },
    "gpt-5.6-sol" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 4.0,
            output_per_1m: 20.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.4,
        },
    },
    "gpt-5.6-terra" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.0,
            output_per_1m: 12.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.2,
        },
    },
    "gpt-5.6-luna" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.2,
            output_per_1m: 1.2,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.02,
        },
    },
    "gpt-5.5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 30.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.5,
        },
    },
    "gpt-5.5-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
        },
        caching: CachingSupport::None,
    },
    "gpt-5.4" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.5,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.25,
        },
    },
    "gpt-5.4-mini" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.75,
            output_per_1m: 4.5,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.075,
        },
    },
    "gpt-5.4-nano" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.2,
            output_per_1m: 1.25,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.02,
        },
    },
    "gpt-5.3-codex" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.175,
        },
    },
    "gpt-5.2" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.175,
        },
    },
    "gpt-5.1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.125,
        },
    },

    // Anthropic Models
    "claude-opus-4-1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 15.0,
            output_per_1m: 75.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 18.75,
            cache_read_per_1m: 1.5,
        },
    },
    "claude-opus-4" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 15.0,
            output_per_1m: 75.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 18.75,
            cache_read_per_1m: 1.5,
        },
    },
    "claude-sonnet-4" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },
    "claude-sonnet-4-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },
    "claude-sonnet-4.5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },
    "claude-3-7-sonnet" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },
    "claude-3-5-sonnet" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },
    "claude-3-5-haiku" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.8,
            output_per_1m: 4.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 1.0,
            cache_read_per_1m: 0.08,
        },
    },
    "claude-haiku-4-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.0,
            output_per_1m: 5.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 1.25,
            cache_read_per_1m: 0.10,
        },
    },
    "claude-3-opus" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 15.0,
            output_per_1m: 75.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 18.75,
            cache_read_per_1m: 1.5,
        },
    },
    "claude-3-haiku" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 1.25,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 0.3,
            cache_read_per_1m: 0.03,
        },
    },
    "claude-opus-4-5-20251101" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 25.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 6.25,
            cache_read_per_1m: 0.5,
        },
    },

    // Anthropic 2026 models (Claude 4.6 through Fable 5.1). Cache write = 5m rate (1.25x).
    "claude-fable-5-1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 50.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 12.5,
            cache_read_per_1m: 0.25,
        },
    },
    "claude-mythos-5-1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 50.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 12.5,
            cache_read_per_1m: 0.25,
        },
    },
    "claude-fable-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 50.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 12.5,
            cache_read_per_1m: 1.0,
        },
    },
    "claude-mythos-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 10.0,
            output_per_1m: 50.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 12.5,
            cache_read_per_1m: 1.0,
        },
    },
    "claude-opus-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 25.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 6.25,
            cache_read_per_1m: 0.5,
        },
    },
    "claude-opus-4-8" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 25.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 6.25,
            cache_read_per_1m: 0.5,
        },
    },
    "claude-opus-4-7" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 25.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 6.25,
            cache_read_per_1m: 0.5,
        },
    },
    "claude-opus-4-6" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 5.0,
            output_per_1m: 25.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 6.25,
            cache_read_per_1m: 0.5,
        },
    },
    "claude-sonnet-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.0,
            output_per_1m: 10.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 2.5,
            cache_read_per_1m: 0.2,
        },
    },
    "claude-sonnet-4-6" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::Anthropic {
            cache_write_per_1m: 3.75,
            cache_read_per_1m: 0.3,
        },
    },

    // Google Models
    "gemini-3-pro-preview-11-2025" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 1.25,
                    output_per_1m: 10.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.31,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.625,
                },
            ],
        },
    },
    "gemini-3-pro-preview" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 2.0,
                    output_per_1m: 12.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 4.0,
                    output_per_1m: 18.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.25,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.4,
                },
            ],
        },
    },

    "gemini-2.5-pro" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 1.25,
                    output_per_1m: 10.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.31,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.625,
                },
            ],
        },
    },
    "gemini-2.5-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.3,
            output_per_1m: 2.5,
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.075,
                },
            ],
        },
    },
    "gemini-2.5-flash-lite" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.1,
            output_per_1m: 0.4,
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.025,
                },
            ],
        },
    },
    "gemini-2.0-pro-exp-02-05" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,
            output_per_1m: 0.0,
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.0,
                },
            ],
        },
    },
    "gemini-2.0-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.1,
            output_per_1m: 0.4,
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.025,
                },
            ],
        },
    },
    "gemini-2.0-flash-lite" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.075,
            output_per_1m: 0.3,
        },
        caching: CachingSupport::None,
    },
    "gemini-1.5-flash" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(128_000),
                    input_per_1m: 0.075,
                    output_per_1m: 0.3,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 0.15,
                    output_per_1m: 0.6,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(128_000),
                    cached_input_per_1m: 0.01875,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.0375,
                },
            ],
        },
    },
    "gemini-1.5-flash-8b" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(128_000),
                    input_per_1m: 0.0375,
                    output_per_1m: 0.15,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 0.075,
                    output_per_1m: 0.3,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(128_000),
                    cached_input_per_1m: 0.01,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.02,
                },
            ],
        },
    },
    "gemini-1.5-pro" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(128_000),
                    input_per_1m: 1.25,
                    output_per_1m: 5.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.5,
                    output_per_1m: 10.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(128_000),
                    cached_input_per_1m: 0.3125,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.625,
                },
            ],
        },
    },

    // Google Gemini 3.x models
    "gemini-3-flash-preview" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.5,
            output_per_1m: 3.0,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.05,
            }],
        },
    },
    "gemini-3.1-pro-preview" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 2.0,
                    output_per_1m: 12.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 4.0,
                    output_per_1m: 18.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.2,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.4,
                },
            ],
        },
    },
    "gemini-3.1-flash-lite" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 1.5,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.025,
            }],
        },
    },
    "gemini-3.5-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.5,
            output_per_1m: 9.0,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.15,
            }],
        },
    },
    "gemini-3.5-flash-lite" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.3,
            output_per_1m: 2.5,
        },
        caching: CachingSupport::None,
    },
    // 3.6/3.7/3.8 Flash: introductory $0.75/$3.75 through 2026-12-31, then $1.50/$7.50
    "gemini-3.6-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.75,
            output_per_1m: 3.75,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.075,
            }],
        },
    },
    "gemini-3.7-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.75,
            output_per_1m: 3.75,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.075,
            }],
        },
    },
    "gemini-3.8-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.75,
            output_per_1m: 3.75,
        },
        caching: CachingSupport::Google {
            tiers: &[CachingTier {
                max_tokens: None,
                cached_input_per_1m: 0.075,
            }],
        },
    },

    // xAI Models
    "x-ai/grok-code-fast-1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 1.50,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.02,
        },
    },
    "x-ai/grok-4.1-fast" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 0.50,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.05,
        },
    },

    // xAI Grok 4.x (tiered at 200K)
    "grok-4.6" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 2.0,
                    output_per_1m: 6.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 4.0,
                    output_per_1m: 12.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.5,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 1.0,
                },
            ],
        },
    },
    "grok-4.5" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 2.0,
                    output_per_1m: 6.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 4.0,
                    output_per_1m: 12.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.3,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.6,
                },
            ],
        },
    },
    "grok-4.3" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 1.25,
                    output_per_1m: 2.5,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.5,
                    output_per_1m: 5.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.2,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.4,
                },
            ],
        },
    },
    "grok-4.20" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 1.25,
                    output_per_1m: 2.5,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.5,
                    output_per_1m: 5.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.2,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.4,
                },
            ],
        },
    },
    "grok-build-0.1" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(200_000),
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 2.0,
                    output_per_1m: 4.0,
                },
            ],
        },
        caching: CachingSupport::Google {
            tiers: &[
                CachingTier {
                    max_tokens: Some(200_000),
                    cached_input_per_1m: 0.2,
                },
                CachingTier {
                    max_tokens: None,
                    cached_input_per_1m: 0.4,
                },
            ],
        },
    },

    // Minimax Models
    "minimax/minimax-m2:free" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.00,
            output_per_1m: 0.00,
        },
        caching: CachingSupport::None,
    },

    // MiniMax M-series (standard tier $0.30/$1.20 up to 512K, then $0.60/$2.40)
    "minimax-m3" => ModelInfo {
        pricing: PricingStructure::Tiered {
            tiers: &[
                PricingTier {
                    max_tokens: Some(512_000),
                    input_per_1m: 0.3,
                    output_per_1m: 1.2,
                },
                PricingTier {
                    max_tokens: None,
                    input_per_1m: 0.6,
                    output_per_1m: 2.4,
                },
            ],
        },
        caching: CachingSupport::None,
    },
    "minimax-m2.5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.3,
            output_per_1m: 1.2,
        },
        caching: CachingSupport::None,
    },

    // DeepSeek V4 (api-docs.deepseek.com; V4-Pro at current 75%-off promotional rate)
    "deepseek-v4-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.14,
            output_per_1m: 0.28,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.0028,
        },
    },
    "deepseek-v4-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.435,
            output_per_1m: 0.87,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.003625,
        },
    },

    // Moonshot Kimi (platform.kimi.ai/docs/pricing)
    "kimi-k3" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.3,
        },
    },
    "kimi-k2.7-code" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.95,
            output_per_1m: 4.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.19,
        },
    },
    "kimi-k2.7-code-highspeed" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.9,
            output_per_1m: 8.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.38,
        },
    },
    "kimi-k2.6" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.95,
            output_per_1m: 4.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.19,
        },
    },
    "kimi-k2.5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.6,
            output_per_1m: 3.0,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.12,
        },
    },

    // Z.ai GLM (docs.z.ai/guides/overview/pricing)
    "glm-5.3" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.4,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.26,
        },
    },
    "glm-5.2" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.4,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.26,
        },
    },
    "glm-5.1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.4,
            output_per_1m: 4.4,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.26,
        },
    },
    "glm-5" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.0,
            output_per_1m: 3.2,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.2,
        },
    },
    "glm-5.3-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.15,
            output_per_1m: 0.5,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.03,
        },
    },
    "glm-4.7" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.6,
            output_per_1m: 2.2,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.11,
        },
    },
    "glm-4.7-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,
            output_per_1m: 0.0,
        },
        caching: CachingSupport::None,
    },

    // Qwen 3.5 / 3.8 (Alibaba Model Studio; cache pricing not published in USD)
    "qwen3.8-max" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 2.0,
            output_per_1m: 6.0,
        },
        caching: CachingSupport::None,
    },
    "qwen3.5-397b-a17b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.6,
            output_per_1m: 3.6,
        },
        caching: CachingSupport::None,
    },
    "qwen3.5-plus" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.4,
            output_per_1m: 2.4,
        },
        caching: CachingSupport::None,
    },
    "qwen3.5-flash" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.1,
            output_per_1m: 0.4,
        },
        caching: CachingSupport::None,
    },

    // Qwen Models
    "coder-model" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.40,
            output_per_1m: 1.20,
        },
        caching: CachingSupport::None,
    },
    "zai-glm-4.6" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.60,
            output_per_1m: 2.20,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.11,
        },
    },
    "qwen-3-235b-a22b-instruct-2507" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.60,
            output_per_1m: 4.80,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 1.60,
        },
    },
    "qwen3-coder:latest" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.15,
            output_per_1m: 0.15,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.15,
        },
    },
    "qwen3-coder:30b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 0.20,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.20,
        },
    },
    "deepseek-coder-v2:latest" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 0.20,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.20,
        },
    },
    "ollama/qwen3-coder:30b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 0.20,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.20,
        },
    },

    // New Qwen 3‑32B (32B dense) price data
    "qwen3-coder:32b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.08,
            output_per_1m: 0.24,
        },
        caching: CachingSupport::None,
    },

     // New Ollama Qwen3‑VL 30B variant – use same 30B pricing
     "ollama/qwen3-vl:30b-a3b-instruct-q4_K_M" => ModelInfo {
         pricing: PricingStructure::Flat {
             input_per_1m: 0.20,
             output_per_1m: 0.20,
         },
         caching: CachingSupport::None,
     },

     // New GPT‑OSS 20B – match o3‑mini price
    "ollama/gpt-oss:20b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.13,
            output_per_1m: 0.13,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.13,
        },
    },

     // Local variants that use the same underlying model pricing
    "ollama-kubuntu-1/qwen3-coder:30b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 0.20,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.20,
        },
    },

    "ollama-kubuntu-1/gpt-oss:20b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.13,
            output_per_1m: 0.13,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.13,
        },
    },
    "gpt-oss:20b" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.13,
            output_per_1m: 0.13,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.13,
        },
    },

    // OpenCode Models
    "opencode-zen" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Free tier
            output_per_1m: 0.0, // Free tier
        },
        caching: CachingSupport::None,
    },
    "big-pickle" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 1.00,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.05,
        },
    },
    "opencode-zen-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.50,
            output_per_1m: 2.00,
        },
        caching: CachingSupport::None,
    },
    "grok-code" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Currently free promotional tier
            output_per_1m: 0.0, // Currently free promotional tier
        },
        caching: CachingSupport::None,
    },
    "grok-code-fast-1" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.20,
            output_per_1m: 1.50,
        },
        caching: CachingSupport::OpenAI {
            cached_input_per_1m: 0.02,
        },
    },
    "grok-code-pro" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 1.00,
            output_per_1m: 4.00,
        },
        caching: CachingSupport::None,
    },
    "openrouter-free" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Free tier
            output_per_1m: 0.0, // Free tier
        },
        caching: CachingSupport::None,
    },
    "openrouter-standard" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.25,
            output_per_1m: 1.00,
        },
        caching: CachingSupport::None,
    },
    "kimi-k2" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.30,
            output_per_1m: 1.20,
        },
        caching: CachingSupport::None,
    },
    "qwen3-coder-free" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Free tier
            output_per_1m: 0.0, // Free tier
        },
        caching: CachingSupport::None,
    },
    "local-llama" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Local models are free
            output_per_1m: 0.0, // Local models are free
        },
        caching: CachingSupport::None,
    },
    "local-codellama" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Local models are free
            output_per_1m: 0.0, // Local models are free
        },
        caching: CachingSupport::None,
    },
    "local-mistral" => ModelInfo {
        pricing: PricingStructure::Flat {
            input_per_1m: 0.0,  // Local models are free
            output_per_1m: 0.0, // Local models are free
        },
        caching: CachingSupport::None,
    },
};

static MODEL_ALIASES: phf::Map<&'static str, &'static str> = phf_map! {
    // OpenAI aliases
    "o4-mini" => "o4-mini",
    "o4-mini-2025-04-16" => "o4-mini",
    "o3" => "o3",
    "o3-2025-04-16" => "o3",
    "o3-pro" => "o3-pro",
    "o3-pro-2025-06-10" => "o3-pro",
    "o3-mini" => "o3-mini",
    "o3-mini-2025-01-31" => "o3-mini",
    "o1" => "o1",
    "o1-2024-12-17" => "o1",
    "o1-preview" => "o1-preview",
    "o1-preview-2024-09-12" => "o1-preview",
    "o1-mini" => "o1-mini",
    "o1-mini-2024-09-12" => "o1-mini",
    "o1-pro" => "o1-pro",
    "o1-pro-2025-03-19" => "o1-pro",
    "gpt-4.1" => "gpt-4.1",
    "gpt-4.1-2025-04-14" => "gpt-4.1",
    "gpt-4o" => "gpt-4o",
    "gpt-4o-2024-11-20" => "gpt-4o",
    "gpt-4o-2024-08-06" => "gpt-4o",
    "gpt-4o-2024-05-13" => "gpt-4o-2024-05-13",
    "gpt-4.1-mini" => "gpt-4.1-mini",
    "gpt-4.1-mini-2025-04-14" => "gpt-4.1-mini",
    "gpt-4.1-nano" => "gpt-4.1-nano",
    "gpt-4.1-nano-2025-04-14" => "gpt-4.1-nano",
    "gpt-4o-mini" => "gpt-4o-mini",
    "gpt-4o-mini-2024-07-18" => "gpt-4o-mini",
    "codex-mini-latest" => "codex-mini-latest",
    "gpt-4-turbo" => "gpt-4-turbo",
    "gpt-4-turbo-2024-04-09" => "gpt-4-turbo",
    "gpt-5" => "gpt-5",
    "gpt-5-codex" => "gpt-5",
    "gpt-5-2025-08-07" => "gpt-5",
    "gpt-5-mini" => "gpt-5-mini",
    "gpt-5-mini-2025-08-07" => "gpt-5-mini",
    "gpt-5-nano" => "gpt-5-nano",
    "gpt-5-nano-2025-08-07" => "gpt-5-nano",
    "gpt-5-codex-mini" => "gpt-5-codex-mini",
    "gpt-5.2-codex" => "gpt-5.2-codex",

    "gpt-6-astra" => "gpt-6-astra",
    "gpt-6" => "gpt-6-astra",
    "gpt-5.6-sol" => "gpt-5.6-sol",
    "gpt-5.6" => "gpt-5.6-sol",
    "gpt-5.6-terra" => "gpt-5.6-terra",
    "gpt-5.6-luna" => "gpt-5.6-luna",
    "gpt-5.5" => "gpt-5.5",
    "gpt-5.5-pro" => "gpt-5.5-pro",
    "gpt-5.4" => "gpt-5.4",
    "gpt-5.4-mini" => "gpt-5.4-mini",
    "gpt-5.4-nano" => "gpt-5.4-nano",
    "gpt-5.3-codex" => "gpt-5.3-codex",
    "gpt-5.3-codex-spark" => "gpt-5.3-codex",
    "codex-auto-review" => "gpt-5.3-codex",
    "gpt-5.2" => "gpt-5.2",
    "gpt-5.1" => "gpt-5.1",
    "gpt-5.1-codex" => "gpt-5.1-codex",
    "gpt-5.1-codex-mini" => "gpt-5.1-codex-mini",
    "gpt-5.1-codex-max" => "gpt-5.1-codex-max",

    // Anthropic aliases
    "claude-opus-4" => "claude-opus-4",
    "claude-opus-4-20250514" => "claude-opus-4",
    "claude-opus-4-0" => "claude-opus-4",
    "claude-opus-4.1" => "claude-opus-4-1",
    "claude-opus-4-1-20250805" => "claude-opus-4-1",
    "claude-sonnet-4" => "claude-sonnet-4",
    "claude-sonnet-4-20250514" => "claude-sonnet-4",
    "claude-sonnet-4-0" => "claude-sonnet-4",
    "claude-sonnet-4.5" => "claude-sonnet-4-5",
    "claude-sonnet-4-5" => "claude-sonnet-4-5",
    "claude-sonnet-4-5-20250929" => "claude-sonnet-4-5",
    "claude-3-7-sonnet" => "claude-3-7-sonnet",
    "claude-3-7-sonnet-20250219" => "claude-3-7-sonnet",
    "claude-3-7-sonnet-latest" => "claude-3-7-sonnet",
    "claude-3-5-sonnet" => "claude-3-5-sonnet",
    "claude-3-5-sonnet-20241022" => "claude-3-5-sonnet",
    "claude-3-5-sonnet-latest" => "claude-3-5-sonnet",
    "claude-3-5-sonnet-20240620" => "claude-3-5-sonnet",
    "claude-3-5-haiku" => "claude-3-5-haiku",
    "claude-3-5-haiku-20241022" => "claude-3-5-haiku",
    "claude-3-5-haiku-latest" => "claude-3-5-haiku",
    "claude-haiku-4-5" => "claude-haiku-4-5",
    "claude-haiku-4.5" => "claude-haiku-4-5",
    "claude-haiku-4-5-20251001" => "claude-haiku-4-5",
    "claude-3-opus" => "claude-3-opus",
    "claude-3-opus-20240229" => "claude-3-opus",
    "claude-3-haiku" => "claude-3-haiku",
    "claude-3-haiku-20240307" => "claude-3-haiku",

    "claude-fable-5-1" => "claude-fable-5-1",
    "claude-fable-5.1" => "claude-fable-5-1",
    "fable" => "claude-fable-5-1",
    "claude-mythos-5-1" => "claude-mythos-5-1",
    "claude-mythos-5.1" => "claude-mythos-5-1",
    "mythos" => "claude-mythos-5-1",
    "claude-fable-5" => "claude-fable-5",
    "claude-mythos-5" => "claude-mythos-5",
    "claude-opus-5" => "claude-opus-5",
    "claude-opus-5-0" => "claude-opus-5",
    "opus" => "claude-opus-5",
    "claude-opus-4-8" => "claude-opus-4-8",
    "claude-opus-4.8" => "claude-opus-4-8",
    "claude-opus-4-7" => "claude-opus-4-7",
    "claude-opus-4.7" => "claude-opus-4-7",
    "claude-opus-4-6" => "claude-opus-4-6",
    "claude-opus-4.6" => "claude-opus-4-6",
    "claude-opus-4-5" => "claude-opus-4-5-20251101",
    "claude-opus-4.5" => "claude-opus-4-5-20251101",
    "claude-sonnet-5" => "claude-sonnet-5",
    "claude-sonnet-5-0" => "claude-sonnet-5",
    "sonnet" => "claude-sonnet-5",
    "claude-sonnet-4-6" => "claude-sonnet-4-6",
    "claude-sonnet-4.6" => "claude-sonnet-4-6",
    "haiku" => "claude-haiku-4-5",

    // Google aliases
    "gemini-3-pro-preview-11-2025" => "gemini-3-pro-preview-11-2025",
    "gemini-2.5-pro" => "gemini-2.5-pro",
    "gemini-2.5-pro-preview-06-05" => "gemini-2.5-pro",
    "gemini-2.5-pro-preview-05-06" => "gemini-2.5-pro",
    "gemini-2.5-pro-preview-03-25" => "gemini-2.5-pro",
    "gemini-2.5-flash" => "gemini-2.5-flash",
    "gemini-2.5-flash-preview-05-20" => "gemini-2.5-flash",
    "gemini-2.5-flash-preview-04-17" => "gemini-2.5-flash",
    "gemini-2.5-flash-lite" => "gemini-2.5-flash-lite",
    "gemini-2.5-flash-lite-06-17" => "gemini-2.5-flash-lite",
    "gemini-2.0-pro-exp-02-05" => "gemini-2.0-pro-exp-02-05",
    "gemini-exp-1206" => "gemini-2.0-pro-exp-02-05",
    "gemini-2.0-flash" => "gemini-2.0-flash",
    "gemini-2.0-flash-001" => "gemini-2.0-flash",
    "gemini-2.0-flash-exp" => "gemini-2.0-flash",
    "gemini-2.0-flash-lite" => "gemini-2.0-flash-lite",
    "gemini-2.0-flash-lite-001" => "gemini-2.0-flash-lite",
    "gemini-1.5-flash" => "gemini-1.5-flash",
    "gemini-1.5-flash-latest" => "gemini-1.5-flash",
    "gemini-1.5-flash-001" => "gemini-1.5-flash",
    "gemini-1.5-flash-002" => "gemini-1.5-flash",
    "gemini-1.5-flash-8b" => "gemini-1.5-flash-8b",
    "gemini-1.5-flash-8b-latest" => "gemini-1.5-flash-8b",
    "gemini-1.5-flash-8b-001" => "gemini-1.5-flash-8b",
    "gemini-1.5-flash-8b-exp-0924" => "gemini-1.5-flash-8b",
    "gemini-1.5-flash-8b-exp-0827" => "gemini-1.5-flash-8b",
    "gemini-1.5-pro" => "gemini-1.5-pro",
    "gemini-1.5-pro-latest" => "gemini-1.5-pro",
    "gemini-1.5-pro-001" => "gemini-1.5-pro",
    "gemini-1.5-pro-002" => "gemini-1.5-pro",
    "gemini-1.5-pro-exp-0827" => "gemini-1.5-pro",
    "gemini-1.5-pro-exp-0801" => "gemini-1.5-pro",

    "gemini-3-flash-preview" => "gemini-3-flash-preview",
    "gemini-3-flash" => "gemini-3-flash-preview",
    "gemini-3.1-pro-preview" => "gemini-3.1-pro-preview",
    "gemini-3.1-pro" => "gemini-3.1-pro-preview",
    "gemini-3.1-flash-lite" => "gemini-3.1-flash-lite",
    "gemini-3.5-flash" => "gemini-3.5-flash",
    "gemini-3.5-flash-lite" => "gemini-3.5-flash-lite",
    "gemini-3.6-flash" => "gemini-3.6-flash",
    "gemini-3.7-flash" => "gemini-3.7-flash",
    "gemini-3.8-flash" => "gemini-3.8-flash",

    // xAI aliases
    "grok-4.6" => "grok-4.6",
    "x-ai/grok-4.6" => "grok-4.6",
    "grok-4.5" => "grok-4.5",
    "x-ai/grok-4.5" => "grok-4.5",
    "grok-4.3" => "grok-4.3",
    "x-ai/grok-4.3" => "grok-4.3",
    "grok-4.20" => "grok-4.20",
    "x-ai/grok-4.20" => "grok-4.20",
    "grok-4.20-0309-reasoning" => "grok-4.20",
    "grok-4.20-0309-non-reasoning" => "grok-4.20",
    "grok-4.20-multi-agent-0309" => "grok-4.20",
    "grok-build-0.1" => "grok-build-0.1",
    "x-ai/grok-build-0.1" => "grok-build-0.1",
    "grok-build" => "grok-build-0.1",

    // Qwen aliases
    "qwen-3-235b-a22b-instruct" => "qwen-3-235b-a22b-instruct-2507",
    "qwen-3-235b" => "qwen-3-235b-a22b-instruct-2507",
    "glm-4.6" => "zai-glm-4.6",
    "zai-glm-4.6" => "zai-glm-4.6",
    "deepseek-coder-v2" => "deepseek-coder-v2:latest",
    "deepseek-coder" => "deepseek-coder-v2:latest",

    "qwen3.8-max" => "qwen3.8-max",
    "qwen/qwen3.8-max" => "qwen3.8-max",
    "qwen3.5-397b-a17b" => "qwen3.5-397b-a17b",
    "qwen/qwen3.5-397b-a17b" => "qwen3.5-397b-a17b",
    "qwen3.5-397b" => "qwen3.5-397b-a17b",
    "qwen3.5-plus" => "qwen3.5-plus",
    "qwen/qwen3.5-plus" => "qwen3.5-plus",
    "qwen3.5-plus-20260420" => "qwen3.5-plus",
    "qwen/qwen3.5-plus-20260420" => "qwen3.5-plus",
    "qwen3.5-flash" => "qwen3.5-flash",
    "qwen/qwen3.5-flash" => "qwen3.5-flash",

    // Z.ai GLM aliases
    "z-ai/glm-4.6" => "zai-glm-4.6",
    "glm-5.3" => "glm-5.3",
    "z-ai/glm-5.3" => "glm-5.3",
    "zai-glm-5.3" => "glm-5.3",
    "glm-5.2" => "glm-5.2",
    "z-ai/glm-5.2" => "glm-5.2",
    "zai-glm-5.2" => "glm-5.2",
    "glm-5.1" => "glm-5.1",
    "z-ai/glm-5.1" => "glm-5.1",
    "zai-glm-5.1" => "glm-5.1",
    "glm-5" => "glm-5",
    "z-ai/glm-5" => "glm-5",
    "zai-glm-5" => "glm-5",
    "glm-5.3-flash" => "glm-5.3-flash",
    "z-ai/glm-5.3-flash" => "glm-5.3-flash",
    "glm-4.7" => "glm-4.7",
    "z-ai/glm-4.7" => "glm-4.7",
    "zai-glm-4.7" => "glm-4.7",
    "glm-4.7-flash" => "glm-4.7-flash",
    "z-ai/glm-4.7-flash" => "glm-4.7-flash",

    // DeepSeek aliases
    "deepseek-v4-flash" => "deepseek-v4-flash",
    "deepseek/deepseek-v4-flash" => "deepseek-v4-flash",
    "deepseek-chat" => "deepseek-v4-flash",
    "deepseek-reasoner" => "deepseek-v4-flash",
    "deepseek-v4-pro" => "deepseek-v4-pro",
    "deepseek/deepseek-v4-pro" => "deepseek-v4-pro",

    // Moonshot Kimi aliases
    "kimi-k3" => "kimi-k3",
    "moonshotai/kimi-k3" => "kimi-k3",
    "moonshot/kimi-k3" => "kimi-k3",
    "kimi-k2.7-code" => "kimi-k2.7-code",
    "moonshotai/kimi-k2.7-code" => "kimi-k2.7-code",
    "kimi-k2.7-code-highspeed" => "kimi-k2.7-code-highspeed",
    "kimi-k2.6" => "kimi-k2.6",
    "moonshotai/kimi-k2.6" => "kimi-k2.6",
    "kimi-k2.5" => "kimi-k2.5",
    "moonshotai/kimi-k2.5" => "kimi-k2.5",

    // MiniMax aliases
    "minimax-m3" => "minimax-m3",
    "minimax/minimax-m3" => "minimax-m3",
    "MiniMax-M3" => "minimax-m3",
    "minimax-m2.5" => "minimax-m2.5",
    "minimax/minimax-m2.5" => "minimax-m2.5",
    "MiniMax-M2.5" => "minimax-m2.5",

    // OpenCode aliases
    "zen" => "opencode-zen",
    "zen-free" => "opencode-zen",
    "zen-pro" => "opencode-zen-pro",
    "opencode" => "opencode-zen",  // Default OpenCode model
    "big-pickle" => "big-pickle",
    "bigpickle" => "big-pickle",
    "pickle" => "big-pickle",
    "grok" => "grok-code",
    "grok-free" => "grok-code",
    "grok-code-fast-1" => "grok-code-fast-1",
    "grok-fast" => "grok-code-fast-1",
    "grok-4.1-fast" => "x-ai/grok-4.1-fast",
    "grok-4-1-fast" => "x-ai/grok-4.1-fast",
    "xai-grok" => "grok-code",
    "xai-grok-fast" => "grok-code-fast-1",
    "xai-grok-4.1-fast" => "x-ai/grok-4.1-fast",
    "openrouter" => "openrouter-standard",
    "openrouter-free-tier" => "openrouter-free",
    "kimi" => "kimi-k2",
    "moonshot-k2" => "kimi-k2",
    "qwen3-free" => "qwen3-coder-free",
    "llama-local" => "local-llama",
    "codellama-local" => "local-codellama",
    "mistral-local" => "local-mistral",
    "ollama" => "local-llama",  // Common local model alias
    "lm-studio" => "local-llama",  // Common local model alias

    // Kiro CLI / Amazon Q aliases
    "auto" => "claude-sonnet-4",  // Default for auto mode
};

/// Get model info by any valid name (canonical or alias)
pub fn get_model_info(model_name: &str) -> Option<&ModelInfo> {
    // First try direct lookup in model index
    if let Some(model_info) = MODEL_INDEX.get(model_name) {
        return Some(model_info);
    }

    // Then try alias lookup
    if let Some(&canonical_name) = MODEL_ALIASES.get(model_name) {
        return MODEL_INDEX.get(canonical_name);
    }

    None
}

/// Calculate cost for input tokens using the model's pricing structure
pub fn calculate_input_cost(model_name: &str, input_tokens: u64) -> f64 {
    match get_model_info(model_name) {
        Some(model_info) => match &model_info.pricing {
            PricingStructure::Flat { input_per_1m, .. } => {
                (input_tokens as f64 / 1_000_000.0) * input_per_1m
            }
            PricingStructure::Tiered { tiers } => calculate_tiered_cost(input_tokens, tiers, true),
        },
        None => {
            warn_once(format!(
                "WARNING: Unknown model: {model_name}. Defaulting to $0."
            ));
            (input_tokens as f64 / 1_000_000.0) * 0.0 // $0 per 1M tokens fallback
        }
    }
}

/// Calculate cost for output tokens using the model's pricing structure
pub fn calculate_output_cost(model_name: &str, output_tokens: u64) -> f64 {
    match get_model_info(model_name) {
        Some(model_info) => match &model_info.pricing {
            PricingStructure::Flat { output_per_1m, .. } => {
                (output_tokens as f64 / 1_000_000.0) * output_per_1m
            }
            PricingStructure::Tiered { tiers } => {
                calculate_tiered_cost(output_tokens, tiers, false)
            }
        },
        None => {
            warn_once(format!(
                "WARNING: Unknown model: {model_name}. Defaulting to $0."
            ));
            (output_tokens as f64 / 1_000_000.0) * 0.0 // $0 per 1M tokens fallback
        }
    }
}

/// Calculate cost for cached tokens
pub fn calculate_cache_cost(
    model_name: &str,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    match get_model_info(model_name) {
        Some(model_info) => {
            match &model_info.caching {
                CachingSupport::None => 0.0,
                CachingSupport::OpenAI {
                    cached_input_per_1m,
                } => {
                    // OpenAI only has cached input cost, no creation cost
                    (cache_read_tokens as f64 / 1_000_000.0) * cached_input_per_1m
                }
                CachingSupport::Anthropic {
                    cache_write_per_1m,
                    cache_read_per_1m,
                } => {
                    let creation_cost =
                        (cache_creation_tokens as f64 / 1_000_000.0) * cache_write_per_1m;
                    let read_cost = (cache_read_tokens as f64 / 1_000_000.0) * cache_read_per_1m;
                    creation_cost + read_cost
                }
                CachingSupport::Google { tiers } => {
                    // Google only has read cost, calculate based on tiers
                    calculate_tiered_cache_cost(cache_read_tokens, tiers)
                }
            }
        }
        None => {
            warn_once(format!(
                "WARNING: Unknown model: {model_name}. Defaulting to $0."
            ));
            (cache_read_tokens as f64 / 1_000_000.0) * 0.0 // $0 per 1M tokens fallback
        }
    }
}

/// Calculate total cost for a model usage
pub fn calculate_total_cost(
    model_name: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    let input_cost = calculate_input_cost(model_name, input_tokens);
    let output_cost = calculate_output_cost(model_name, output_tokens);
    let cache_cost = calculate_cache_cost(model_name, cache_creation_tokens, cache_read_tokens);

    input_cost + output_cost + cache_cost
}

fn calculate_tiered_cost(tokens: u64, tiers: &[PricingTier], is_input: bool) -> f64 {
    let mut total_cost = 0.0;
    let mut remaining_tokens = tokens;

    for tier in tiers {
        if remaining_tokens == 0 {
            break;
        }

        let tier_limit = tier.max_tokens.unwrap_or(u64::MAX);
        let tokens_in_tier = remaining_tokens.min(tier_limit);

        let rate = if is_input {
            tier.input_per_1m
        } else {
            tier.output_per_1m
        };
        total_cost += (tokens_in_tier as f64 / 1_000_000.0) * rate;

        remaining_tokens = remaining_tokens.saturating_sub(tokens_in_tier);
    }

    total_cost
}

fn calculate_tiered_cache_cost(tokens: u64, tiers: &[CachingTier]) -> f64 {
    let mut total_cost = 0.0;
    let mut remaining_tokens = tokens;

    for tier in tiers {
        if remaining_tokens == 0 {
            break;
        }

        let tier_limit = tier.max_tokens.unwrap_or(u64::MAX);
        let tokens_in_tier = remaining_tokens.min(tier_limit);

        total_cost += (tokens_in_tier as f64 / 1_000_000.0) * tier.cached_input_per_1m;

        remaining_tokens = remaining_tokens.saturating_sub(tokens_in_tier);
    }

    total_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontier_2026_models_resolve() {
        for name in [
            // Anthropic
            "claude-fable-5-1",
            "claude-fable-5",
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-6",
            "claude-sonnet-5",
            "claude-sonnet-4-6",
            "claude-opus-4-5",
            "opus",
            "sonnet",
            "haiku",
            "fable",
            // OpenAI
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.3-codex",
            "gpt-5.2",
            "gpt-5.1",
            "codex-auto-review",
            // Google
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "gemini-3.5-flash",
            "gemini-3.8-flash",
            // xAI
            "grok-4.6",
            "x-ai/grok-4.6",
            "grok-4.20-0309-reasoning",
            "grok-build-0.1",
            // Others
            "deepseek-v4-flash",
            "deepseek-chat",
            "kimi-k3",
            "glm-5.2",
            "z-ai/glm-5.2",
            "minimax/minimax-m3",
            "qwen3.5-plus",
            "qwen/qwen3.5-plus-20260420",
        ] {
            assert!(
                get_model_info(name).is_some(),
                "{name} should resolve to a priced model"
            );
        }
    }

    #[test]
    fn fable_5_1_cache_read_is_quarter_dollar() {
        // 1M cache-read tokens on Fable 5.1 = $0.25; on Fable 5 = $1.00
        assert!((calculate_cache_cost("claude-fable-5-1", 0, 1_000_000) - 0.25).abs() < 1e-9);
        assert!((calculate_cache_cost("claude-fable-5", 0, 1_000_000) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn grok_4_6_uses_higher_tier_past_200k() {
        // Existing tier math is progressive: first 200K at $2/M, remainder at $4/M.
        assert!((calculate_input_cost("grok-4.6", 100_000) - 0.2).abs() < 1e-9);
        assert!((calculate_input_cost("grok-4.6", 300_000) - 0.8).abs() < 1e-9);
    }
}

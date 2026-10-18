// Library interface for splitrail-dashboard
// This allows benchmarking and testing of internal modules

pub mod advisor;
pub mod analyzer;
pub mod analyzers;
pub mod config;
pub mod cursor_limits;
pub mod droid_limits;
pub mod incremental;
pub mod kiro_limits;
pub mod models;
pub mod mqtt;
pub mod plan_limits;
pub mod price_feed;
pub mod records;
pub mod rollup;
pub mod self_update;
pub mod stamp_cache;
pub mod status_file;
pub mod types;
pub mod utils;
pub mod warp;
pub mod warp_limits;

// Re-export commonly used types for convenience
pub use analyzer::{Analyzer, AnalyzerRegistry};
pub use types::{AgenticCodingToolStats, ConversationMessage, MultiAnalyzerStats};

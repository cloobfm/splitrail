// Library interface for splitrail-dashboard
// This allows benchmarking and testing of internal modules

pub mod analyzer;
pub mod analyzers;
pub mod config;
pub mod incremental;
pub mod models;
pub mod types;
pub mod utils;
pub mod warp;

// Re-export commonly used types for convenience
pub use analyzer::{Analyzer, AnalyzerRegistry};
pub use types::{AgenticCodingToolStats, ConversationMessage, MultiAnalyzerStats};

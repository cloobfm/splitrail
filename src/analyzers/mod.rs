pub mod amazon_q;
pub mod antigravity;
pub mod antigravity_store;
pub mod claude_code;
pub mod cline;
pub mod codex_cli;
pub mod copilot;
pub mod copilot_cli;
pub mod cursor_agent;
pub mod droid_cli;
#[allow(dead_code)] // Retired 2026-09; kept for re-enablement. See docs/GEMINI_CLI_DEPRECATION.md.
pub mod gemini_cli;
pub mod kilo_code;
pub mod kiro_cli;
pub mod kiro_sessions;
pub mod ollama;
pub mod opencode;
pub mod pi_agent;
pub mod qwen_code;
pub mod roo_code;
pub mod warp_dev;

pub use antigravity::AntigravityAnalyzer;
pub use claude_code::ClaudeCodeAnalyzer;
pub use cline::ClineAnalyzer;
pub use codex_cli::CodexCliAnalyzer;
pub use copilot_cli::CopilotCliAnalyzer;
pub use cursor_agent::CursorAgentAnalyzer;
// pub use copilot::CopilotAnalyzer; // Temporarily disabled
pub use droid_cli::DroidCliAnalyzer;
#[allow(unused_imports)]
pub use gemini_cli::GeminiCliAnalyzer;
pub use kilo_code::KiloCodeAnalyzer;
pub use kiro_cli::KiroCliAnalyzer;
pub use ollama::OllamaAnalyzer;
pub use opencode::OpenCodeAnalyzer;
pub use pi_agent::PiAgentAnalyzer;
pub use qwen_code::QwenCodeAnalyzer;
pub use warp_dev::WarpDevAnalyzer;
// pub use roo_code::RooCodeAnalyzer; // Temporarily disabled

#[cfg(test)]
pub mod tests;

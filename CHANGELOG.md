# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0-dash-0.8] - 2025-11-29

### Added
- **Droid CLI (Factory AI) support**: Complete analyzer for Factory AI's Droid CLI usage data
  - Comprehensive token tracking including input, output, cache creation, cache read, and thinking tokens
  - Session metadata extraction (working directory, duration, autonomy mode, reasoning effort)
  - Project context detection from working directories
  - Cost calculation using model-specific pricing for Claude Opus 4.5, Sonnet 4, GPT-5, and more
  - Full schema documentation with JSON validation files and sample data
  - Integration with Splitrail TUI and dashboard alongside existing AI tools
- **Model pricing support**: Added pricing for Claude Opus 4.5 (`claude-opus-4-5-20251101`) with Anthropic-style cache pricing
- **Data source coverage**: Droid CLI sessions stored in `~/.factory/sessions/*/` with JSONL format for conversations and JSON for settings

### Changed
- Enhanced application enum with `DroidCli` variant for proper categorization
- Updated notifications module to display "Droid" as the friendly name for Droid CLI data
- Extended analyzer registry to include Droid CLI analyzer in the default tool set

## [2.0.0-dash-0.7] - 2025-11-24

### Added
- Dual stacked bar charts for tokens/day with separate scales for total and output tokens
- Braille character rendering (⢀⢠⢰⢸⣸⣼⣾⣿) for smoother chart visualization
- Phase 2 CPU optimizations: batched file watching to reduce analyzer reloads (20-30% reduction during high file activity)
- Phase 1 CPU optimizations: adaptive polling with dynamic rates based on user activity (50-75% idle CPU reduction)
- Lazy clock updates and optional clock disable via SPLITRAIL_DISABLE_CLOCK environment variable
- xAI Grok 4.1 Fast model pricing and aliases
- CPU measurement scripts and benchmark guide
- Incremental aggregation infrastructure for future optimizations

### Changed
- Improved tokens/day chart with Y-axis scale, better contrast, and fixed width
- Split tokens/day chart into dual displays: total tokens/day (blue) and output tokens/day (green)
- Updated internal pricing for qwen3 and gpt-oss models
- Enhanced chart X-axis with 7-day interval markers and date labels
- Switched from block characters to smooth braille characters for chart rendering
- Reduced Y-axis width from 4 to 3 characters for more horizontal space
- Added Min/Max/Avg statistics for each chart
- Implemented batched processing with 500ms window for file events
- Optimized polling rates: 1000ms when idle, 250ms when active

### Fixed
- Chart bar alignment and cleaned compiler warnings
- UTF-8 panic in verbose mode message truncation
- Streak calculation to show exactly 30 days (not 31)
- Fixed indentation errors in models.rs pricing definitions

## [2.0.0-dash-0.6] - 2025-11-22

### Added
- Waiting-for-input notification module with Slack webhook delivery and TUI status indicators (countdown + last-send)

### Changed
- Notification delivery now sends only one alert per analyzer per check and ignores long-stale sessions (over-threshold by >15 minutes)

## [2.0.0-dash-0.5] - 2025-11-22

### Fixed
- OpenCode analyzer now properly displays actual conversation content for both user and assistant messages
- Fixed OpenCode schema to include summary.body field for message content
- Fixed timezone conversion for OpenCode daily stats (UTC to local time)
- OpenCode messages now read from part files to show real conversation text instead of action descriptions
- OpenCode role detection now properly distinguishes between user and assistant messages
- Live activity feed now shows meaningful OpenCode conversations matching other CLI patterns

## [2.0.0-dash-0.4] - 2025-11-20

### Fixed
- Health bar visualization now depletes from left to right (fills right to left) for more intuitive display
- Updated braille partial characters to fill from right side first
- Maintains 104-dot granularity (13 chars × 8 dots) for smooth percentage representation

## [2.0.0] - 2025-11-10

- Add support for Cline, Kilo Code, Roo Code (Cline forks) and Qwen Code (Gemini CLI fork) (#25 and #26) - @bl-ue
- Various fixes and improvements to Splitrail Cloud

## [1.2.0] - 2025-10-27

- Correct Claude Code token aggregation for split messages (#23) - @bl-ue

## [1.1.3] - 2025-10-24

- Add support for Claude Haiku 4.5 to the Claude Code analyzer (#21) - @bl-ue

## [1.1.2] - 2025-10-01

- Add support for Claude Sonnet 4.5 to the Claude Code analyzer
- Ignore the "file-history-snapshot" entry added in Claude Code 2.0.0

## [1.1.1] - 2025-09-18

- Fix Codex CLI (#16) - @bl-ue

## [1.1.0] - 2025-09-18

### Added
- Enhanced Codex CLI support with updated analyzer functionality
- Better formatting and lint compliance

## [1.0.1] - 2024-09-14

### Added
- Download link in README for binary releases

### Changed
- Rearranged README structure for better readability
- Improved documentation and project presentation

### Fixed
- Various formatting and lint error corrections

## [1.0.0] - 2025-08-09

### Added

- **Initial stable release** of Splitrail
- Real-time automatic uploading to Splitrail Cloud
- Comprehensive multi-tool support:
  - Claude Code
  - Gemini CLI
  - Codex CLI
- Rich Terminal User Interface (TUI) using ratatui
- Advanced cost calculation and token usage analytics
- File operation tracking (read/write/edit operations with byte/line counts)
- Comprehensive model support:
  - Claude models (Sonnet 4, Opus 4, Opus 4.1)
  - GPT models (GPT-5 series, GPT-4 series)
  - Gemini models (2.5-pro, 2.5-flash, legacy 1.5 series)
  - Codex CLI models (o3, o1 series, gpt-4.1 series)
- Configuration management with TOML-based config
- Splitrail Cloud integration with API token authentication
- Deduplication logic to prevent duplicate entries
- Real-time file watching capabilities
- Parallel processing for performance

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0-dash-0.15] - 2026-09-05

### Changed
- Analyzer stats are shared, not copied. `MultiAnalyzerStats` now holds `Arc<AgenticCodingToolStats>` per analyzer, so a reload replaces one entry and the watch channel, TUI, notifier, and uploader all reference the same allocation. Previously every reload cloned all ~118K retained messages three to six times (manager copy, channel copy, TUI copy, notifier copy).
- The per-CLI health column compared each message's date to today by formatting both to strings. It now compares dates directly. That formatting ran for every message on every redraw and was the hottest frame in the render profile.
- The incremental cache pre-sizes the merged message vector instead of growing it by doubling.

### Performance
- Same isolated benchmark as 0.14 (copy of real sessions, one assistant line appended every 2 s, release build, steady state): CPU 11.7% average with 30% peaks before, 7.7% average with 25% peaks after. Resident memory 863 MB before, 601 MB after; large heap allocations 742 MB before, 222 MB after.

## [2.0.0-dash-0.14] - 2026-09-05

### Changed
- Claude Code sessions are parsed incrementally. A new per-file cache (`src/incremental.rs`) remembers each file's size, mtime, parsed byte offset, and parser state. On reload, unchanged files are not opened, grown files are parsed from the previous offset with the project label carried over, and only fully newline-terminated lines are consumed so a half-written line is never parsed twice or half-parsed. Shrunk or rewritten files fall back to a full parse; files that disappear are dropped.
- `parse_jsonl_file` is now a thin wrapper over a resumable `parse_jsonl_chunk` that reports bytes consumed.

### Performance
- Reloading no longer re-parses the entire 1.2 GB Claude Code corpus on every file write. Isolated benchmark with a copy of the real sessions and one assistant line appended every 2 s: steady-state CPU 198.7% average with 524% peaks before, 11.7% average with 30% peaks after.
- Known trade-off: the cache keeps parsed messages resident, so RSS rose from about 670 MB to about 860 MB in that benchmark. Removing the duplicate copies is tracked separately (BZL-8).

## [2.0.0-dash-0.13] - 2026-09-05

### Changed
- Codex CLI is only re-parsed when one of its files actually changed. The 5-second poll now compares a fingerprint of every source file's path, size, and mtime (one `stat` per file, no reads) instead of unconditionally re-parsing all sessions on disk.

### Performance
- Steady-state CPU in an isolated environment with only Codex data: 17.6% average with 179% peaks before, 1.4% average with 2.8% peaks after

## [2.0.0-dash-0.12] - 2026-09-05

### Fixed
- Test suite is green again: 64 unit tests pass (was 55 pass, 7 fail, 1 ignored) and the integration target compiles and passes 7 tests. The failures were stale expectations behind intentional changes (tool-result turns dropped, Droid session-total message, aggregate gap-filling and local-date bucketing), plus a HOME-override race between integration tests, now serialized behind a lock with a guard that restores HOME on drop.

## [2.0.0-dash-0.11] - 2026-09-05

### Fixed
- Ctrl-C now quits the TUI. Raw mode delivered it as a key event that nothing handled, so the only way out was to kill the process.
- The terminal is restored on every exit path: normal quit, error, panic, and SIGTERM/SIGHUP/SIGINT. A drop guard plus a panic hook disable raw mode, mouse capture, and the alternate screen and show the cursor. Killing the app no longer leaves the shell printing `35;col;rowM` mouse-tracking sequences on every mouse move.
- If a signal arrives while the event loop is blocked in a long reload, the terminal is restored directly after 3 seconds and the process exits

## [2.0.0-dash-0.10] - 2026-09-05

### Fixed
- Claude Code parser no longer rejects lines with unfamiliar record types (`attachment`, `cost-state`, `last-prompt`, `mode`, `permission-mode`, `worktree-state`, and others) or unfamiliar content block types (`tool_reference` inside ToolSearch results, `fallback`). Both enums gained a `#[serde(other)]` catch-all.
- Removes roughly 266,000 "Skipping invalid entry" warnings that permanently occupied the TUI warnings panel during active sessions
- Token usage on assistant messages that contain an unmodeled block is now counted instead of discarded

## [2.0.0-dash-0.9] - 2026-09-05

### Added
- 2026 frontier model pricing: 48 models and 133 aliases covering Claude 4.6 through Fable 5.1 and Mythos 5.1, GPT-5.1 through GPT-6 Astra (including the GPT-5.6 Sol/Terra/Luna family and GPT-5.3-codex), Gemini 3 Flash through 3.8 Flash and 3.1 Pro, Grok 4.20 through 4.6 and Grok Build, DeepSeek V4, Kimi K3 and K2.x, GLM 5.x and 4.7, MiniMax M3 and M2.5, Qwen 3.8 Max and the 3.5 family
- Short aliases `opus`, `sonnet`, `haiku`, `fable`, and the missing `claude-opus-4-5` alias; `codex-auto-review` priced as gpt-5.3-codex
- Unit tests for model resolution and Fable 5.1 cache-read pricing

### Fixed
- Every model released since late 2025 was silently priced at $0

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

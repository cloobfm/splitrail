# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Splitrail is a comprehensive agentic AI coding tool usage analyzer written in Rust that provides detailed analytics for Claude Code, Codex CLI, Copilot, Gemini CLI, and WARP usage. It features a rich TUI (Terminal User Interface), automatic data upload to Splitrail Cloud, and extensive usage statistics including token counts, costs, file operations, tool usage, and productivity metrics.

## Development Commands

### Building and Testing
- `cargo check` - Check code compilation without building
- `cargo build` - Build the project in debug mode
- `cargo build --release` - Build optimized release version
- `cargo run` - Run with default behavior (show stats + optional auto-upload)
- `cargo run -- upload` - Manually upload stats to Splitrail Cloud
- `cargo run -- config <subcommand>` - Manage configuration

### Available Commands
- `splitrail` - Show stats in TUI with real-time watching and auto-upload when changes are detected (if auto-upload is enabled)
- `splitrail upload` - Manually upload stats to Splitrail Cloud
- `splitrail config init` - Create default configuration file
- `splitrail config show` - Display current configuration
- `splitrail config set <key> <value>` - Set configuration values
- `splitrail help` - Show help information

## Architecture

### Core Modules

1. **Main Module** (`src/main.rs`): Command-line interface with subcommand routing
2. **Analyzer Framework** (`src/analyzer.rs`): Trait-based analyzer architecture for multiple AI tools
3. **Claude Code Analyzer** (`src/analyzers/claude_code.rs`): Analysis engine for Claude Code data
4. **Codex CLI Analyzer** (`src/analyzers/codex_cli.rs`): Analysis engine for Codex CLI data
5. **Gemini CLI Analyzer** (`src/analyzers/gemini_cli.rs`): Retired 2026-09, no longer registered (see `docs/GEMINI_CLI_DEPRECATION.md`)
6. **GitHub Copilot Analyzer** (`src/analyzers/copilot.rs`): Analysis engine for GitHub Copilot Chat data
7. **Cline Analyzer** (`src/analyzers/cline.rs`): Analysis engine for Cline data
8. **Roo Code Analyzer** (`src/analyzers/roo_code.rs`): Analysis engine for Roo Code data
9. **Kilo Code Analyzer** (`src/analyzers/kilo_code.rs`): Analysis engine for Kilo Code data
10. **Qwen Code Analyzer** (`src/analyzers/qwen_code.rs`): Analysis engine for Qwen Code data
11. **WARP Analyzer** (`src/analyzers/warp_dev.rs`): Analysis engine for WARP terminal telemetry and GraphQL conversation data
12. **TUI Module** (`src/tui.rs`): Rich terminal user interface using ratatui
13. **Upload Module** (`src/upload.rs`): HTTP client for Splitrail Cloud integration
14. **Config Module** (`src/config.rs`): Configuration file management
15. **Types Module** (`src/types.rs`): Core data structures and enums
16. **Models Module** (`src/models.rs`): Model pricing definitions
17. **Utils Module** (`src/utils.rs`): Utility functions and helpers

### Key Data Structures

#### Core Types
- `ConversationMessage`: Represents individual AI/User messages with full analytics
- `DailyStats`: Comprehensive daily usage aggregations
- `AgenticCodingToolStats`: Top-level container for all analytics
- `ModelPricing`: Token cost definitions per model

#### Analytics Types
- `FileOperationStats`: Tracks file read/write/edit operations by type and volume
- `TodoStats`: Tracks todo list usage and task completion
- `FileCategory`: Categorizes files by type (SourceCode, Data, Documentation, etc.)

### Core Functionality

1. **Multi-Tool Data Discovery**:
   - Claude Code: `~/.claude/projects` directories (JSONL files)
   - Codex CLI: `~/.codex/sessions/**/*.jsonl` files
   - Gemini CLI (retired): `~/.gemini/tmp/*/chats/*.json` directories (JSON session files)
   - GitHub Copilot: `~/.vscode/extensions/github.copilot-chat-*/sessions/*.json` files (VSCode, Cursor, Insiders variants)
   - WARP: `~/Library/Application Support/Warp/warp_network.log` (terminal telemetry) + GraphQL API at `app.warp.dev/graphql/v2` (conversation data)
   - Cline, Roo Code, Kilo Code, Qwen Code: Various VSCode extension data directories
2. **Flexible Conversation Parsing**: Processes different file formats (JSONL, JSON sessions, chat logs)
3. **Advanced Deduplication**: Uses tool-specific hashing strategies to prevent duplicate entries
4. **Comprehensive Cost Calculation**: Uses actual cost values or calculates from tokens using model pricing
5. **File Operation Tracking**: Monitors tool usage across different AI coding assistants
6. **Todo Analytics**: Tracks TodoWrite/TodoRead usage and task management (Claude Code)
7. **TUI Display**: Interactive terminal interface with multiple views and navigation
8. **Splitrail Cloud Integration**: Secure upload to Splitrail Cloud with API tokens
9. **Configuration Management**: TOML-based config with auto-upload settings

### Model Support

Currently supports:
**Claude Models:**
- `claude-sonnet-4-20250514` (Sonnet 4): $0.003/$0.015 per 1K input/output tokens
- `claude-opus-4-20250514` (Opus 4): $0.015/$0.075 per 1K input/output tokens
- `claude-opus-4.1` / `claude-opus-4-1-20250805` (Opus 4.1): Same as Opus 4 pricing (aliases)
- Cache pricing for both models (creation + read costs)

**GPT Models:**
- `gpt-5`: $1.25/$10.00 per 1K input/output tokens
- `gpt-5-mini`: $0.25/$2.00 per 1K input/output tokens
- `gpt-5-nano`: $0.05/$0.40 per 1K input/output tokens
- Cache pricing supported for all GPT-5 series models

**Gemini CLI Models:**
- `gemini-2.5-pro`: $0.001/$0.003 per 1K input/output tokens
- `gemini-2.5-flash`: $0.0005/$0.0015 per 1K input/output tokens
- `gemini-1.5-pro`: Legacy model support
- `gemini-1.5-flash`: Legacy model support
- Cache read pricing supported

**Codex CLI Models:**
- `o4-mini`: $1.10/$4.40 per 1M input/output tokens (cached: $0.275 per 1M)
- `o3`: $2.00/$8.00 per 1M input/output tokens (cached: $0.50 per 1M) 
- `o3-mini`: $1.10/$4.40 per 1M input/output tokens (cached: $0.55 per 1M)
- `o3-pro`: $20.00/$80.00 per 1M input/output tokens (no caching)
- `o1`, `o1-preview`: $15.00/$60.00 per 1M input/output tokens (cached: $7.50 per 1M)
- `o1-mini`: $1.10/$4.40 per 1M input/output tokens (cached: $0.55 per 1M)
- `o1-pro`: $150.00/$600.00 per 1M input/output tokens (no caching)
- `codex-mini-latest`: $1.50/$6.00 per 1M input/output tokens (cached: $0.375 per 1M)
- `gpt-4.1`: $2.00/$8.00 per 1M input/output tokens (cached: $0.50 per 1M)
- `gpt-4.1-mini`: $0.40/$1.60 per 1M input/output tokens (cached: $0.10 per 1M)
- `gpt-4.1-nano`: $0.10/$0.40 per 1M input/output tokens (cached: $0.025 per 1M)
- `gpt-4o`: $2.50/$10.00 per 1M input/output tokens (cached: $1.25 per 1M)
- `gpt-4o-mini`: $0.15/$0.60 per 1M input/output tokens (cached: $0.075 per 1M)
- `gpt-4-turbo`: $10.00/$30.00 per 1M input/output tokens (no caching)

**Features:**
- Fallback pricing for unknown models
- Multi-dimensional token tracking (input, output, cached, thoughts, tool tokens for Gemini)

### File Categories

Automatically categorizes files into:
- **Source Code**: .rs, .py, .js, .ts, .java, .cpp, .go, etc.
- **Data**: .json, .xml, .yaml, .csv, .sql, .db, etc.
- **Documentation**: .md, .txt, .html, .pdf, etc.
- **Media**: .png, .jpg, .mp4, .mp3, etc.
- **Config**: .config, .env, .toml, .ini, etc.
- **Other**: Everything else

### Dependencies

Core dependencies:
- `serde`/`simd-json` - SIMD-optimized JSON serialization and parsing
- `chrono`/`chrono-tz` - Timestamp handling and timezone conversion
- `ratatui` - Rich terminal user interface framework
- `crossterm` - Cross-platform terminal manipulation
- `reqwest` - HTTP client for Splitrail Cloud uploads
- `tokio` - Async runtime for HTTP operations
- `async-trait` - Async trait support for analyzer framework
- `toml` - Configuration file format
- `anyhow` - Error handling and context
- `colored` - Terminal color output
- `glob` - File pattern matching for data discovery
- `itertools` - Iterator utilities
- `rayon` - Parallel processing for file parsing
- `dashmap` - Concurrent hash maps
- `num-format` - Number formatting
- `home` - Home directory detection
- `lazy_static` - Static data initialization

## Configuration

Configuration is stored in `~/.config/splitrail/config.toml`:

```toml
[upload]
api_token = "st_your_token_here"
auto_upload = false
```

### Configuration Commands
- `splitrail config init` - Creates default config file
- `splitrail config show` - Displays current settings
- `splitrail config set api-token <token>` - Sets API token
- `splitrail config set auto-upload <true|false>` - Enables/disables auto-upload

## Features

### Multi-Tool Support
- **Claude Code**: Full support for JSONL conversation files, TodoWrite/TodoRead tracking
- **Codex CLI**: Command-line coding agent with shell command execution, reasoning model support, and token tracking
- **Gemini CLI**: retired 2026-09 (Google ended individual-account service on 2026-06-18); analyzer kept in tree but not registered
- **GitHub Copilot**: Chat session analysis from VSCode/Cursor/Insiders extensions with tool invocation tracking
- **WARP**: Terminal telemetry and GraphQL API conversation data with credits tracking, multi-model usage, comprehensive tool stats, and code modification metrics (lines added/removed)
- **Cline, Roo Code, Kilo Code, Qwen Code**: Additional VSCode extension analyzers for comprehensive coverage

### Terminal User Interface
- **Daily Stats View**: Comprehensive daily breakdown with costs, tokens, and operations
- **Model Usage**: Model-specific statistics and abbreviations across all supported tools
- **File Operations**: Detailed file operation analytics by category
- **Navigation**: Keyboard controls for scrolling and interaction

### Splitrail Cloud Integration
- Secure API token-based authentication
- Automatic daily stats upload when configured
- Manual upload command for on-demand sharing
- Privacy-focused: only aggregated statistics are uploaded to the leaderboard; per-day statistics are uploaded but are only shown to the user themselves

### Analytics Tracking
- **Token Usage**: Input, output, cache, thoughts, and tool token consumption
- **Cost Analysis**: Precise cost calculations per model and tool
- **File Operations**: Read/write/edit operations with byte/line counts
- **Tool Usage**: Tool-specific command tracking (Bash, Glob, Grep for Claude Code; shell command execution and file operations for Codex CLI; read_many_files, replace, run_shell_command for Gemini CLI)
- **Todo Management**: Task creation, completion, and productivity metrics (Claude Code)
- **Conversation Analytics**: Message counts, tool calls, and flow analysis
- **Deduplication**: Prevents duplicate entries across multiple data sources


## WARP Integration

### Overview

WARP is a modern terminal with AI coding assistance built-in. Splitrail provides comprehensive analytics for WARP usage through two data sources:

1. **Terminal Telemetry**: Local log files tracking command execution
2. **GraphQL API**: Cloud-based conversation data with detailed usage metrics

### Data Sources

#### Terminal Telemetry (Implemented)
- **Location**: `~/Library/Application Support/Warp/warp_network.log`
- **Format**: JSON telemetry events with batch data
- **Content**:
  - Command execution history
  - AI suggestion requests
  - Terminal interactions
  - Timestamps and working directories

#### GraphQL Conversation Data (Planned)
- **Endpoint**: `https://app.warp.dev/graphql/v2?op=GetConversationUsage`
- **Authentication**: Bearer token required
- **Historical Data**: 1+ month retention
- **Content**:
  - Full conversation history with titles
  - Credits spent per conversation
  - Model-specific token usage
  - Context window utilization
  - Comprehensive tool usage stats
  - Code modification metrics (lines added/removed)

### Credits System

WARP uses a "credits" system instead of direct USD pricing. Credits correlate with computational cost based on model complexity and reasoning depth.

**Average Credits per 1,000 Tokens**:

| Model | Credits/1K Tokens | Relative Cost |
|-------|-------------------|---------------|
| Claude 3.5 Sonnet | 0.0195 | Lowest |
| Gemini 2.5 Flash | 0.0293 | Very low |
| GLM 4.6 | 0.0323 | Low |
| Claude 4.5 Sonnet | 0.0405 | Baseline |
| Gemini 3 Pro | 0.1547 | Medium |
| Claude 4.5 Haiku | 0.2136 | Medium-high |
| GPT-5 (low reasoning) | 0.2197 | Medium-high |
| Claude 4 Sonnet | 1.2250 | High |
| GPT-5 (medium reasoning) | 1.7777 | Highest |

**Estimated Conversion**: 1 credit ≈ $0.22 USD (based on known model pricing)

### Supported Models

WARP provides access to multiple AI providers and models:

**Claude Family**:
- Claude 4.5 Sonnet (primary)
- Claude 4.5 Haiku (fast)
- Claude 4 Sonnet
- Claude 3.5 Sonnet (legacy)

**OpenAI GPT**:
- GPT-5 (medium reasoning)
- GPT-5 (low reasoning)

**Google Gemini**:
- Gemini 3 Pro
- Gemini 2.5 Flash
- Gemini 2.5 Pro

**Other**:
- GLM 4.6 (Zhipu AI)

### Unique Analytics Features

WARP provides analytics not available from other tools:

1. **Code Modification Metrics**:
   ```json
   "applyFileDiffStats": {
     "count": 85,
     "linesAdded": 1201,
     "linesRemoved": 629,
     "filesChanged": 4
   }
   ```

2. **Context Window Utilization**:
   - Tracks percentage of context window used (0.0-1.0)
   - Helps identify conversations approaching limits
   - Optimization opportunity for prompt engineering

3. **Multi-Model Conversations**:
   - Tracks which models were used in each conversation
   - Per-model token consumption
   - Switching patterns visible

4. **Comprehensive Tool Usage**:
   - `runCommandsExecuted`: Shell command count
   - `readFilesStats`: File read operations
   - `grepStats`: Code search operations
   - `searchCodebaseStats`: Codebase searches
   - `fileGlobStats`: File pattern matching
   - `callMcpToolStats`: MCP tool invocations
   - `applyFileDiffStats`: Code modifications with line counts

### Collection Methods

#### Method 1: Proxy Capture (Development)

Use mitmproxy to intercept GraphQL responses:

```bash
# Start proxy
./scripts/start_warp_proxy_quiet.sh

# Use WARP normally
# ...

# Stop proxy
./scripts/stop_warp_proxy.sh

# View captured data
ls schemas/warp/captured/
```

**Pros**: Real-time capture, no authentication needed
**Cons**: Requires proxy configuration, must run during usage

#### Method 2: Direct API Query (Recommended)

Query GraphQL API directly with auth token:

```rust
// Planned implementation
let response = client
    .post("https://app.warp.dev/graphql/v2?op=GetConversationUsage")
    .header("Authorization", format!("Bearer {}", auth_token))
    .send()
    .await?;
```

**Pros**: Historical data access, simpler UX, periodic sync
**Cons**: Requires auth token extraction, rate limits unknown

#### Method 3: Hybrid (Optimal)

1. Initial proxy capture to extract auth token
2. Save token to `~/.config/splitrail/config.toml`
3. Query API periodically for updates
4. Track `lastUpdated` timestamps for incremental sync

### Schema Documentation

Complete WARP data schema documentation available in:

- **`schemas/warp/WARP_SCHEMA.md`**: GraphQL API documentation
- **`schemas/warp/COLLECTION_STRATEGY.md`**: Implementation roadmap
- **`schemas/warp/ANALYSIS_SUMMARY.md`**: Sample data analysis and insights

### Sample Data Statistics

From captured sample data (208 conversations analyzed):

**Usage Patterns**:
- Average: ~86 credits per conversation (~$19 USD)
- Range: 0.5-2,211 credits per conversation
- Total analyzed: 17,860 credits (~$3,950 USD)

**Model Distribution**:
- Claude 4.5 Sonnet: 85% of total tokens
- Claude 4.5 Haiku: 8%
- Gemini 3 Pro: 4%
- GPT-5/Others: 3%

**Productivity Metrics**:
- Typical feature: 1,200 lines added, 630 removed
- Average 60+ file reads per investigation
- Average 50-70 shell commands per debug session

### Configuration

WARP configuration will be stored in `~/.config/splitrail/config.toml`:

```toml
[warp]
# GraphQL API authentication token
auth_token = "Bearer eyJ..."

# Enable automatic sync from API
auto_sync = true

# Sync interval in hours
sync_interval_hours = 6
```

### Implementation Status

**Completed**:
- ✅ Terminal telemetry parsing (`warp_network.log`)
- ✅ GraphQL schema documentation
- ✅ Collection strategy design
- ✅ Sample data analysis
- ✅ Credits correlation research

**In Progress**:
- ⬜ GraphQL response parsing (Rust structs)
- ⬜ API client implementation
- ⬜ Auth token management
- ⬜ Conversation deduplication

**Planned**:
- ⬜ Background sync daemon
- ⬜ TUI display enhancements
- ⬜ Splitrail Cloud integration
- ⬜ Official WARP API partnership

### Comparison with Other Tools

| Feature | Claude Code | Codex CLI | Gemini CLI | **WARP** |
|---------|-------------|-----------|------------|----------|
| **Data Source** | Local JSONL | Local JSONL | Local JSON | **Cloud API** |
| **Historical** | All time | All time | All time | **1+ month** |
| **Auth Required** | ❌ | ❌ | ❌ | **✅** |
| **Token Detail** | Input/output/cache | Input/output/cache | Input/output/thoughts | **Total only** |
| **Cost Format** | USD (calc) | USD (calc) | USD (calc) | **Credits (direct)** |
| **Code Metrics** | ❌ | ❌ | ❌ | **✅ Lines +/-** |
| **Multi-Model** | ❌ | ✅ | ✅ | **✅ (9+ models)** |
| **Context Track** | ❌ | ❌ | ❌ | **✅ Usage %** |
| **Tool Stats** | Basic | Shell only | File ops | **Comprehensive** |

**Key Advantage**: WARP provides the most comprehensive analytics with code modification metrics and context utilization tracking.

**Key Challenge**: Requires authentication setup, unlike file-based tools.

### Future Enhancements

1. **Official API Support**: Partner with WARP for documented API access
2. **OAuth Flow**: Implement proper token refresh mechanism
3. **Real-time Sync**: WebSocket subscriptions if WARP provides them
4. **Credit→USD Mapping**: Get official conversion rate from WARP
5. **Input/Output Split**: Request separate token counts in API response

### Resources

- **WARP Website**: https://warp.dev
- **GraphQL Endpoint**: https://app.warp.dev/graphql/v2
- **Schema Docs**: `schemas/warp/WARP_SCHEMA.md`
- **Collection Strategy**: `schemas/warp/COLLECTION_STRATEGY.md`
- **Analysis Summary**: `schemas/warp/ANALYSIS_SUMMARY.md`
- **Sample Data**: `schemas/warp/captured/*.jsonl`

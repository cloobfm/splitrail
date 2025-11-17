# Splitrail Schemas

This directory contains JSON schemas and sample data for the various AI coding tools that Splitrail supports. These schemas help document and validate the data formats used by different tools.

## Directory Structure

```
schemas/
├── codex-cli/          # Codex CLI data formats
│   ├── schema.json     # Session data schema
│   ├── schema-history.json  # History data schema
│   ├── samples/        # Sample session entries
│   └── README.md
├── claude-code/        # Claude Code data formats
│   ├── schema.json     # JSONL entries schema
│   ├── samples/        # Sample message entries
│   └── README.md
├── qwen-code/          # Qwen Code data formats
│   ├── schema.json     # Session data schema
│   ├── samples/        # Sample session files
│   └── README.md
├── kilo-code/          # Kilo Code data formats
│   ├── schema.json     # Configuration and history schema
│   ├── samples/        # Sample configuration and history entries
│   └── README.md
├── gemini-cli/         # Gemini CLI data formats
│   ├── schema.json     # Session data schema
│   ├── samples/        # Sample session files
│   └── README.md
└── amazon-q/           # Amazon Q data formats
    ├── schema.json     # SQLite database schema
    ├── samples/        # Sample conversation entries
    └── README.md
```

## Schema Overview

### Codex CLI
- **Session files** (`~/.codex/sessions/YYYY/MM/DD/*.jsonl`): Detailed conversation logs with working directory information
- **History file** (`~/.codex/history.jsonl`): Summary commands sent via CLI prompt

Key fields with working directory information:
- `payload.cwd` in `session_meta` and `turn_context` entries

### Claude Code
- **Project files** (`~/.claude/projects/*/UUID.jsonl`): Conversation logs by project
- Each line is a JSON object with various entry types

Key fields with working directory information:
- `cwd` field in message entries

### Qwen Code
- **Session files** (`~/.qwen/tmp/PROJECT_ID/chats/session-*.json`): Complete session conversations
- JSON files containing all messages in a session

Key fields with project information:
- `projectHash` identifies the project context

### Kilo Code
- **Configuration files** (`~/.kilocode/cli/config.json`): Configuration settings
- **History files** (`~/.kilocode/cli/history.json`): Command history
- **Task files** (`~/.kilocode/cli/global/tasks/*/api_conversation_history.json`): Conversation histories

Key fields with context information:
- Context information stored in task metadata and conversation files

### Gemini CLI
- **Session files** (`~/.gemini/tmp/PROJECT_ID/chats/session-*.json`): Complete session conversations
- JSON files containing all messages in a session

Key fields with project information:
- `projectHash` identifies the project context

### Amazon Q
- **SQLite database** (`~/Library/Application Support/amazon-q/data.sqlite3`): Contains conversations, history, and other data
- Multiple tables: `conversations`, `history`, `auth_kv`, `state`, `migrations`

Key fields with working directory information:
- `history.cwd` in the history table
- `env_state.current_working_directory` in conversation JSON data

## Sample Usage

These schemas can be used for:
- Validating log file formats
- Documentation of data structures
- Testing parser implementations
- Debugging format changes from tool updates

## Validation

The schemas follow JSON Schema Draft 2020-12 and can be validated using standard JSON Schema validators.
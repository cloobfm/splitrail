# Codex CLI Schema

Schema and samples for Codex CLI JSONL log files.

## Format Overview

Codex CLI uses two types of JSONL files:

### Session Files
- Location: `~/.codex/sessions/YYYY/MM/DD/*.jsonl`
- Content: Detailed conversation logs with comprehensive metadata
- Each line is a JSON object with different entry types

### History File  
- Location: `~/.codex/history.jsonl`
- Content: Summary commands sent via CLI prompt
- Each line contains basic session information

## Working Directory Tracking

Session files contain `cwd` field in:
- `session_meta` entries: Initial working directory for the session
- `turn_context` entries: Working directory for each turn/action

## File Structure

- `schema.json`: Schema for session files
- `schema-history.json`: Schema for history file
- `samples/`: Real sample entries extracted from actual Codex CLI sessions
# Claude Code Schema

Schema and samples for Claude Code JSONL log files.

## Format Overview

Claude Code stores conversation data in JSONL files located at:
- `~/.claude/projects/*/*.jsonl`

Each line in the file is a separate JSON object representing different types of entries:
- Message entries (user/assistant/system)
- Summary entries
- File history snapshots
- Queue operations

## Working Directory Tracking

The `cwd` field is present in most message entries and tracks the current working directory where Claude Code was operating when the message was created.

## File Structure

- `schema.json`: Complete JSON Schema definition
- `samples/`: Real sample entries extracted from actual Claude Code sessions
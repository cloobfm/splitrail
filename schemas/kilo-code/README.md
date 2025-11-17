# Kilo Code Schema

Schema and samples for Kilo Code configuration and conversation files.

## Format Overview

Kilo Code stores data in multiple file types located at:
- `~/.kilocode/cli/config.json` - Configuration settings
- `~/.kilocode/cli/history.json` - Command history
- `~/.kilocode/cli/global/tasks/*/api_conversation_history.json` - Conversation histories
- `~/.kilocode/cli/global/tasks/*/task_metadata.json` - Task metadata

## Configuration and Context Tracking

Kilo Code uses:
- Configuration files to store provider settings and preferences
- Task-based organization for conversations
- History files for command tracking

## File Structure

- `schema.json`: Complete JSON Schema definition
- `samples/`: Real sample configuration and history entries from actual Kilo Code sessions
# Qwen Code Schema

Schema and samples for Qwen Code JSON session files.

## Format Overview

Qwen Code stores conversation data in JSON files located at:
- `~/.qwen/tmp/PROJECT_ID/chats/session-*.json`

Each file contains a complete session with multiple messages between user and assistant.

## Project Context Tracking

Instead of working directory, Qwen Code uses:
- `projectHash`: A hash identifier for the project context
- `sessionId`: Unique identifier for the conversation session
- `startTime` / `lastUpdated`: Timestamps for session tracking

## File Structure

- `schema.json`: Complete JSON Schema definition
- `samples/`: Real sample session files extracted from actual Qwen Code sessions
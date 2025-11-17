# Gemini CLI Schema

Schema and samples for Gemini CLI JSON session files.

## Format Overview

Gemini CLI stores conversation data in JSON files located at:
- `~/.gemini/tmp/PROJECT_ID/chats/session-*.json`

Each file contains a complete session with multiple messages between user and assistant.

## Project Context Tracking

Gemini CLI uses:
- `projectHash`: A hash identifier for the project context
- `sessionId`: Unique identifier for the conversation session
- `startTime` / `lastUpdated`: Timestamps for session tracking

## File Structure

- `schema.json`: Complete JSON Schema definition
- `samples/`: Real sample session files extracted from actual Gemini CLI sessions
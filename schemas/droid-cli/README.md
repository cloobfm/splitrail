# Droid CLI Schema

Droid CLI (Factory AI) stores session data in JSONL format with accompanying settings files.

## Data Location

```
~/.factory/sessions/
├── <hashed-project-path>/
│   ├── <session-id>.jsonl
│   └── <session-id>.settings.json
```

## Session File Format (.jsonl)

Each line in the JSONL file represents a session event:

```json
{
  "type": "session_start",
  "id": "9a3222f3-b4e8-4126-8cc5-bca9399fe37a",
  "title": "New Session",
  "owner": "bzl",
  "version": 2,
  "cwd": "/Users/bzl/Projects/my-project"
}
```

### Fields

- `type`: Event type (typically "session_start")
- `id`: Unique session identifier (UUID)
- `title`: Session title or description
- `owner`: User who owns the session
- `version`: Session format version
- `cwd`: Working directory where session was created

## Settings File Format (.settings.json)

Contains token usage and session metadata:

```json
{
  "assistantActiveTimeMs": 60000,
  "model": "claude-opus-4-5-20251101",
  "reasoningEffort": "off",
  "autonomyMode": "normal",
  "tokenUsage": {
    "inputTokens": 1000,
    "outputTokens": 500,
    "cacheCreationTokens": 2000,
    "cacheReadTokens": 1000,
    "thinkingTokens": 0
  }
}
```

### Fields

- `assistantActiveTimeMs`: Time spent with assistant active (milliseconds)
- `model`: AI model used for the session
- `reasoningEffort`: Reasoning effort level ("off", "low", "medium", "high")
- `autonomyMode`: Autonomy mode ("normal", "high")
- `tokenUsage`: Detailed token usage breakdown

#### Token Usage Fields

- `inputTokens`: Number of input tokens
- `outputTokens`: Number of output tokens
- `cacheCreationTokens`: Tokens used for cache creation
- `cacheReadTokens`: Tokens read from cache
- `thinkingTokens`: Tokens used for internal reasoning (if supported)

## Supported Models

Droid CLI supports various models including:

- `claude-opus-4-5-20251101` - Claude Opus 4.5
- `claude-sonnet-4-20250514` - Claude Sonnet 4
- `claude-sonnet-4-1-20250714` - Claude Sonnet 4.1
- `claude-haiku-4-20250514` - Claude Haiku 4
- `gpt-5` - GPT-5
- `gpt-5-mini` - GPT-5 Mini
- `gpt-5-nano` - GPT-5 Nano

## Analytics Features

### Token Tracking
- Input/output tokens with precise counts
- Cache usage tracking (creation + read)
- Thinking tokens for models that support internal reasoning
- Cost calculation using model-specific pricing

### Session Metadata
- Working directory extraction for project context
- Session duration tracking
- Autonomy mode and reasoning effort levels
- User ownership and session identification

### Project Context
- Automatic project name extraction from working directory
- Full path preservation for analysis
- Session-based organization (one message per session)

## Integration Notes

- Droid CLI stores data in session directories, not single files
- Each session has a JSONL file for events and a JSON file for settings
- Token usage is comprehensive including cache metrics
- Working directory provides project context
- Sessions are created per project/workspace

## Sample Data

See `samples/` directory for example Droid CLI session files.
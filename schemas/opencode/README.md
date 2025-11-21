# OpenCode Schema

This directory contains the schema definition for OpenCode CLI conversation data.

## Current Status

**OpenCode does not currently store conversation data locally.** This schema is a placeholder implementation that anticipates potential future local data storage functionality.

## Expected Data Format (Future)

When OpenCode implements local conversation storage, it's expected to use a JSONL format similar to other AI coding tools:

### File Locations
- `~/.config/opencode/sessions/**/*.jsonl` - Global session storage
- `~/.local/share/opencode/sessions/**/*.jsonl` - Alternative global location  
- `~/.opencode/sessions/**/*.jsonl` - User home directory storage
- `**/.opencode/sessions/**/*.jsonl` - Project-specific storage

### Message Format

Each line in the JSONL file represents a single message:

```json
{
  "id": "msg_123456789",
  "timestamp": "2024-01-01T12:00:00Z",
  "role": "user|assistant",
  "content": "Message content here",
  "model": "opencode-zen|grok-code|local-llama|...",
  "tokens": {
    "input": 100,
    "output": 50,
    "cached": 20,
    "reasoning": 0
  },
  "tools": [
    {
      "name": "read|write|edit|bash|search|...",
      "duration_ms": 150,
      "success": true
    }
  ],
  "files": [
    {
      "path": "/path/to/file.ext",
      "operation": "read|write|edit|delete",
      "bytes": 1024,
      "lines": 50
    }
  ]
}
```

## Supported Models

OpenCode supports 75+ LLM providers through AI SDK and Models.dev:

### Free Models
- `opencode-zen` - OpenCode's free tier model
- `big-pickle` - OpenCode's Big Pickle model (free during beta)
- `grok-code` - xAI's Grok Code (currently free promotional tier)
- `openrouter-free` - Free tier models via OpenRouter
- `qwen3-coder-free` - Qwen's free coding model
- `local-*` - Any local model via Ollama/LM Studio (completely free)

### Paid Models
- `opencode-zen-pro` - OpenCode's premium tier
- `grok-code-pro` - xAI's premium Grok Code
- `openrouter-standard` - Standard tier via OpenRouter
- `kimi-k2` - Moonshot AI's Kimi K2 model

### Special Models
- `big-pickle` - OpenCode's Big Pickle model (200K context, free during beta)

## Integration Notes

The OpenCode analyzer in Splitrail is designed to:

1. **Auto-discover** OpenCode data from multiple potential locations
2. **Parse JSONL format** with robust error handling
3. **Track all model types** including free and local options
4. **Monitor tool usage** for file operations, searches, and terminal commands
5. **Calculate costs** with $0.00 for free/local models
6. **Deduplicate messages** using global hashing

## Current Limitations

- No local data storage available in OpenCode yet
- Analyzer will return empty results until OpenCode implements session persistence
- Schema is based on anticipated format and may need adjustments

## Future Development

When OpenCode adds local conversation storage, this analyzer will be ready to:

1. Parse the actual data format
2. Extract comprehensive usage statistics
3. Track costs across all model types
4. Monitor file operations and tool usage
5. Provide detailed analytics in the Splitrail TUI

For now, the OpenCode analyzer is registered but will show no data until local storage is implemented.
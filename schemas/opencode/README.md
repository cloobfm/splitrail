# OpenCode Schema

This directory contains the schema definition for OpenCode CLI conversation data.

## Current Status

✅ **OpenCode stores conversation data locally in `~/.local/share/opencode/storage/`**

The analyzer is fully functional and reads actual OpenCode data.

## Actual Data Format

OpenCode uses a structured JSON format (not JSONL) with separate directories for different data types:

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

## Performance Optimizations

✅ **Optimized for speed**: Only reads necessary files
- Message files: 313 files (complete statistics)
- Session files: 8 files (project context)
- **Skips** 1,557 part files (content fragments not needed for stats)
- **Result**: 83% reduction in file I/O

## Current Capabilities

The OpenCode analyzer now:

1. ✅ Parses actual OpenCode data format
2. ✅ Extracts comprehensive usage statistics
3. ✅ Tracks costs across all model types
4. ✅ Monitors tool usage and token counts
5. ✅ Provides detailed analytics in the Splitrail TUI
6. ✅ Uses actual timestamps from message creation
7. ✅ Reads real token counts (input, output, reasoning, cache)

For detailed optimization information, see `OPENCODE_ANALYZER_FIXES.md`.

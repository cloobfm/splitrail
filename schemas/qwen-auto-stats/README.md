# Qwen Auto-Stats Schemas

This directory contains JSON Schema definitions for Qwen's auto-generated statistics files. These schemas document the format of historical data that can be extracted from Qwen's auto-stats functionality.

## Files

### `request-log.schema.json`
Schema for Qwen request log files (`session-*.requests.jsonl`):
- Located in `~/.qwen/auto_stats/`
- Capture detailed request/response logs for each interaction
- Contain full conversation history with token-level details
- Format: JSONL (JSON Lines) - one JSON object per line

### `stats-log.schema.json` 
Schema for Qwen aggregated stats files (`session-*.stats.jsonl`):
- Located in `~/.qwen/auto_stats/`
- Contain aggregated statistics for each session
- Include token counts, latency measurements, model usage
- Format: JSONL (JSON Lines) - one JSON object per line

### `unified-qwen-schema.json`
Comprehensive schema that can validate all Qwen data formats (chat JSON, request JSONL, stats JSONL) in a unified structure.

## Data Location

Qwen auto-stats files are generated in:
```
~/.qwen/auto_stats/
```

## File Types

1. **Request Logs** (`*.requests.jsonl`): Detailed conversation records with:
   - Full prompt/response content
   - Token usage per request
   - Model information
   - Timing and performance data

2. **Stats Logs** (`*.stats.jsonl`): Aggregated session statistics with:
   - Per-model usage metrics
   - Token counts (input, output, cached)
   - Error rates and latency
   - Request volumes

3. **Chat Files** (`~/.qwen/tmp/*/chats/*.json`): Traditional chat format files

## Purpose

These schemas enable proper parsing and analysis of Qwen's historical data that would otherwise be stored in different formats across multiple files. The unified schema allows for consolidated processing of all Qwen data sources while maintaining compatibility with existing chat JSON format.
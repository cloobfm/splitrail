# Splitrail Cloud API Specification

This document specifies the internal API format for uploading usage data from Splitrail CLI to Splitrail Cloud.

## Overview

Splitrail uploads aggregated usage data from multiple AI coding tools to Splitrail Cloud for analytics and leaderboard functionality. The API uses chunked uploads with deduplication to handle high-volume data efficiently.

## API Endpoint

```
POST https://splitrail.dev/api/upload-stats
```

### Headers

```
Authorization: Bearer <api_token>
Content-Type: application/json
```

### Configuration

API configuration is stored in `~/.splitrail.toml`:

```toml
[server]
url = "https://splitrail.dev"
api_token = "st_xxx"

[upload]
auto_upload = false
upload_today_only = false
retry_attempts = 3
last_date_uploaded = 1704067200000  # Unix timestamp in milliseconds
```

## Upload Payload Format

### Request Body

The API accepts an array of `ConversationMessage` objects (max 4500 per request):

```json
[
  {
    "application": "claudeCode",
    "date": "2024-01-15T10:30:15Z",
    "projectHash": "abc123def456789",
    "conversationHash": "conv789xyz012345",
    "localHash": "msg_002_claude_local",
    "globalHash": "global_hash_abc123def456789",
    "model": "claude-sonnet-4-20250514",
    "role": "assistant",
    "content": "I'll help you create a Fibonacci function...",
    "stats": {
      "inputTokens": 25,
      "outputTokens": 20,
      "reasoningTokens": 0,
      "cacheCreationTokens": 0,
      "cacheReadTokens": 0,
      "cachedTokens": 0,
      "cost": 0.00075,
      "toolCalls": 1,
      
      "terminalCommands": 0,
      "fileSearches": 0,
      "fileContentSearches": 0,
      "filesRead": 1,
      "filesAdded": 1,
      "filesEdited": 0,
      "filesDeleted": 0,
      "linesRead": 8,
      "linesAdded": 25,
      "linesEdited": 0,
      "linesDeleted": 0,
      "bytesRead": 128,
      "bytesAdded": 512,
      "bytesEdited": 0,
      "bytesDeleted": 0,
      
      "todosCreated": 0,
      "todosCompleted": 0,
      "todosInProgress": 0,
      "todoWrites": 0,
      "todoReads": 0,
      
      "codeLines": 25,
      "docsLines": 0,
      "dataLines": 0,
      "mediaLines": 0,
      "configLines": 0,
      "otherLines": 0,
      
      "rateLimits": null
    }
  }
  // ... up to 4499 more messages
]
```

### Data Types

#### Application Enum
```json
"claudeCode" | "geminiCli" | "qwenCode" | "codexCli" | "cline" | 
"rooCode" | "kiloCode" | "copilot" | "amazonQ" | "kiroCli" | "warp" | "openCode"
```

#### Role Enum
```json
"user" | "assistant"
```

#### Stats Object

All fields are optional and default to 0:

**Token & Cost Metrics:**
- `inputTokens: number` - Input token count
- `outputTokens: number` - Output token count  
- `reasoningTokens: number` - Reasoning/thinking tokens
- `cacheCreationTokens: number` - Cache creation tokens
- `cacheReadTokens: number` - Cache read tokens
- `cachedTokens: number` - Total cached tokens
- `cost: number` - Calculated cost in USD
- `toolCalls: number` - Number of tool invocations

**File Operation Metrics:**
- `terminalCommands: number` - Shell commands executed
- `fileSearches: number` - File search operations
- `fileContentSearches: number` - Content search operations
- `filesRead/Added/Edited/Deleted: number` - File operation counts
- `linesRead/Added/Edited/Deleted: number` - Line operation counts
- `bytesRead/Added/Edited/Deleted: number` - Byte operation counts

**Todo Management (Claude Code specific):**
- `todosCreated/Completed/InProgress: number` - Todo state counts
- `todoWrites/Reads: number` - Todo operation counts

**Code Composition:**
- `codeLines: number` - Source code lines
- `docsLines: number` - Documentation lines
- `dataLines: number` - Data file lines
- `mediaLines: number` - Media file lines
- `configLines: number` - Configuration file lines
- `otherLines: number` - Other file type lines

**Rate Limits:**
- `rateLimits: object` - Optional rate limit information per model

## Response Format

### Success Response
```json
{
  "success": true,
  "error": null
}
```

### Error Response
```json
{
  "success": false,
  "error": "Invalid API token"
}
```

## Upload Process

### 1. Data Collection
- Multiple analyzers scan AI tool directories for conversation data
- Raw data is converted to standardized `ConversationMessage` format
- Global hashes are generated for deduplication

### 2. Filtering & Deduplication
- Messages are filtered by `last_date_uploaded` timestamp
- `get_messages_later_than(timestamp)` returns only new messages
- Global hashes prevent duplicate uploads on server side

### 3. Chunked Upload
- Messages are sent in chunks of 4500 to manage memory
- Each chunk is uploaded sequentially with progress tracking
- HTTP timeout: 30 seconds per request

### 4. Progress Tracking
```rust
// Progress callback format
fn(current: usize, total: usize) // current/total messages processed
```

### 5. State Management
- After successful upload: `config.set_last_date_uploaded(now)`
- Upload status: `None → Uploading → Uploaded/Failed`
- Auto-upload: 3-second debounce after file changes

## Server-Side Aggregation

The server aggregates raw messages into hierarchical views:

### Daily Stats (Per Tool)
```json
{
  "date": "2024-01-15",
  "userMessages": 15,
  "aiMessages": 12,
  "conversations": 3,
  "models": {
    "claude-sonnet-4-20250514": 8,
    "claude-opus-4-20250514": 4
  },
  "stats": { /* aggregated Stats object */ }
}
```

### Tool-Level Stats
```json
{
  "dailyStats": {
    "2024-01-15": { /* DailyStats */ },
    "2024-01-14": { /* DailyStats */ }
  },
  "numConversations": 27,
  "messages": [ /* all messages for this tool */ ],
  "analyzerName": "claude_code"
}
```

### Multi-Tool Aggregation
```json
{
  "analyzerStats": [
    { /* Tool-level stats for each analyzer */ }
  ]
}
```

## Privacy & Security

### Data Classification
- **Public Leaderboard**: Aggregated statistics only (totals, averages)
- **Private Dashboard**: Full per-day analytics for the user
- **Raw Messages**: Stored securely, used for detailed analytics

### Hashing Strategy
- `localHash`: Tool-specific message identifier (optional)
- `globalHash`: SHA-256 hash for cross-tool deduplication
- `projectHash`: Project-level grouping
- `conversationHash`: Conversation-level grouping

### Content Handling
- `content` field: Only included for recent messages to save memory
- Optional field: Skipped from serialization when null/empty
- Privacy: Message content never shown in public leaderboards

## Error Handling

### Client-Side
- Network timeouts: 30-second HTTP timeout
- SSL validation: `danger_accept_invalid_certs(true)` for flexibility
- Retry logic: Configurable retry attempts (default: 3)
- Graceful degradation: Continues working if API unavailable

### Server-Side
- HTTP status codes: Proper error response handling
- JSON parsing: SIMD-optimized for performance
- Deduplication: Global hash prevents duplicate insertion
- Validation: Schema validation for all incoming data

## Performance Characteristics

### Data Volume
- **Message Size**: ~1-3KB each (depending on content)
- **Chunk Size**: 4500 messages (~5-15MB per request)
- **Daily Volume**: 50-500 messages for heavy users
- **Compression**: JSON with SIMD optimization

### Optimization Features
- **Chunked Upload**: Prevents memory issues with large datasets
- **Incremental Sync**: Only uploads new data
- **Parallel Processing**: Concurrent file parsing across tools
- **SIMD JSON**: Fast serialization/deserialization

## Configuration Options

### Upload Behavior
```toml
[upload]
auto_upload = false          # Enable automatic uploads
upload_today_only = false    # Only upload today's data
retry_attempts = 3          # Number of retry attempts
last_date_uploaded = 0      # Timestamp for incremental sync
```

### Server Configuration
```toml
[server]
url = "https://splitrail.dev"  # API endpoint
api_token = ""                # Bearer token for authentication
```

## Testing & Development

### Local Testing
- Use `splitrail upload` for manual uploads
- Check configuration with `splitrail config show`
- Monitor progress in TUI interface

### Debug Information
- Upload status displayed in real-time
- Error messages with full context
- Config validation and helpful error messages

## Version History

- **v2.0.0**: Current specification with comprehensive stats tracking
- **v1.x**: Legacy format with limited metrics
- Future: Extended rate limiting, real-time sync, enhanced analytics

---

*This document is internal-facing and subject to change as the API evolves.*
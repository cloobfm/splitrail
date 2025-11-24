# WARP Data Schema Documentation

## Overview

WARP (by Cloudflare) provides detailed usage analytics via GraphQL API at `app.warp.dev/graphql/v2`. This document describes the data structures and collection methods for integrating WARP usage data into Splitrail.

## Data Collection Strategy

### Current Methods
1. **Live Proxy Capture** (mitmproxy): Intercepts GraphQL responses in real-time
2. **Direct API Query** (recommended): Query historical data via GraphQL API

### Historical Data Availability
- **Time Range**: At least 1 month of historical conversation data
- **Granularity**: Per-conversation metrics with full tool usage breakdown
- **Update Frequency**: Real-time updates as conversations progress

## GraphQL Operations

### 1. GetRequestLimitInfo
**Endpoint**: `app.warp.dev/graphql/v2?op=GetRequestLimitInfo`

Returns user quota and usage limits.

```json
{
  "data": {
    "user": {
      "__typename": "UserOutput",
      "user": {
        "requestLimitInfo": {
          "isUnlimited": false,
          "nextRefreshTime": "2025-12-19T05:44:43.851819Z",
          "requestLimit": 10000,
          "requestsUsedSinceLastRefresh": 9700,
          "requestLimitRefreshDuration": "MONTHLY",
          "isUnlimitedAutosuggestions": true,
          "acceptedAutosuggestionsLimit": 999999,
          "acceptedAutosuggestionsSinceLastRefresh": 21,
          "isUnlimitedVoice": true,
          "voiceRequestLimit": 999999,
          "voiceRequestsUsedSinceLastRefresh": 9,
          "voiceTokenLimit": 10000000,
          "voiceTokensUsedSinceLastRefresh": 0,
          "isUnlimitedCodebaseIndices": false,
          "maxCodebaseIndices": 40,
          "maxFilesPerRepo": 20000,
          "embeddingGenerationBatchSize": 100,
          "requestLimitPooling": "USER"
        }
      }
    }
  }
}
```

### 2. GetConversationUsage (Primary Analytics Source)
**Endpoint**: `app.warp.dev/graphql/v2?op=GetConversationUsage`

Returns comprehensive per-conversation usage metrics.

#### Response Structure

```json
{
  "data": {
    "user": {
      "__typename": "UserOutput",
      "user": {
        "conversationUsage": [
          {
            "conversationId": "391c6d16-9080-40e6-9cba-260c44229d27",
            "lastUpdated": "2025-11-24T06:36:23Z",
            "title": "Investigate Facility Layout Dragging Lag",
            "usageMetadata": {
              "contextWindowUsage": 0.50455,
              "creditsSpent": 1169.789466666666,
              "summarized": true,
              "tokenUsage": [
                {
                  "modelId": "gpt-5 (medium reasoning)",
                  "totalTokens": 359311
                },
                {
                  "modelId": "claude 4.5 sonnet",
                  "totalTokens": 35608828
                }
              ],
              "toolUsageMetadata": {
                "runCommandStats": {"count": 69},
                "runCommandsExecuted": 69,
                "readFilesStats": {"count": 62},
                "searchCodebaseStats": {"count": 0},
                "grepStats": {"count": 10},
                "fileGlobStats": {"count": 0},
                "callMcpToolStats": {"count": 0},
                "readMcpResourceStats": {"count": 1},
                "suggestPlanStats": {"count": 0},
                "suggestCreatePlanStats": {"count": 0},
                "writeToLongRunningShellCommandStats": {"count": 0},
                "applyFileDiffStats": {
                  "count": 85,
                  "linesAdded": 1201,
                  "linesRemoved": 629,
                  "filesChanged": 4
                },
                "readShellCommandOutputStats": {"count": 0}
              }
            }
          }
        ]
      }
    }
  }
}
```

## Data Model

### ConversationUsage

| Field | Type | Description |
|-------|------|-------------|
| `conversationId` | String (UUID) | Unique conversation identifier |
| `lastUpdated` | ISO 8601 DateTime | Last modification timestamp |
| `title` | String | Auto-generated conversation title |
| `usageMetadata` | Object | Detailed usage metrics |

### UsageMetadata

| Field | Type | Description |
|-------|------|-------------|
| `contextWindowUsage` | Float | Context window utilization (0.0-1.0) |
| `creditsSpent` | Float | Total credits consumed (see Credits section) |
| `summarized` | Boolean | Whether conversation has been summarized |
| `tokenUsage` | Array[TokenUsage] | Per-model token consumption |
| `toolUsageMetadata` | Object | Tool execution statistics |

### TokenUsage

| Field | Type | Description |
|-------|------|-------------|
| `modelId` | String | Model identifier (e.g., "claude 4.5 sonnet") |
| `totalTokens` | Integer | Total tokens (input + output combined) |

**Note**: WARP does not provide separate input/output token counts in the API response.

### ToolUsageMetadata

| Tool | Type | Fields | Description |
|------|------|--------|-------------|
| `runCommandStats` | Object | `count` | Shell commands executed |
| `runCommandsExecuted` | Integer | - | Total commands run |
| `readFilesStats` | Object | `count` | Files read |
| `searchCodebaseStats` | Object | `count` | Codebase searches |
| `grepStats` | Object | `count` | Grep operations |
| `fileGlobStats` | Object | `count` | File glob patterns |
| `callMcpToolStats` | Object | `count` | MCP tool calls |
| `readMcpResourceStats` | Object | `count` | MCP resource reads |
| `suggestPlanStats` | Object | `count` | Plan suggestions |
| `suggestCreatePlanStats` | Object | `count` | Create plan suggestions |
| `writeToLongRunningShellCommandStats` | Object | `count` | Long-running shell writes |
| `applyFileDiffStats` | Object | `count`, `linesAdded`, `linesRemoved`, `filesChanged` | Code modifications |
| `readShellCommandOutputStats` | Object | `count` | Shell output reads |

## Credits System

WARP uses a "credits" system that represents computational cost, not just token count. Credits correlate with model complexity and reasoning depth.

### Credits per 1K Tokens (Averaged)

| Model | Credits/1K Tokens | Notes |
|-------|-------------------|-------|
| Claude 3.5 Sonnet | 0.0195 | Legacy model |
| Claude 4.5 Sonnet | 0.0405 | Primary model |
| Claude 4.5 Haiku | 0.2136 | Fast/cheap variant |
| Claude 4 Sonnet | 1.2250 | Higher cost |
| Gemini 2.5 Flash | 0.0293 | Fast Google model |
| Gemini 3 Pro | 0.1547 | Advanced Google model |
| GLM 4.6 | 0.0323 | Chinese model |
| GPT-5 (low reasoning) | 0.2197 | OpenAI reasoning |
| GPT-5 (medium reasoning) | 1.7777 | High-cost reasoning |

**Important**: These ratios are averages and may vary based on:
- Input vs. output token distribution (not exposed in API)
- Reasoning depth for reasoning models
- Cache hits (not tracked separately)

### Credit Calculation Formula

Given the lack of input/output breakdown, approximate cost calculation:

```
credits ≈ sum(totalTokens[model] * creditsPerToken[model]) / 1000
```

For more accurate cost tracking, monitor actual `creditsSpent` from API rather than calculating from tokens.

## Supported Models

### Claude Family
- `claude 4.5 sonnet` (primary)
- `claude 4.5 haiku` (fast)
- `claude 4 sonnet`
- `claude 3.5 sonnet` (legacy)

### OpenAI GPT
- `gpt-5 (medium reasoning)`
- `gpt-5 (low reasoning)`

### Google Gemini
- `gemini 3 pro`
- `gemini 2.5 flash`
- `gemini 2.5 pro` (not observed in sample)

### Other
- `glm 4.6` (Zhipu AI)

## Data Collection Recommendations

### For Splitrail Integration

**Option 1: Direct GraphQL API** (Recommended)
- Query `GetConversationUsage` periodically (daily/hourly)
- Store conversations by `conversationId`
- Track `lastUpdated` timestamp for incremental updates
- No proxy required, works from any environment

**Option 2: Proxy Interception** (Development/Testing)
- Use mitmproxy with interceptor script
- Captures real-time GraphQL responses
- Useful for schema discovery and debugging
- Requires proxy configuration in WARP app

### Implementation Strategy

```rust
// Pseudocode for WARP analyzer
struct WarpAnalyzer {
    api_endpoint: String,
    auth_token: String,
}

impl WarpAnalyzer {
    async fn fetch_conversations(&self) -> Vec<Conversation> {
        // Query GetConversationUsage API
        // Parse response JSON
        // Return conversation objects
    }

    fn parse_conversation(&self, data: Value) -> ConversationMessage {
        // Extract conversationId, title, lastUpdated
        // Parse usageMetadata
        // Build ConversationMessage with:
        //   - Model usage (from tokenUsage array)
        //   - Credits spent
        //   - Tool usage stats
        //   - Context window utilization
    }

    fn calculate_stats(&self, convos: Vec<Conversation>) -> DailyStats {
        // Aggregate by date (from lastUpdated)
        // Sum credits, tokens per model
        // Count tool operations
        // Calculate productivity metrics (lines added/removed)
    }
}
```

## API Authentication

WARP API requires authentication token. Methods:
1. **Extract from mitmproxy**: Capture `Authorization` header from intercepted requests
2. **Browser DevTools**: Inspect GraphQL requests in browser console
3. **WARP config files**: Check `~/.warp/` for stored credentials (if available)

## Schema Evolution

Track schema changes:
- New tool types (e.g., future `editFileStats`)
- Additional models
- New metadata fields (e.g., separate input/output tokens)
- Credit pricing adjustments

## Sample Queries

### Get All Conversations
```graphql
query GetConversationUsage {
  user {
    user {
      conversationUsage {
        conversationId
        lastUpdated
        title
        usageMetadata {
          creditsSpent
          contextWindowUsage
          summarized
          tokenUsage {
            modelId
            totalTokens
          }
          toolUsageMetadata {
            runCommandsExecuted
            readFilesStats { count }
            applyFileDiffStats {
              count
              linesAdded
              linesRemoved
              filesChanged
            }
          }
        }
      }
    }
  }
}
```

### Get Request Limits
```graphql
query GetRequestLimitInfo {
  user {
    user {
      requestLimitInfo {
        requestLimit
        requestsUsedSinceLastRefresh
        nextRefreshTime
      }
    }
  }
}
```

## Data Retention

Based on observed data:
- Minimum 1 month of historical conversations
- Unknown maximum retention period
- Conversations persist after summarization
- No apparent conversation deletion in API

## Comparison with Other Tools

| Feature | Claude Code | Codex CLI | Gemini CLI | WARP |
|---------|-------------|-----------|------------|------|
| Data Format | JSONL files | JSONL files | JSON sessions | GraphQL API |
| Historical Access | Local files only | Local files only | Local files only | **API query** |
| Token Breakdown | Input/output/cache | Input/output/cache | Input/output/thoughts | **Total only** |
| Cost Tracking | Calculate from tokens | Calculate from tokens | Calculate from tokens | **Direct credits** |
| Tool Tracking | Tool calls logged | Shell commands | File operations | **Comprehensive** |
| File Modifications | Not tracked | Not tracked | replace operations | **Lines added/removed** |

## Future Enhancements

Potential additions to WARP analyzer:
1. **Conversation deduplication**: Track by `conversationId`
2. **Incremental updates**: Use `lastUpdated` timestamp
3. **Credit-to-USD conversion**: Map credits to actual dollar costs
4. **Model abbreviations**: Standardize model names (e.g., "c45s" for Claude 4.5 Sonnet)
5. **Productivity metrics**: Leverage `applyFileDiffStats` for code contribution tracking
6. **Multi-workspace support**: Handle workspace-specific data if available

## Known Limitations

1. **No input/output token split**: Total tokens only, limits precise cost calculation
2. **No cache tracking**: Cache hits not separately reported
3. **Credits are opaque**: No documented conversion rate to USD
4. **Authentication complexity**: Token extraction non-trivial
5. **Rate limiting**: Unknown API rate limits for queries

## References

- WARP API endpoint: `https://app.warp.dev/graphql/v2`
- Sample data location: `schemas/warp/captured/`
- Interceptor script: `scripts/warp_interceptor.py`

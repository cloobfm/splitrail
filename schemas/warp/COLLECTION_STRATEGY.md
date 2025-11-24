# WARP Data Collection Strategy

## Executive Summary

WARP provides two distinct data sources for analysis:
1. **Terminal Telemetry**: Command history from `warp_network.log` (already implemented)
2. **AI Conversation Data**: GraphQL API responses (new discovery)

This document outlines the strategy to collect and integrate both data sources into Splitrail.

## Current State

### Existing Implementation (`warp_dev.rs`)
- **Data Source**: `~/Library/Application Support/Warp/warp_network.log`
- **Content**: Terminal command execution logs and AI suggestion requests
- **Limitations**:
  - No detailed AI conversation tracking
  - No credit/token usage
  - No tool usage breakdown
  - Limited to local log files

### New Discovery (GraphQL API)
- **Endpoint**: `https://app.warp.dev/graphql/v2`
- **Content**: Full AI conversation history with:
  - Per-conversation credit costs
  - Model-specific token usage
  - Comprehensive tool operation stats
  - Code modification metrics (lines added/removed)
  - Historical data (1+ month retention)

## Data Collection Methods

### Method 1: Live Proxy Capture (Current Proof-of-Concept)

**How it works:**
1. Run `mitm proxy` on localhost:8080
2. Configure WARP to use proxy
3. Intercept GraphQL responses
4. Save to JSONL files

**Scripts:**
- `scripts/start_warp_proxy_quiet.sh` - Start proxy in background
- `scripts/stop_warp_proxy.sh` - Stop proxy
- `scripts/warp_interceptor.py` - Parse and save GraphQL data

**Pros:**
- Real-time data capture
- No API authentication needed
- Useful for schema discovery

**Cons:**
- Requires proxy setup
- Must run during WARP usage
- User friction (proxy configuration)
- No historical data on first run

### Method 2: Direct API Query (Recommended)

**How it works:**
1. Extract auth token from WARP session
2. Query GraphQL API directly
3. Parse `GetConversationUsage` responses
4. Store conversations by ID

**Implementation:**
```rust
// Pseudocode
async fn fetch_warp_conversations(auth_token: &str) -> Result<Vec<Conversation>> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://app.warp.dev/graphql/v2?op=GetConversationUsage")
        .header("Authorization", format!("Bearer {}", auth_token))
        .send()
        .await?;

    let data: GraphQLResponse = response.json().await?;
    parse_conversations(data)
}
```

**Pros:**
- No proxy needed
- Access to full historical data (1+ months)
- Simpler user experience
- Can run periodically (cron/background)

**Cons:**
- Requires auth token extraction
- Need to handle API rate limits
- Token may expire

### Method 3: Hybrid Approach (Best Long-Term)

**Initial Setup:**
1. Use proxy capture to extract auth token
2. Save token to config file (`~/.config/splitrail/warp_token`)

**Ongoing Collection:**
1. Query API periodically (daily/hourly)
2. Track `lastUpdated` timestamps
3. Only fetch new/updated conversations
4. Fall back to proxy if API fails

**Benefits:**
- Best of both worlds
- Historical data on first run
- Incremental updates
- Resilient to API changes

## Authentication Token Extraction

### Option 1: From Proxy Capture
```python
# In warp_interceptor.py
def request(flow: http.HTTPFlow):
    if "app.warp.dev/graphql" in flow.request.pretty_url:
        auth_header = flow.request.headers.get("Authorization")
        if auth_header:
            # Save to file
            with open(os.path.expanduser("~/.config/splitrail/warp_token"), "w") as f:
                f.write(auth_header)
```

### Option 2: From Browser DevTools
1. Open WARP web interface
2. Open browser DevTools (Network tab)
3. Filter for "graphql"
4. Copy `Authorization` header
5. Save to `~/.config/splitrail/config.toml`:
   ```toml
   [warp]
   auth_token = "Bearer eyJ..."
   ```

### Option 3: From WARP Config Files
Investigate if WARP stores tokens in:
- `~/Library/Application Support/Warp/`
- `~/.warp/`
- `~/.config/warp/`

## Data Mapping to Splitrail Schema

### Conversation → ConversationMessage

```rust
// Map WARP conversation to Splitrail message
fn map_warp_conversation(conv: WarpConversation) -> Vec<ConversationMessage> {
    let mut messages = Vec::new();

    // Create user message (conversation start)
    messages.push(ConversationMessage {
        date: parse_iso8601(&conv.last_updated),
        application: Application::Warp,
        project_hash: hash_text("warp_cloud"), // WARP doesn't expose project
        conversation_hash: hash_text(&conv.conversation_id),
        global_hash: hash_text(&format!("warp_{}", conv.conversation_id)),
        role: MessageRole::User,
        content: Some(conv.title.clone()),
        model: None,
        stats: Stats::default(),
        local_hash: None,
    });

    // Create assistant message with full stats
    messages.push(ConversationMessage {
        date: parse_iso8601(&conv.last_updated),
        application: Application::Warp,
        project_hash: hash_text("warp_cloud"),
        conversation_hash: hash_text(&conv.conversation_id),
        global_hash: hash_text(&format!("warp_{}__resp", conv.conversation_id)),
        role: MessageRole::Assistant,
        content: Some(format!("Completed conversation: {}", conv.title)),
        model: conv.usage_metadata.primary_model(), // Most-used model
        stats: Stats {
            // Token usage - sum across all models
            tokens_in: 0, // Not available (total only)
            tokens_out: 0, // Not available (total only)
            total_tokens: conv.usage_metadata.total_tokens(),

            // Cost tracking
            cost: Some(conv.usage_metadata.credits_to_usd()),

            // Tool usage from toolUsageMetadata
            tool_calls: conv.usage_metadata.tool_metadata.total_tool_calls(),
            bash_commands: conv.usage_metadata.tool_metadata.run_commands_executed,
            file_reads: conv.usage_metadata.tool_metadata.read_files_stats.count,
            grep_searches: conv.usage_metadata.tool_metadata.grep_stats.count,

            // Code modifications
            files_edited: conv.usage_metadata.tool_metadata.apply_file_diff_stats.files_changed,
            lines_added: conv.usage_metadata.tool_metadata.apply_file_diff_stats.lines_added,
            lines_removed: conv.usage_metadata.tool_metadata.apply_file_diff_stats.lines_removed,

            ..Stats::default()
        },
        local_hash: None,
    });

    messages
}
```

### Credits → USD Conversion

Since WARP uses credits instead of dollars, we need a conversion rate:

**Option 1: Empirical Mapping**
```rust
fn credits_to_usd(credits: f64) -> f64 {
    // Based on observed credits/token ratios and known pricing
    // Example: Claude 4.5 Sonnet @ 0.0405 credits/1K tokens
    // If actual cost is $0.003 input + $0.015 output per 1K
    // Assuming 50/50 split: avg = $0.009 per 1K
    // Then: 1 credit ≈ $0.009 / 0.0405 ≈ $0.222

    credits * 0.22 // Approximate conversion
}
```

**Option 2: Request Limit Correlation**
```rust
// If WARP charges $X/month for Y requests (10,000 in free tier)
// And average request costs Z credits
// Then: 1 credit = ($X / (Y * Z))
```

**Option 3: Direct Mapping** (if WARP publishes rates)
- Check WARP documentation/pricing page
- Look for credit → USD conversion

## Model Pricing Configuration

Add WARP models to `src/models.rs`:

```rust
// WARP uses "credits" not USD, so we store credits/1K tokens
pub static WARP_MODEL_PRICING: &[(&str, ModelPricing)] = &[
    ("claude 4.5 sonnet", ModelPricing {
        input_tokens: 0.0405,  // credits per 1K
        output_tokens: 0.0405, // Same (can't distinguish)
        cache_creation_tokens: 0.0,
        cache_read_tokens: 0.0,
    }),
    ("claude 4.5 haiku", ModelPricing {
        input_tokens: 0.2136,
        output_tokens: 0.2136,
        cache_creation_tokens: 0.0,
        cache_read_tokens: 0.0,
    }),
    ("gpt-5 (medium reasoning)", ModelPricing {
        input_tokens: 1.7777,
        output_tokens: 1.7777,
        cache_creation_tokens: 0.0,
        cache_read_tokens: 0.0,
    }),
    ("gemini 2.5 flash", ModelPricing {
        input_tokens: 0.0293,
        output_tokens: 0.0293,
        cache_creation_tokens: 0.0,
        cache_read_tokens: 0.0,
    }),
    ("gemini 3 pro", ModelPricing {
        input_tokens: 0.1547,
        output_tokens: 0.1547,
        cache_creation_tokens: 0.0,
        cache_read_tokens: 0.0,
    }),
    // ... other models
];
```

## Implementation Plan

### Phase 1: Schema & Data Structures (1-2 hours)
- [ ] Define Rust structs for GraphQL response
- [ ] Add WARP model pricing to `src/models.rs`
- [ ] Create deserialization for conversation data
- [ ] Write unit tests for parsing

### Phase 2: API Client (2-3 hours)
- [ ] Implement GraphQL query function
- [ ] Add auth token management
- [ ] Handle pagination (if needed)
- [ ] Error handling & retries
- [ ] Rate limiting

### Phase 3: Extend `warp_dev.rs` Analyzer (3-4 hours)
- [ ] Add `discover_graphql_sources()` method
- [ ] Look for captured JSONL files in `schemas/warp/captured/`
- [ ] Parse GraphQL conversation data
- [ ] Merge with existing terminal telemetry
- [ ] Deduplication by conversation ID
- [ ] Map to `ConversationMessage` format

### Phase 4: Configuration & Auth (1-2 hours)
- [ ] Add WARP section to `config.toml`
- [ ] Token storage & retrieval
- [ ] Auto-detect token from proxy capture
- [ ] Token validation

### Phase 5: Testing & Validation (2-3 hours)
- [ ] Test with captured sample data
- [ ] Verify aggregation accuracy
- [ ] Check deduplication
- [ ] Validate cost calculations
- [ ] TUI display testing

### Phase 6: Documentation (1 hour)
- [ ] Update CLAUDE.md with WARP details
- [ ] Add usage instructions
- [ ] Document auth token setup
- [ ] Create troubleshooting guide

**Total Estimated Time: 10-15 hours**

## Future Collection Strategy

### Short Term (MVP)
1. Parse captured JSONL files from `schemas/warp/captured/`
2. Manual proxy capture when user wants fresh data
3. Display in TUI alongside other tools

### Medium Term (Automated)
1. Implement direct API querying
2. Store auth token in config
3. Daily background sync (cron or daemon)
4. Incremental updates via `lastUpdated` tracking

### Long Term (Production)
1. Official WARP API support (if/when available)
2. OAuth flow for token refresh
3. Real-time webhook subscriptions (if supported)
4. Splitrail Cloud integration for centralized tracking

## Data Frequency & Patterns

Based on captured data analysis:

### Update Frequency
- **Real-time**: Conversations update as messages are sent
- **Batch updates**: GraphQL returns all conversations in single query
- **Polling interval**: Recommended every 1-6 hours (most users won't have changes more frequently)

### Data Retention
- **Historical**: At least 1 month of conversation history
- **Conversation lifetime**: Persists indefinitely (no observed deletions)
- **Summarization**: Older conversations get `summarized: true` flag

### Data Volume
Sample user had:
- 208 conversations over ~1 month
- Average: ~7 conversations/day
- Total credits: 17,860 (~$3,950 equivalent if 1 credit ≈ $0.22)

### Peak Usage Patterns
- Most expensive conversations: 1,000-2,200 credits
- Typical conversation: 10-100 credits
- Quick fixes: 0.5-5 credits

## Security & Privacy

### Token Storage
```toml
# ~/.config/splitrail/config.toml
[warp]
# NEVER commit this file to git!
auth_token = "Bearer eyJhbGc..."
auto_sync = true
sync_interval_hours = 6
```

### Data Handling
- Store conversation IDs, titles, and stats only
- Do NOT store conversation content (prompts/responses)
- Only aggregate metrics uploaded to Splitrail Cloud
- Full conversation data stays local

### Token Security
- Store in user-only readable file (chmod 600)
- Encrypt if possible (keychain integration)
- Provide easy token rotation
- Clear error messages if token expires

## Comparison with Other CLIs

| Feature | Claude Code | Codex CLI | Gemini CLI | **WARP** |
|---------|-------------|-----------|------------|----------|
| **Data Location** | Local JSONL | Local JSONL | Local JSON | **Cloud API** |
| **Historical Access** | File-based | File-based | File-based | **API Query** |
| **Auth Required** | No | No | No | **Yes** |
| **Token Detail** | Input/output | Input/output | Input/output/thoughts | **Total only** |
| **Cost Tracking** | Calculated | Calculated | Calculated | **Direct credits** |
| **Tool Stats** | Basic | Shell only | File ops | **Comprehensive** |
| **Code Metrics** | No | No | replace() | **Lines +/-** |
| **Retention** | Forever (local) | Forever (local) | Forever (local) | **1+ month (cloud)** |

**Key Advantage**: WARP's cloud API provides historical data without local file management.

**Key Challenge**: Authentication adds complexity compared to file-based tools.

## Next Steps

1. **Immediate**: Document findings in CLAUDE.md
2. **This Week**: Implement Phase 1-3 (core parsing)
3. **This Month**: Complete Phase 4-6 (full integration)
4. **Future**: Explore official WARP API partnership for better integration

## Questions for WARP Team (Future)

If we reach out to WARP for official API support:

1. Is there a documented API for conversation data access?
2. What are the rate limits for GraphQL queries?
3. How long is conversation history retained?
4. Can we get separate input/output token counts?
5. What's the official credits → USD conversion rate?
6. Is there an OAuth flow for token management?
7. Any plans for developer-friendly analytics APIs?

## Conclusion

WARP's GraphQL API provides rich conversation analytics that surpass most local-file-based CLIs. The main implementation challenge is authentication, but the payoff is access to comprehensive historical data with detailed tool usage and cost tracking. The hybrid approach (initial proxy capture + ongoing API sync) provides the best user experience while maintaining data accuracy.

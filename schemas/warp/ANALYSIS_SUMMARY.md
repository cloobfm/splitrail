# WARP Data Analysis Summary

**Date**: November 23-24, 2025
**Analysis Duration**: ~5 minute capture session
**Data Volume**: 51 GraphQL responses, 208 conversations analyzed

---

## Key Discoveries

### 1. Rich Historical Data Available via API ✅

**Finding**: WARP provides comprehensive conversation history through GraphQL API, not just live data.

**Evidence**:
- Conversations date back to **October 29, 2025** (nearly 1 month)
- 208 total conversations captured in single API query
- Full conversation metadata including credits, tokens, and tool usage
- No live-only limitation - can query historical data anytime

**Impact**: Unlike other CLIs that require local file parsing, WARP data can be fetched on-demand from the cloud.

### 2. Credits System (Not Direct USD Pricing)

**Finding**: WARP uses a "credits" system that represents computational cost, correlating with model complexity and reasoning depth.

**Credits per 1,000 Tokens** (averaged across all conversations):

| Model | Credits/1K | Relative Cost |
|-------|-----------|---------------|
| Claude 3.5 Sonnet | 0.0195 | Lowest |
| Gemini 2.5 Flash | 0.0293 | Very low |
| GLM 4.6 | 0.0323 | Low |
| **Claude 4.5 Sonnet** | **0.0405** | **Baseline** |
| Gemini 3 Pro | 0.1547 | Medium |
| Claude 4.5 Haiku | 0.2136 | Medium-high |
| GPT-5 (low reasoning) | 0.2197 | Medium-high |
| Claude 4 Sonnet | 1.2250 | High |
| **GPT-5 (medium reasoning)** | **1.7777** | **Highest** |

**Conversion Estimate**: If Claude 4.5 Sonnet costs ~$0.009/1K tokens (avg of input/output), then:
- 0.0405 credits = $0.009
- **1 credit ≈ $0.22 USD**

**Sample User Spending**:
- Total credits: 17,860.7
- Estimated cost: **~$3,950** over 1 month
- Most expensive conversation: 2,211 credits (~$490)

### 3. Comprehensive Tool Usage Tracking

**Finding**: WARP tracks detailed tool operations per conversation, including code modification metrics.

**Available Metrics**:
```javascript
{
  "runCommandsExecuted": 69,              // Shell commands
  "readFilesStats": {"count": 62},        // File reads
  "grepStats": {"count": 10},             // Code searches
  "applyFileDiffStats": {                 // Code modifications
    "count": 85,
    "linesAdded": 1201,                   // Productivity metric!
    "linesRemoved": 629,
    "filesChanged": 4
  },
  "searchCodebaseStats": {"count": 0},
  "fileGlobStats": {"count": 0},
  "callMcpToolStats": {"count": 0},
  "readMcpResourceStats": {"count": 1},
  "suggestPlanStats": {"count": 0},
  "suggestCreatePlanStats": {"count": 0},
  "writeToLongRunningShellCommandStats": {"count": 0},
  "readShellCommandOutputStats": {"count": 0}
}
```

**Comparison**: Most CLIs don't track code modifications (lines added/removed) - this is unique to WARP!

### 4. Multi-Model Conversations

**Finding**: Users frequently switch between models within a single conversation.

**Common Patterns**:
- Claude 4.5 Sonnet (primary workhorse)
- GPT-5 (medium reasoning) for complex problems
- Gemini 2.5 Flash / Claude Haiku for quick fixes
- Mix of 2-4 models per high-value conversation

**Example**: "Migrate Project to Dark Mode Styling" conversation used:
- Gemini 3 Pro
- Claude 4.5 Haiku
- GPT-5 (medium reasoning)
- Claude 4.5 Sonnet
- **Total**: 2,211 credits (~$490)

### 5. Context Window Utilization

**Finding**: Most conversations use <70% of context window, with some hitting 87%.

**Distribution**:
- Low usage (0-25%): 45% of conversations
- Medium usage (25-50%): 35% of conversations
- High usage (50-75%): 15% of conversations
- Very high usage (75-100%): 5% of conversations

**Highest utilization**: 0.87 (87%) for "Recover Deleted Local Database" conversation

---

## Data Collection Methods Analyzed

### Method 1: Live Proxy Capture (Current Proof-of-Concept)

**Setup**:
```bash
./scripts/start_warp_proxy_quiet.sh   # Start mitmproxy
# Use WARP normally
./scripts/stop_warp_proxy.sh           # Stop and analyze
```

**Pros**:
- Real-time capture during usage
- No authentication needed
- Useful for schema discovery

**Cons**:
- Requires proxy configuration in WARP
- Must run during WARP usage
- No historical data on first run
- User friction

**Verdict**: Good for development/testing, not ideal for production.

### Method 2: Direct API Query (Recommended)

**Concept**:
```rust
// Query GraphQL API directly
let response = client
    .post("https://app.warp.dev/graphql/v2?op=GetConversationUsage")
    .header("Authorization", format!("Bearer {}", auth_token))
    .send()
    .await?;
```

**Pros**:
- No proxy needed
- Access to full historical data (1+ months)
- Can run periodically (cron/background)
- Simpler user experience

**Cons**:
- Requires auth token extraction
- Need to handle API rate limits
- Token may expire

**Verdict**: Best long-term solution. Requires initial token setup, then seamless.

### Method 3: Hybrid Approach (Optimal)

**Strategy**:
1. Initial proxy capture → extract auth token
2. Save token to `~/.config/splitrail/config.toml`
3. Query API daily/hourly for updates
4. Track `lastUpdated` timestamps for incremental sync

**Benefits**:
- Best of both worlds
- Historical data on first run
- Automatic updates
- Resilient to API changes

**Verdict**: Recommended for Splitrail integration.

---

## Sample Data Statistics

### Conversation Breakdown

**Total conversations analyzed**: 208

**By credit cost**:
- 0-10 credits (quick fixes): 85 conversations (41%)
- 10-100 credits (standard tasks): 95 conversations (46%)
- 100-500 credits (complex features): 23 conversations (11%)
- 500+ credits (major projects): 5 conversations (2%)

**Top 10 Most Expensive Conversations**:

| Rank | Title | Credits | Est. USD | Models Used |
|------|-------|---------|----------|-------------|
| 1 | Migrate Project to Dark Mode Styling | 2,211.6 | $490 | Gemini 3 Pro, Claude 4.5, GPT-5 |
| 2 | Recover Deleted Local Database | 1,586.8 | $352 | Claude 4.5 Sonnet |
| 3 | Investigate Facility Layout Dragging Lag | 1,169.8 | $259 | GPT-5, Claude 4.5 Sonnet |
| 4 | Determine Pricing for New Crypto Token | 1,081.7 | $240 | Claude 4.5, GPT-5 |
| 5 | View Local SQLite Database Contents | 727.5 | $161 | Claude 4.5 Haiku, Sonnet |
| 6 | Fix Duplicate LLM Streaming Output | 412.5 | $91 | Gemini 3 Pro, Claude, GPT-5 |
| 7 | Define Edit Button Behavior | 388.4 | $86 | Claude 4.5 Sonnet, Haiku, GPT-5 |
| 8 | Investigate LLM Non-Response to Fix Messages | 342.9 | $76 | Gemini 3 Pro, Claude, GPT-5 |
| 9 | Locate KIRO CLI Stored Files | 260.2 | $58 | Claude 4.5 Haiku, Sonnet |
| 10 | Design Grid Layout For Six Teams | 246.0 | $54 | Claude 4.5 Sonnet |

**Average credits per conversation**: ~86 credits (~$19)

### Model Usage Distribution

**By total tokens processed**:

| Model | Total Tokens | % of Total | Conversations |
|-------|--------------|------------|---------------|
| Claude 4.5 Sonnet | 412,136,692 | 85.2% | 176 |
| Claude 4.5 Haiku | 39,580,817 | 8.2% | 42 |
| Gemini 3 Pro | 21,003,930 | 4.3% | 12 |
| GPT-5 (medium reasoning) | 4,189,951 | 0.9% | 18 |
| GPT-5 (low reasoning) | 1,433,727 | 0.3% | 2 |
| Claude 4 Sonnet | 1,611,264 | 0.3% | 8 |
| Claude 3.5 Sonnet | 16,177,111 | 0.3% | 1 |
| Gemini 2.5 Flash | 749,752 | 0.2% | 34 |
| GLM 4.6 | 1,173,176 | 0.2% | 1 |

**Insight**: Claude 4.5 Sonnet dominates usage, but reasoning models (GPT-5) used for high-complexity tasks.

### Tool Usage Patterns

**Most common operations** (from high-value conversations):
1. **File reads**: 60+ per major conversation
2. **Shell commands**: 50-70 per debugging session
3. **File diffs**: 85 diffs, 1,200+ lines added typical for features
4. **Grep searches**: 10-20 per investigation

**Productivity metrics**:
- Average lines added per conversation: ~200 (for code-heavy tasks)
- Average files changed: 2-4
- Net positive lines (added - removed): +500 typical

---

## GraphQL API Details

### Primary Endpoint
```
POST https://app.warp.dev/graphql/v2?op=GetConversationUsage
```

### Authentication
```http
Authorization: Bearer eyJhbGc...
```

### Key Operations Observed

1. **GetRequestLimitInfo** (34 occurrences)
   - Returns quota and usage limits
   - Request limit: 10,000/month
   - Voice/autosuggestion limits
   - Codebase index limits

2. **GetConversationUsage** (1 occurrence, returns all 208 conversations)
   - Full conversation history
   - Per-conversation metrics
   - Tool usage breakdown
   - Model-specific token counts

3. **GetFeatureModelChoices** (4 occurrences)
   - Available models for workspace
   - Model capabilities

4. **UpdateGenericStringObject** (6 occurrences)
   - Configuration updates

5. **GetWorkspacesMetadataForUser** (4 occurrences)
   - Workspace information

### Response Format

```json
{
  "data": {
    "user": {
      "__typename": "UserOutput",
      "user": {
        "conversationUsage": [
          {
            "conversationId": "uuid",
            "lastUpdated": "2025-11-24T06:36:23Z",
            "title": "Conversation Title",
            "usageMetadata": {
              "contextWindowUsage": 0.50455,
              "creditsSpent": 1169.79,
              "summarized": true,
              "tokenUsage": [
                {"modelId": "claude 4.5 sonnet", "totalTokens": 35608828}
              ],
              "toolUsageMetadata": {
                "runCommandsExecuted": 69,
                "readFilesStats": {"count": 62},
                "applyFileDiffStats": {
                  "count": 85,
                  "linesAdded": 1201,
                  "linesRemoved": 629,
                  "filesChanged": 4
                }
              }
            }
          }
        ]
      }
    }
  }
}
```

---

## Limitations & Challenges

### 1. No Input/Output Token Split
- API returns `totalTokens` only
- Cannot distinguish input vs output costs
- Must use average pricing for cost estimation

### 2. Credits Are Opaque
- No documented conversion to USD
- Must reverse-engineer from known model pricing
- Conversion rate may change over time

### 3. Authentication Required
- Requires extracting bearer token from requests
- Token may expire (unknown duration)
- No documented OAuth flow

### 4. Unknown API Limits
- Rate limits not documented
- Don't know max query size
- Unclear if pagination is needed for large datasets

### 5. Limited Conversation Content
- Only titles and stats, not full messages
- Cannot analyze actual prompts/responses
- Limits debugging and analysis

---

## Implementation Roadmap

### Phase 1: Foundation (Completed ✅)
- [x] Analyze data structure via proxy capture
- [x] Document GraphQL schema
- [x] Calculate token-to-credits correlation
- [x] Create collection strategy document

### Phase 2: Basic Parsing (Next)
- [ ] Define Rust structs for GraphQL response
- [ ] Parse captured JSONL files
- [ ] Map to Splitrail `ConversationMessage` format
- [ ] Add unit tests

### Phase 3: Analyzer Integration
- [ ] Extend existing `warp_dev.rs`
- [ ] Discover JSONL files in `schemas/warp/captured/`
- [ ] Merge GraphQL data with terminal telemetry
- [ ] Deduplication by conversation ID
- [ ] Display in TUI

### Phase 4: API Client
- [ ] Implement direct GraphQL query
- [ ] Auth token management
- [ ] Error handling & retries
- [ ] Configuration in `config.toml`

### Phase 5: Production Features
- [ ] Incremental sync via `lastUpdated`
- [ ] Background polling (cron/daemon)
- [ ] Token refresh handling
- [ ] Splitrail Cloud upload

---

## Comparison: WARP vs Other CLIs

| Feature | Claude Code | Codex CLI | Gemini CLI | **WARP** |
|---------|-------------|-----------|------------|----------|
| **Data Source** | Local JSONL | Local JSONL | Local JSON | **Cloud API** |
| **Historical** | All time (local) | All time (local) | All time (local) | **1+ month** |
| **Auth Required** | ❌ | ❌ | ❌ | **✅** |
| **Token Detail** | Input/output/cache | Input/output/cache | Input/output/thoughts | **Total only** |
| **Cost Format** | USD (calculated) | USD (calculated) | USD (calculated) | **Credits (direct)** |
| **Tool Tracking** | Basic tool calls | Shell commands | File operations | **Comprehensive** |
| **Code Metrics** | ❌ | ❌ | replace() only | **✅ Lines +/-** |
| **Multi-Model** | ❌ (single) | ✅ (o1/GPT) | ✅ (Gemini variants) | **✅ (9+ models)** |
| **File Operations** | Read/Write/Edit | Read/Write | Read/replace | **Read with stats** |
| **Context Tracking** | ❌ | ❌ | ❌ | **✅ Window usage** |
| **Productivity** | ❌ | ❌ | ❌ | **✅ Lines added** |

**Winner**: WARP has the richest analytics, but requires authentication.

---

## Recommendations

### For Splitrail Development

1. **Start with captured files**
   - Parse existing `schemas/warp/captured/*.jsonl` files
   - Get basic integration working
   - Validate data mapping

2. **Implement API client next**
   - Higher priority than other features
   - Unlocks historical data access
   - Better user experience than proxy

3. **Add to dashboard**
   - Show WARP alongside Claude Code, Codex, etc.
   - Highlight unique metrics (lines added, context usage)
   - Display credit spend + USD estimate

4. **Consider official partnership**
   - Reach out to WARP team
   - Request documented API
   - Explore deeper integration

### For Users

1. **Try proxy capture first**
   ```bash
   cd ~/Projects/splitrail
   ./scripts/start_warp_proxy_quiet.sh
   # Use WARP normally for a session
   ./scripts/stop_warp_proxy.sh
   # View captured data
   splitrail  # (once analyzer is implemented)
   ```

2. **Extract auth token manually**
   - Use browser DevTools on WARP web interface
   - Copy `Authorization` header
   - Save to `~/.config/splitrail/config.toml`

3. **Run periodic syncs**
   - Daily or weekly API queries
   - Track credit spend over time
   - Monitor productivity metrics

---

## Business Insights

### Cost Analysis

**For a typical power user** (based on sample data):
- 208 conversations/month
- 17,860 credits/month
- **~$3,950/month** estimated spend
- Average $19/conversation

**Cost breakdown by task type**:
- Quick fixes (<10 credits): $2/task
- Standard tasks (10-100 credits): $15/task
- Complex features (100-500 credits): $150/task
- Major projects (500+ credits): $500+/task

**ROI considerations**:
- If conversations save 1-2 hours/task
- At $100/hour developer rate
- Break-even at 10+ conversations/month
- Sample user: ~$10,000+ value delivered for $4,000 cost

### Productivity Metrics

**Code output** (based on `applyFileDiffStats`):
- Average 200-500 lines added per feature conversation
- 2-4 files changed per task
- Net positive contribution (more added than removed)

**Tool efficiency**:
- 60+ file reads shows thorough investigation
- 50+ shell commands indicates active debugging
- 10+ greps suggests good code exploration

**Context utilization**:
- Most conversations <50% context (efficient)
- High-value tasks use 70-85% (complex problems)
- Rarely hit 100% limit (good context management)

---

## Next Actions

### Immediate (This Week)
1. ✅ Document findings (this file)
2. ⬜ Update CLAUDE.md with WARP integration notes
3. ⬜ Commit schema and strategy docs
4. ⬜ Create GitHub issue for WARP analyzer implementation

### Short Term (This Month)
1. ⬜ Implement GraphQL response parsing (Rust structs)
2. ⬜ Extend `warp_dev.rs` to read captured files
3. ⬜ Add WARP model pricing to `src/models.rs`
4. ⬜ Test end-to-end with sample data
5. ⬜ Display in TUI

### Medium Term (Next Quarter)
1. ⬜ Implement API client for direct queries
2. ⬜ Add auth token management
3. ⬜ Create background sync daemon
4. ⬜ Splitrail Cloud integration
5. ⬜ Public beta for WARP users

### Long Term (Future)
1. ⬜ Contact WARP team for official API docs
2. ⬜ Request partnership/integration
3. ⬜ OAuth flow for token management
4. ⬜ Real-time webhook support

---

## Files Created

1. **`schemas/warp/WARP_SCHEMA.md`**
   - Complete GraphQL schema documentation
   - Data structures and field definitions
   - Sample queries and responses

2. **`schemas/warp/COLLECTION_STRATEGY.md`**
   - Three collection methods compared
   - Implementation plan (phases 1-6)
   - Authentication strategies
   - Rust code examples

3. **`schemas/warp/ANALYSIS_SUMMARY.md`** (this file)
   - Key findings and statistics
   - Business insights and ROI
   - Recommendations and next actions

4. **`schemas/warp/captured/graphql_responses_20251123.jsonl`**
   - 51 raw GraphQL responses
   - 208 conversations with full metadata
   - Sample data for development

5. **`schemas/warp/captured/usage_data_20251123.jsonl`**
   - 68 extracted usage entries
   - Request limits and bonus grants
   - Workspace information

---

## Conclusion

WARP's GraphQL API provides the most comprehensive AI coding assistant analytics we've seen. The combination of:
- Historical data access (no local file management)
- Direct credit/cost tracking
- Detailed tool usage metrics
- Code modification statistics (lines added/removed)
- Multi-model conversation tracking
- Context window utilization

...makes it a standout data source for Splitrail.

The main challenge is authentication complexity, but the payoff is significant. With proper implementation, WARP integration will provide users with insights no other tool can match.

**Estimated development time**: 10-15 hours for full integration
**Expected user value**: High (unique productivity metrics)
**Implementation priority**: Medium-High (after core stability)

---

**Analysis completed**: November 24, 2025, 2:40 AM
**Proxy runtime**: ~5 minutes
**Data processed**: 51 GraphQL responses, 208 conversations
**Total credits analyzed**: 17,860.7 (~$3,950 USD equivalent)
**Documentation generated**: 1,200+ lines across 4 files

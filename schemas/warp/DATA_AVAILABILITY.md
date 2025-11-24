# Warp Data Availability Analysis

**Date**: 2025-11-23
**Analysis**: Comprehensive review of Warp's local data storage and network logging

## Summary

The rich usage summary shown in Warp's UI (credits spent, models used, tool calls, file changes, etc.) is **NOT stored in local logs**. This data exists only in:
1. GraphQL response bodies transmitted over HTTPS (not logged locally)
2. In-memory during active sessions
3. Server-side on Warp's backend

## Data Sources

### 1. `warp_network.log`
**Location**: `~/Library/Application Support/dev.warp.Warp-Stable/warp_network.log`

**Format**: Plain text log with structured sections for network requests and responses

**What It Contains:**

✅ **Request Data (Full)**
- GraphQL query structures
- Request headers
- Request bodies (query variables, operation names)
- Timestamps

✅ **Response Metadata (Partial)**
- HTTP status codes
- Response headers
- Content-length
- Response timestamps

✅ **Telemetry Batch Events**
- Event type (e.g., `AgentMode.SyncCodebaseContext.Success`)
- Event properties (payload with metrics like sync duration, fragment counts)
- Original timestamps
- User/anonymous IDs
- Session IDs from Amplitude integration

❌ **Response Bodies (Missing)**
- GraphQL response data
- Credit usage details
- Model information
- Tool call statistics
- Token counts
- File operation details
- Diff statistics

### 2. Other Local Files

Searched locations:
- `~/Library/Application Support/dev.warp.Warp-Stable/` (all subdirectories)
- No SQLite databases found
- No JSON/JSONL data files found
- No IndexedDB or local storage files accessible

**Result**: No additional local data sources for usage metrics

## Available Data Points

### From Network Log

1. **GraphQL Queries Sent:**
   - `GetFeatureModelChoices` - Model options with credit multipliers
   - `GetRequestLimitInfo` - Credit limits and usage quotas
   - Agent-related queries

2. **Telemetry Events:**
   - `AgentMode.SyncCodebaseContext.Success`
   - Sync duration metrics
   - Fragment and node counts
   - Cache population status

3. **Timing Information:**
   - Request timestamps
   - Response timestamps
   - Network round-trip time (calculated)

4. **Session Tracking:**
   - Amplitude session IDs
   - User IDs
   - Anonymous IDs

### Unavailable Locally

❌ **Credit/Cost Data:**
- Credits spent per response
- Total credits remaining
- Credit multipliers applied
- Bonus grant balances

❌ **Model Information:**
- Model name used (e.g., "gpt-5", "claude 4.5 sonnet")
- Model reasoning level
- Model provider

❌ **Token Usage:**
- Input tokens
- Output tokens
- Context window utilization percentage
- Cached tokens

❌ **Tool Call Details:**
- Number of tool calls
- Tool types invoked
- Tool execution results

❌ **File Operations:**
- Files changed count
- Lines added/deleted
- Diff statistics

❌ **Commands:**
- Commands executed count
- Command types
- Command outputs

❌ **Response Timing:**
- Time to first token (ms)
- Total agent response time
- Total time including tool calls

## Data Collection Strategies

### Option 1: Local Log Monitoring (Implemented)

**Script**: `scripts/monitor_warp.sh`

**Captures:**
- Incremental log updates using offset tracking
- Daily capture files with telemetry events
- GraphQL query patterns
- Session tracking

**Limitations:**
- Only gets request data and event telemetry
- No response body data
- No detailed usage metrics

**Use Case:** Basic activity tracking, session monitoring

### Option 2: Network Interception (Advanced)

**Tools:** mitmproxy, Charles Proxy, Wireshark

**Requirements:**
- HTTPS proxy setup
- SSL certificate installation
- Warp configured to use proxy

**Captures:**
- Full GraphQL response bodies
- Complete usage statistics
- Real-time credit tracking
- All data shown in UI

**Limitations:**
- Complex setup
- May violate Warp's terms of service
- Requires proxy reconfiguration

**Use Case:** Complete usage analytics, debugging

**Documentation**: See `mitmproxy_setup.md` in Warp data directory

### Option 3: Server-Side API (Future)

**Potential:** Warp may offer API access to usage data

**Would Provide:**
- Historical usage statistics
- Credit consumption reports
- Model usage breakdown
- Authenticated API access

**Status:** Not currently available publicly

## Recommended Approach for Splitrail

### Current Implementation
Track what's available locally:
1. **Telemetry events** - Agent mode activations, sync operations
2. **Session tracking** - Session IDs, timestamps
3. **Activity patterns** - When Warp agent is used
4. **Basic metrics** - Sync durations, fragment counts

### Future Enhancements
1. **Optional mitmproxy integration** - For users who want full metrics
2. **GraphQL response parsing** - If users provide intercepted data
3. **Manual import** - Allow users to paste usage data from UI
4. **Warp API integration** - If official API becomes available

## Tracking Script Usage

### Monitor for New Activity
```bash
# Run periodically (e.g., every 5 minutes via cron)
/Users/bzl/Projects/splitrail/scripts/monitor_warp.sh
```

### View Today's Captured Data
```bash
cat ~/Projects/splitrail/schemas/warp/samples/warp_daily_$(date +%Y%m%d).log
```

### Reset Offset (Start Fresh)
```bash
rm ~/Projects/splitrail/schemas/warp/samples/.warp_offset
```

## Comparison with Other Tools

| Tool | Local Data | Usage Metrics | Cost Tracking |
|------|------------|---------------|---------------|
| Claude Code | ✅ Full JSONL | ✅ Complete | ✅ Token-based |
| Codex CLI | ✅ Full JSONL | ✅ Complete | ✅ Token-based |
| Gemini CLI | ✅ JSON sessions | ✅ Complete | ✅ Token-based |
| GitHub Copilot | ✅ Session JSON | ✅ Partial | ❌ No cost data |
| **Warp** | ⚠️ Partial logs | ❌ Server-side | ❌ Not available |

## Conclusions

1. **Warp stores minimal usage data locally** - Only network request/response metadata and basic telemetry
2. **Rich usage metrics are server-side** - Transmitted in GraphQL responses but not logged
3. **UI data is ephemeral** - Calculated in real-time during sessions, not persisted
4. **Network interception required** - Only way to capture complete usage statistics locally
5. **Limited Splitrail integration** - Can track basic activity but not detailed costs/usage

## Next Steps

1. ✅ Implement basic activity tracking from telemetry events
2. ✅ Set up incremental log monitoring
3. ⏳ Document mitmproxy integration option for advanced users
4. ⏳ Update Warp analyzer to parse available telemetry data
5. ⏳ Add session tracking and activity metrics
6. ⏳ Consider adding manual data import feature

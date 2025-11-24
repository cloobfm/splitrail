# Splitrail Performance Optimization Plan

## Current Architecture Analysis

### Main Loop (TUI)
- **Poll interval**: 250ms (4 times per second)
- **Forced redraw**: Every 1 second for clock updates
- **Stats checking**: Every poll via `stats_receiver.has_changed()`
- **Sparkline refresh**: Every 5 seconds

### Real-time Data Management
- **File watching**: FSEvents for all analyzer directories
- **Reload debounce**: 2 seconds per analyzer
- **Polling**: Codex CLI polled every 5 seconds
- **Upload debounce**: 3 seconds

## Identified Optimization Opportunities

### 1. Adaptive Polling (High Impact)
**Current**: Fixed 250ms polling interval regardless of activity
**Optimization**: Adaptive polling that adjusts based on activity
- Idle (no user input, no file changes): 1000ms (1 FPS)
- Low activity (user viewing): 500ms (2 FPS)
- Active (user input, data changes): 250ms (4 FPS)
- Very active (animations): 100ms (10 FPS)

**Expected improvement**: 50-75% CPU reduction when idle

### 2. Lazy Clock Updates (Medium Impact)
**Current**: Force redraw every second for clock
**Optimization**: Only update clock display if visible on current tab
- Skip clock redraw on tabs without time display
- Use cached formatted time strings

**Expected improvement**: 10-20% CPU reduction

### 3. Cached Aggregations (High Impact)
**Current**: Aggregations recalculated on every stats reload
**Optimization**: Cache aggregated daily stats with invalidation
- Cache `aggregate_by_date` results
- Only recalculate for changed analyzer data
- Store incrementally

**Expected improvement**: 30-40% faster stats updates

### 4. Batch File Watching (Medium Impact)
**Current**: Each file event triggers immediate reload
**Optimization**: Batch multiple file events within debounce window
- Collect events over 2s window
- Single reload for all changes
- Reduce redundant parsing

**Expected improvement**: 20-30% reduction during high file activity

### 5. Optimize String Operations (Low-Medium Impact)
**Current**: String allocations in hot paths (formatting, truncation)
**Optimization**:
- Reuse string buffers
- Use `write!` macro instead of `format!`
- Pre-allocate with capacity

**Expected improvement**: 5-10% reduction in allocations

### 6. Parallel Analyzer Loading (Medium Impact)
**Current**: Analyzers loaded sequentially via `load_all_stats().await`
**Optimization**: Load all analyzers in parallel
- Use `tokio::join!` or `futures::join_all`
- Each analyzer runs independently

**Expected improvement**: 40-60% faster initial load

### 7. Incremental Stats Updates (High Impact)
**Current**: Full reload of analyzer data on file change
**Optimization**: Incremental updates for new messages only
- Track last processed timestamp per analyzer
- Only parse new data
- Merge with existing stats

**Expected improvement**: 70-90% faster file change updates

### 8. Memoized Sparklines (Low Impact)
**Current**: Sparklines recalculated on every draw
**Optimization**: Cache sparkline strings with invalidation
- Cache per-analyzer sparklines
- Only recalculate when data changes

**Expected improvement**: 5-10% faster rendering

### 9. Reduce Polling Frequency (Low Impact)
**Current**: Codex CLI polled every 5 seconds
**Optimization**: Increase interval to 10-15 seconds
- Less aggressive polling
- Add manual refresh keybinding

**Expected improvement**: 5-10% CPU reduction

### 10. Optimize JSON Parsing (Medium Impact)
**Current**: Parse entire files even for incremental updates
**Optimization**:
- Use memory-mapped files for large JSONL
- Stream parsing instead of load-all
- Skip already-processed lines

**Expected improvement**: 30-40% faster parsing

## Implementation Priority

### Phase 1: Quick Wins (1-2 hours)
1. ✅ Fix UTF-8 panic (DONE)
2. ✅ Fix Kilo Code message filtering (DONE)
3. Adaptive polling (src/tui.rs)
4. Lazy clock updates (src/tui.rs)
5. Disable polling via env var check (src/watcher.rs)

### Phase 2: Caching & Aggregation (2-3 hours)
1. Cached aggregations (src/utils.rs)
2. Memoized sparklines (src/tui/summary_dashboard.rs)
3. Batch file watching (src/watcher.rs)

### Phase 3: Incremental Updates (3-4 hours)
1. Incremental stats tracking
2. Timestamp-based filtering
3. Merge strategies

### Phase 4: Parallel & Advanced (2-3 hours)
1. Parallel analyzer loading
2. Memory-mapped file parsing
3. Stream parsing optimization

## Measurement Plan

### Baseline Metrics (Current)
- Startup time: ?ms
- Idle CPU usage: ?%
- Memory usage: ?MB
- Stats reload time: ?ms

### Target Metrics
- Startup time: <1000ms (50% improvement)
- Idle CPU usage: <2% (75% reduction)
- Memory usage: Similar or better
- Stats reload time: <100ms (80% improvement)

### Benchmarking Strategy
1. Run `scripts/profile_cpu.sh` for baseline
2. Implement Phase 1 optimizations
3. Re-run profiling, compare results
4. Document improvements
5. Repeat for each phase

## Configuration Options

Add environment variables for testing:
- `SPLITRAIL_POLL_INTERVAL_MS`: Override default 250ms
- `SPLITRAIL_DISABLE_POLLING`: Disable Codex CLI polling
- `SPLITRAIL_DISABLE_CLOCK`: Disable forced clock redraws
- `SPLITRAIL_DEBUG_PERF`: Enable performance logging

## Notes

- Focus on idle CPU first (biggest user-visible impact)
- Maintain responsiveness during active use
- No functionality regressions
- Backward compatible with existing configs

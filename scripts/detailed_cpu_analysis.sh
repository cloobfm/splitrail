#!/bin/bash

echo "🔬 Detailed CPU Analysis"
echo "========================="

# Build with debug symbols for profiling
echo "📦 Building with debug symbols..."
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release --quiet

echo ""
echo "📊 Test 1: CPU at idle (no file changes)"
echo "--------------------------------------------"

# Start splitrail and measure CPU for 15 seconds
timeout 15s ./target/release/splitrail-dashboard 2>/dev/null &
PID=$!
sleep 2  # Let it stabilize

total_cpu=0
samples=0
for i in {1..10}; do
    cpu=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ')
    if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
        total_cpu=$((total_cpu + $(echo "$cpu" | cut -d. -f1)))
        samples=$((samples + 1))
        echo "  Sample $i: ${cpu}%"
    fi
    sleep 1
done

if [ $samples -gt 0 ]; then
    avg_idle_cpu=$((total_cpu / samples))
    echo "  Average idle CPU: ${avg_idle_cpu}%"
else
    echo "  Could not measure idle CPU"
fi

kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

echo ""
echo "📊 Test 2: CPU during simulated file changes"
echo "----------------------------------------------"

# Start splitrail again
timeout 20s ./target/release/splitrail-dashboard 2>/dev/null &
PID=$!
sleep 2

# Find and modify a file to trigger file watcher
CLAUDE_FILE=$(find "$HOME/.claude" -name "*.jsonl" 2>/dev/null | head -1)

if [ -n "$CLAUDE_FILE" ] && [ -f "$CLAUDE_FILE" ]; then
    echo "  Modifying file: $CLAUDE_FILE"
    
    # Measure CPU during rapid file changes
    total_cpu=0
    samples=0
    
    for i in {1..15}; do
        # Trigger file change every second
        echo "# Change $i at $(date)" >> "$CLAUDE_FILE"
        
        # Measure CPU immediately after change
        sleep 0.5
        cpu=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ')
        if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
            total_cpu=$((total_cpu + $(echo "$cpu" | cut -d. -f1)))
            samples=$((samples + 1))
            echo "  After change $i: ${cpu}%"
        fi
        sleep 0.5
    done
    
    if [ $samples -gt 0 ]; then
        avg_active_cpu=$((total_cpu / samples))
        echo "  Average active CPU: ${avg_active_cpu}%"
    fi
    
    # Clean up the file
    head -n -1 "$CLAUDE_FILE" > "${CLAUDE_FILE}.tmp" && mv "${CLAUDE_FILE}.tmp" "$CLAUDE_FILE"
else
    echo "  No Claude Code files found to modify"
fi

kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

echo ""
echo "📊 Test 3: Peak CPU during startup"
echo "-----------------------------------"

# Measure CPU during startup
timeout 10s ./target/release/splitrail-dashboard 2>/dev/null &
PID=$!

max_cpu=0
for i in {1..20}; do
    cpu=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ')
    if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
        cpu_int=$(echo "$cpu" | cut -d. -f1)
        if [ $cpu_int -gt $max_cpu ]; then
            max_cpu=$cpu_int
        fi
        echo "  Startup sample $i: ${cpu}%"
    fi
    sleep 0.25
done

echo "  Peak startup CPU: ${max_cpu}%"

kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

echo ""
echo "📊 Test 4: Memory usage over time"
echo "---------------------------------"

timeout 10s ./target/release/splitrail-dashboard 2>/dev/null &
PID=$!
sleep 1

for i in {1..8}; do
    mem=$(ps -p $PID -o rss= 2>/dev/null | tr -d ' ')
    if [ -n "$mem" ] && [ "$mem" != "" ]; then
        mem_mb=$((mem / 1024))
        echo "  Memory sample $i: ${mem_mb}MB"
    fi
    sleep 1
done

kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

echo ""
echo "✅ Analysis Complete"
echo ""
echo "🎯 Key Findings:"
if [ -n "${avg_idle_cpu:-}" ]; then
    echo "  - Idle CPU: ${avg_idle_cpu}%"
fi
if [ -n "${avg_active_cpu:-}" ]; then
    echo "  - Active CPU: ${avg_active_cpu}%"
fi
echo "  - Peak CPU: ${max_cpu}%"
echo "  - Memory usage: Measured"

echo ""
echo "🔍 Analysis:"
if [ -n "${avg_idle_cpu:-}" ] && [ $avg_idle_cpu -gt 10 ]; then
    echo "  ⚠️  High idle CPU detected - investigate background processing"
fi
if [ -n "${avg_active_cpu:-}" ] && [ $avg_active_cpu -gt 50 ]; then
    echo "  ⚠️  High active CPU - file watcher may be too aggressive"
fi
if [ $max_cpu -gt 100 ]; then
    echo "  ⚠️  Very high peak CPU - optimization needed"
fi
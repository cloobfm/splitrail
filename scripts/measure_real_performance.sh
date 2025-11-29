#!/bin/bash

echo "🔬 Real-World Performance Data Collection"
echo "======================================"

# Start splitrail in background
echo "🚀 Starting splitrail..."
./target/release/splitrail-dashboard &
SPLITRAIL_PID=$!
echo "  PID: $SPLITRAIL_PID"

# Wait for app to stabilize
echo "⏳ Waiting for app to stabilize (5 seconds)..."
sleep 5

echo ""
echo "📊 Measuring Baseline CPU (10 seconds)..."
echo "------------------------------------------"

baseline_total=0
baseline_samples=0
for i in {1..10}; do
    cpu=$(ps -p $SPLITRAIL_PID -o %cpu= 2>/dev/null | tr -d ' ')
    if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
        cpu_int=$(echo "$cpu" | cut -d. -f1)
        baseline_total=$((baseline_total + cpu_int))
        baseline_samples=$((baseline_samples + 1))
        echo "  Sample $i: ${cpu}%"
    fi
    sleep 1
done

if [ $baseline_samples -gt 0 ]; then
    baseline_avg=$((baseline_total / baseline_samples))
    echo "  Baseline average CPU: ${baseline_avg}%"
else
    echo "  ❌ Could not measure baseline CPU"
    exit 1
fi

echo ""
echo "📁 Triggering File Changes..."
echo "----------------------------"

# Find a Claude Code file to modify
CLAUDE_FILE=$(find "$HOME/.claude" -name "conversations_*.jsonl" 2>/dev/null | head -1)

if [ -n "$CLAUDE_FILE" ] && [ -f "$CLAUDE_FILE" ]; then
    echo "  Using file: $CLAUDE_FILE"
    
    # Create backup
    cp "$CLAUDE_FILE" "${CLAUDE_FILE}.backup"
    
    echo ""
    echo "📊 Measuring CPU During File Changes (15 seconds)..."
    echo "----------------------------------------------------"
    
    active_total=0
    active_samples=0
    peak_cpu=0
    
    # Make rapid changes to simulate Claude Code activity
    for i in {1..5}; do
        echo "  📝 Change $i: Adding line to file..."
        echo "{\"type\": \"message\", \"timestamp\": \"$(date -Iseconds)\", \"content\": \"Test change $i\"}" >> "$CLAUDE_FILE"
        
        # Measure CPU for 3 seconds after each change
        for j in {1..3}; do
            cpu=$(ps -p $SPLITRAIL_PID -o %cpu= 2>/dev/null | tr -d ' ')
            if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
                cpu_int=$(echo "$cpu" | cut -d. -f1)
                active_total=$((active_total + cpu_int))
                active_samples=$((active_samples + 1))
                
                if [ $cpu_int -gt $peak_cpu ]; then
                    peak_cpu=$cpu_int
                fi
                
                echo "    CPU after change $i.${j}: ${cpu}%"
            fi
            sleep 1
        done
        
        echo "  ⏳ Waiting 2 seconds..."
        sleep 2
    done
    
    if [ $active_samples -gt 0 ]; then
        active_avg=$((active_total / active_samples))
        echo "  Active average CPU: ${active_avg}%"
        echo "  Peak CPU during changes: ${peak_cpu}%"
    fi
    
    # Restore original file
    mv "${CLAUDE_FILE}.backup" "$CLAUDE_FILE"
    
else
    echo "  ❌ No Claude Code files found"
    echo "  📝 Creating test file instead..."
    
    TEST_DIR="$HOME/.claude/projects/test-project"
    mkdir -p "$TEST_DIR"
    TEST_FILE="$TEST_DIR/conversations_test.jsonl"
    echo '{"type": "message", "timestamp": "2024-01-01T00:00:00Z", "content": "test"}' > "$TEST_FILE"
    
    echo ""
    echo "📊 Measuring CPU During Test File Changes..."
    echo "--------------------------------------------"
    
    # Same measurement with test file
    for i in {1..3}; do
        echo "  📝 Change $i: Modifying test file..."
        echo '{"type": "message", "timestamp": "'$(date -Iseconds)'", "content": "Test change $i"}' >> "$TEST_FILE"
        
        for j in {1..3}; do
            cpu=$(ps -p $SPLITRAIL_PID -o %cpu= 2>/dev/null | tr -d ' ')
            if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
                cpu_int=$(echo "$cpu" | cut -d. -f1)
                echo "    CPU: ${cpu}%"
            fi
            sleep 1
        done
        sleep 1
    done
fi

echo ""
echo "📊 Final Memory Measurement..."
echo "------------------------------"

# Get final memory usage
mem=$(ps -p $SPLITRAIL_PID -o rss= 2>/dev/null | tr -d ' ')
if [ -n "$mem" ] && [ "$mem" != "" ]; then
    mem_mb=$((mem / 1024))
    echo "  Final RSS memory: ${mem_mb}MB"
fi

# Clean up
echo ""
echo "🧹 Cleaning up..."
kill $SPLITRAIL_PID 2>/dev/null || true
wait $SPLITRAIL_PID 2>/dev/null || true

echo ""
echo "✅ Data Collection Complete"
echo ""
echo "📈 RESULTS SUMMARY:"
echo "==================="
echo "  Baseline CPU: ${baseline_avg}%"
if [ -n "${active_avg:-}" ]; then
    echo "  Active CPU: ${active_avg}%"
fi
if [ -n "${peak_cpu:-}" ]; then
    echo "  Peak CPU: ${peak_cpu}%"
fi
if [ -n "${mem_mb:-}" ]; then
    echo "  Memory Usage: ${mem_mb}MB"
fi

echo ""
echo "🎯 ANALYSIS:"
if [ -n "${baseline_avg:-}" ] && [ $baseline_avg -gt 15 ]; then
    echo "  ⚠️  HIGH baseline CPU - investigate background processing"
fi
if [ -n "${active_avg:-}" ] && [ $active_avg -gt 50 ]; then
    echo "  ⚠️  HIGH active CPU - file watcher too aggressive"
fi
if [ -n "${peak_cpu:-}" ] && [ $peak_cpu -gt 100 ]; then
    echo "  ⚠️  VERY HIGH peak CPU - optimization needed"
fi
if [ -n "${baseline_avg:-}" ] && [ $baseline_avg -lt 10 ] && [ -n "${active_avg:-}" ] && [ $active_avg -gt 40 ]; then
    echo "  ✅ Baseline OK but spikes during file changes - target file watcher optimization"
fi

echo ""
echo "📝 Next steps:"
echo "  1. If baseline > 15%: Check TUI polling, background tasks"
echo "  2. If active > 50%: Optimize file re-parsing logic"
echo "  3. If peak > 100%: Implement batching/caching"
echo "  4. Memory < 50MB: Good, focus on CPU optimization"
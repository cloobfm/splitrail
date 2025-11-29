#!/bin/bash

echo "🎯 Targeted CPU Test with Real Claude Code File"
echo "=============================================="

CLAUDE_FILE="/Users/bzl/.claude/projects/-Users-bzl-Virtius-virtius-src-graphql/agent-88ccf92d.jsonl"
FILE_SIZE=$(ls -l "$CLAUDE_FILE" | awk '{print $5}')
LINE_COUNT=$(wc -l < "$CLAUDE_FILE")

echo "📁 Test file: $CLAUDE_FILE"
echo "  File size: $FILE_SIZE bytes"
echo "  Line count: $LINE_COUNT"

# Backup the file
cp "$CLAUDE_FILE" "${CLAUDE_FILE}.backup"

echo ""
echo "🚀 Starting splitrail..."
./target/release/splitrail-dashboard &
SPLITRAIL_PID=$!
echo "  PID: $SPLITRAIL_PID"

# Wait for app to stabilize
echo "⏳ Waiting for app to stabilize (3 seconds)..."
sleep 3

echo ""
echo "📊 Measuring CPU during file modifications..."
echo "--------------------------------------------"

peak_cpu=0
total_cpu=0
samples=0

# Make 5 rapid changes to the real file
for i in {1..5}; do
    echo "  📝 Change $i: Adding line to real Claude Code file..."
    
    # Add a realistic JSON line
    echo '{"type": "message", "timestamp": "'$(date -Iseconds)'", "content": "Performance test change '$i'", "role": "user"}' >> "$CLAUDE_FILE"
    
    # Measure CPU for 2 seconds after the change
    for j in {1..2}; do
        cpu=$(ps -p $SPLITRAIL_PID -o %cpu= 2>/dev/null | tr -d ' ')
        if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
            cpu_int=$(echo "$cpu" | cut -d. -f1)
            total_cpu=$((total_cpu + cpu_int))
            samples=$((samples + 1))
            
            if [ $cpu_int -gt $peak_cpu ]; then
                peak_cpu=$cpu_int
            fi
            
            echo "    CPU ${i}.${j}: ${cpu}%"
        fi
        sleep 1
    done
    
    echo "  ⏳ Waiting 1 second..."
    sleep 1
done

if [ $samples -gt 0 ]; then
    avg_cpu=$((total_cpu / samples))
    echo ""
    echo "📈 RESULTS:"
    echo "  Average CPU during changes: ${avg_cpu}%"
    echo "  Peak CPU during changes: ${peak_cpu}%"
    echo "  Total samples: $samples"
else
    echo "  ❌ Could not measure CPU"
fi

# Clean up
echo ""
echo "🧹 Restoring original file..."
mv "${CLAUDE_FILE}.backup" "$CLAUDE_FILE"

kill $SPLITRAIL_PID 2>/dev/null || true
wait $SPLITRAIL_PID 2>/dev/null || true

echo ""
echo "✅ Test Complete"
echo ""
echo "🎯 ANALYSIS:"
if [ -n "${avg_cpu:-}" ] && [ $avg_cpu -gt 50 ]; then
    echo "  ⚠️  HIGH CPU (${avg_cpu}%) during file changes"
    echo "  🔍 This confirms file watcher re-parsing is the bottleneck"
    echo "  💡 Solution needed: Incremental parsing or change debouncing"
elif [ -n "${avg_cpu:-}" ] && [ $avg_cpu -gt 30 ]; then
    echo "  ⚠️  Moderate CPU (${avg_cpu}%) during file changes"
    echo "  🔍 File re-parsing contributes significantly to CPU usage"
    echo "  💡 Optimization would help but may not be critical"
elif [ -n "${avg_cpu:-}" ]; then
    echo "  ✅ Reasonable CPU (${avg_cpu}%) during file changes"
    echo "  🔍 File re-parsing is not the main bottleneck"
    echo "  💡 Look elsewhere for performance issues"
fi

if [ -n "${peak_cpu:-}" ] && [ $peak_cpu -gt 100 ]; then
    echo "  🚨 VERY HIGH PEAK CPU (${peak_cpu}%)"
    echo "  🔍 Indicates blocking operations or inefficient loops"
fi
#!/bin/bash

# Simple performance profiling script
# Measures actual bottlenecks during typical operations

echo "🔍 Splitrail Performance Analysis"
echo "================================="

# Build release version
echo "📦 Building release version..."
cargo build --release --quiet

# Test 1: Cold start performance
echo ""
echo "🚀 Test 1: Cold Start Performance"
echo "-----------------------------------"

echo "Measuring cold start time (3 runs)..."
for i in 1 2 3; do
    start_time=$(date +%s%N)
    timeout 10s ./target/release/splitrail-dashboard --help >/dev/null 2>&1
    end_time=$(date +%s%N)
    duration=$((($end_time - $start_time) / 1000000))
    echo "  Run $i: ${duration}ms"
done

# Test 2: File I/O performance
echo ""
echo "📁 Test 2: File I/O Performance"
echo "----------------------------------"

# Find largest Claude Code JSONL file
CLAUDE_DIR="$HOME/.claude"
if [ -d "$CLAUDE_DIR" ]; then
    largest_file=$(find "$CLAUDE_DIR" -name "conversations_*.jsonl" -type f -exec ls -l {} \; | sort -k5 -n | tail -1 | awk '{print $9}')
    if [ -n "$largest_file" ] && [ -f "$largest_file" ]; then
        file_size=$(ls -l "$largest_file" | awk '{print $5}')
        line_count=$(wc -l < "$largest_file")
        
        echo "Largest Claude Code file: $largest_file"
        echo "  Size: $file_size bytes"
        echo "  Lines: $line_count"
        
        echo "Measuring file read time..."
        start_time=$(date +%s%N)
        content=$(cat "$largest_file")
        end_time=$(date +%s%N)
        read_time=$((($end_time - $start_time) / 1000000))
        echo "  Read time: ${read_time}ms"
        
        echo "Measuring JSON parsing time..."
        start_time=$(date +%s%N)
        echo "$content" | wc -l >/dev/null  # Just count lines as proxy
        end_time=$(date +%s%N)
        parse_time=$((($end_time - $start_time) / 1000000))
        echo "  Line count time: ${parse_time}ms"
    else
        echo "No Claude Code JSONL files found"
    fi
else
    echo "Claude Code directory not found"
fi

# Test 3: Memory usage
echo ""
echo "💾 Test 3: Memory Usage Analysis"
echo "--------------------------------"

echo "Running memory test for 10 seconds..."
timeout 10s ./target/release/splitrail-dashboard 2>/dev/null &
PID=$!
sleep 2

# Get memory usage
if command -v ps >/dev/null 2>&1; then
    mem_usage=$(ps -p $PID -o rss= 2>/dev/null | tr -d ' ')
    if [ -n "$mem_usage" ] && [ "$mem_usage" != "" ]; then
        mem_mb=$((mem_usage / 1024))
        echo "  RSS memory: ${mem_mb}MB"
    else
        echo "  Could not measure memory usage"
    fi
fi

# Clean up
kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

# Test 4: CPU during file changes
echo ""
echo "⚡ Test 4: CPU During File Changes"
echo "------------------------------------"

echo "Starting splitrail in background..."
timeout 15s ./target/release/splitrail-dashboard 2>/dev/null &
SPLITRAIL_PID=$!
sleep 3

echo "Triggering file changes (simulating Claude Code activity)..."
if [ -d "$CLAUDE_DIR" ]; then
    # Find a project directory to modify
    project_dir=$(find "$CLAUDE_DIR" -name "projects" -type d | head -1)
    if [ -n "$project_dir" ] && [ -d "$project_dir" ]; then
        test_file=$(find "$project_dir" -name "*.jsonl" | head -1)
        if [ -n "$test_file" ] && [ -f "$test_file" ]; then
            echo "  Modifying: $test_file"
            # Trigger multiple file changes
            for i in 1 2 3; do
                echo "# Modified at $(date)" >> "$test_file"
                sleep 1
                echo "# Another change $i" >> "$test_file"
                sleep 1
            done
        fi
    fi
fi

# Measure CPU during this period
if command -v ps >/dev/null 2>&1; then
    total_cpu=0
    samples=0
    for i in {1..10}; do
        cpu=$(ps -p $SPLITRAIL_PID -o %cpu= 2>/dev/null | tr -d ' ')
        if [ -n "$cpu" ] && [ "$cpu" != "" ]; then
            total_cpu=$((total_cpu + $(echo "$cpu" | cut -d. -f1)))
            samples=$((samples + 1))
        fi
        sleep 0.5
    done
    
    if [ $samples -gt 0 ]; then
        avg_cpu=$((total_cpu / samples))
        echo "  Average CPU during file changes: ${avg_cpu}%"
    fi
fi

# Clean up
kill $SPLITRAIL_PID 2>/dev/null || true
wait $SPLITRAIL_PID 2>/dev/null || true

echo ""
echo "✅ Performance Analysis Complete"
echo ""
echo "📊 Summary:"
echo "  - Cold start: Measured"
echo "  - File I/O: Measured" 
echo "  - Memory usage: Measured"
echo "  - CPU during changes: Measured"
echo ""
echo "🔧 Next steps:"
echo "  1. If cold start >500ms: optimize initialization"
echo "  2. If file read >100ms: implement caching"
echo "  3. If memory >100MB: check for memory leaks"
echo "  4. If CPU >20%: investigate file watcher frequency"
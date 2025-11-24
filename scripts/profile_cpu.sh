#!/bin/bash
# CPU profiling script for splitrail-dashboard
# Measures CPU usage, startup time, and idle performance

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RESULTS_DIR="$PROJECT_DIR/perf_results"

mkdir -p "$RESULTS_DIR"

echo "======================================"
echo "Splitrail Performance Profiling"
echo "======================================"
echo ""

# Build release binary
echo "Building release binary..."
cd "$PROJECT_DIR"
cargo build --release --quiet

BINARY="$PROJECT_DIR/target/release/splitrail-dashboard"

# Test 1: Measure startup time and initial load
echo ""
echo "Test 1: Startup Time & Initial Load"
echo "------------------------------------"
STARTUP_LOG="$RESULTS_DIR/startup_time.log"
> "$STARTUP_LOG"

for i in {1..5}; do
    echo -n "  Run $i: "
    # Use time command for accurate measurement
    TIME_OUTPUT=$( { time timeout 5s "$BINARY" > /dev/null 2>&1; } 2>&1 )
    # Extract real time in seconds
    DURATION=$(echo "$TIME_OUTPUT" | grep real | awk '{print $2}' | sed 's/[^0-9.]//g')
    # Convert to ms (multiply by 1000)
    DURATION_MS=$(echo "$DURATION * 1000" | bc | cut -d. -f1)
    echo "${DURATION_MS}ms" | tee -a "$STARTUP_LOG"
    sleep 1
done

# Calculate average
AVG_STARTUP=$(awk '{ sum += $1; n++ } END { if (n > 0) print sum / n; }' "$STARTUP_LOG")
echo "  Average: ${AVG_STARTUP}ms"

# Test 2: Memory usage profiling
echo ""
echo "Test 2: Memory Usage"
echo "------------------------------------"
MEMORY_LOG="$RESULTS_DIR/memory_usage.log"

# Run for 10 seconds and sample memory every second
echo "  Starting memory profiling (10s sample)..."
timeout 10s "$BINARY" > /dev/null 2>&1 &
PID=$!

sleep 1  # Give it time to start

# Sample memory usage
> "$MEMORY_LOG"
for i in {1..9}; do
    if ps -p $PID > /dev/null 2>&1; then
        MEM=$(ps -o rss= -p $PID 2>/dev/null || echo "0")
        MEM_MB=$((MEM / 1024))
        echo "$i ${MEM_MB}MB" >> "$MEMORY_LOG"
        sleep 1
    fi
done

# Kill if still running
kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

if [ -s "$MEMORY_LOG" ]; then
    AVG_MEM=$(awk '{ sum += $2; n++ } END { if (n > 0) print sum / n; }' "$MEMORY_LOG" | sed 's/MB//')
    MAX_MEM=$(awk '{ gsub(/MB/, "", $2); if ($2 > max) max = $2 } END { print max }' "$MEMORY_LOG")
    echo "  Average memory: ${AVG_MEM}MB"
    echo "  Peak memory: ${MAX_MEM}MB"
else
    echo "  Could not measure memory usage"
fi

# Test 3: CPU usage during idle
echo ""
echo "Test 3: CPU Usage (Idle)"
echo "------------------------------------"
CPU_LOG="$RESULTS_DIR/cpu_usage.log"

echo "  Starting CPU profiling (10s sample)..."
timeout 10s "$BINARY" > /dev/null 2>&1 &
PID=$!

sleep 2  # Let it stabilize

# Sample CPU usage
> "$CPU_LOG"
for i in {1..8}; do
    if ps -p $PID > /dev/null 2>&1; then
        # Get CPU percentage (macOS)
        CPU=$(ps -o %cpu= -p $PID 2>/dev/null | awk '{print $1}' || echo "0")
        echo "$i ${CPU}%" >> "$CPU_LOG"
        sleep 1
    fi
done

kill $PID 2>/dev/null || true
wait $PID 2>/dev/null || true

if [ -s "$CPU_LOG" ]; then
    AVG_CPU=$(awk '{ gsub(/%/, "", $2); sum += $2; n++ } END { if (n > 0) print sum / n; }' "$CPU_LOG")
    MAX_CPU=$(awk '{ gsub(/%/, "", $2); if ($2 > max) max = $2 } END { print max }' "$CPU_LOG")
    echo "  Average CPU: ${AVG_CPU}%"
    echo "  Peak CPU: ${MAX_CPU}%"
else
    echo "  Could not measure CPU usage"
fi

# Summary
echo ""
echo "======================================"
echo "Performance Summary"
echo "======================================"
echo "Startup time: ${AVG_STARTUP}ms"
echo "Memory usage: ${AVG_MEM}MB (peak: ${MAX_MEM}MB)"
echo "CPU usage: ${AVG_CPU}% (peak: ${MAX_CPU}%)"
echo ""
echo "Results saved to: $RESULTS_DIR"

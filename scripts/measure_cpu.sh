#!/bin/bash
# Simple CPU measurement script for splitrail-dashboard
# Works reliably on macOS

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RESULTS_FILE="${1:-cpu_results.txt}"

echo "======================================"
echo "Splitrail CPU Measurement"
echo "======================================"
echo ""

# Check if binary exists
BINARY="$PROJECT_DIR/target/release/splitrail-dashboard"
if [ ! -f "$BINARY" ]; then
    echo "Error: Release binary not found at $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

# Function to measure CPU for a running process
measure_cpu() {
    local pid=$1
    local duration=$2
    local interval=1
    local samples=0
    local total_cpu=0
    local max_cpu=0

    echo "  Measuring CPU for ${duration}s (sampling every ${interval}s)..."

    for ((i=0; i<duration; i++)); do
        if ! ps -p $pid > /dev/null 2>&1; then
            echo "  Warning: Process died at ${i}s"
            break
        fi

        # Get CPU percentage (macOS format)
        cpu=$(ps -p $pid -o %cpu= 2>/dev/null | awk '{print $1}' | sed 's/[^0-9.]//g')

        if [ -n "$cpu" ]; then
            total_cpu=$(echo "$total_cpu + $cpu" | bc)
            samples=$((samples + 1))

            # Track max
            if (( $(echo "$cpu > $max_cpu" | bc -l) )); then
                max_cpu=$cpu
            fi

            # Show dot for progress
            echo -n "."
        fi

        sleep $interval
    done

    echo ""

    if [ $samples -gt 0 ]; then
        avg_cpu=$(echo "scale=2; $total_cpu / $samples" | bc)
        echo "  Samples: $samples"
        echo "  Average CPU: ${avg_cpu}%"
        echo "  Peak CPU: ${max_cpu}%"
        echo "${avg_cpu},${max_cpu}" >> "$RESULTS_FILE"
    else
        echo "  No samples collected"
        echo "0,0" >> "$RESULTS_FILE"
    fi
}

echo "Test 1: Idle CPU Usage"
echo "------------------------------------"
echo ""
echo "INSTRUCTIONS:"
echo "1. In another terminal, run: cargo run --release"
echo "2. Wait for the app to fully start (TUI appears)"
echo "3. DO NOT INTERACT with the app"
echo "4. Come back here and press Enter to start measurement"
echo ""
read -p "Press Enter when app is running and idle..."

# Find the splitrail-dashboard process
PID=$(pgrep -f "splitrail-dashboard" | head -1)

if [ -z "$PID" ]; then
    echo "  Error: splitrail-dashboard process not found"
    echo "  Make sure the app is running in another terminal"
    exit 1
fi

echo "  Found PID: $PID"
echo "  Waiting 3s to ensure idle state..."
sleep 3

# Clear results file
> "$RESULTS_FILE"

# Measure idle CPU (user should not interact)
echo ""
echo "  DO NOT INTERACT WITH THE APP"
echo "  Measuring idle CPU usage..."
measure_cpu $PID 15

# Note: Don't kill the process - user is running it manually
echo ""
echo "Measurement complete (app still running in other terminal)"

# Read results
if [ -f "$RESULTS_FILE" ]; then
    echo ""
    echo "======================================"
    echo "Results"
    echo "======================================"

    IFS=',' read -r avg_cpu max_cpu < "$RESULTS_FILE"

    echo "Idle CPU:"
    echo "  Average: ${avg_cpu}%"
    echo "  Peak: ${max_cpu}%"
    echo ""
    echo "Results saved to: $RESULTS_FILE"
    echo ""

    # Interpretation
    if (( $(echo "$avg_cpu < 3" | bc -l) )); then
        echo "✅ Excellent: Very low idle CPU (<3%)"
    elif (( $(echo "$avg_cpu < 5" | bc -l) )); then
        echo "✅ Good: Low idle CPU (3-5%)"
    elif (( $(echo "$avg_cpu < 10" | bc -l) )); then
        echo "⚠️  Moderate: Noticeable idle CPU (5-10%)"
    else
        echo "❌ High: Significant idle CPU (>10%)"
    fi
else
    echo "Error: No results file generated"
    exit 1
fi

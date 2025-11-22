#!/bin/bash

# CPU Benchmark Script for Splitrail Dashboard
# Compares performance before and after optimization changes

set -e

echo "=== Splitrail Dashboard CPU Benchmark ==="
echo ""

# Function to measure CPU usage
measure_cpu() {
    local commit=$1
    local label=$2
    local binary=$3
    
    echo "📊 Testing: $label (commit: $commit)"
    
    # Start the dashboard in background
    $binary &
    local pid=$!
    
    # Give it time to initialize
    sleep 3
    
    # Sample CPU usage over 10 seconds (10 samples, 1 second apart)
    echo "   Sampling CPU for 10 seconds..."
    local total_cpu=0
    local samples=0
    
    for i in {1..10}; do
        # Get CPU percentage for the process on macOS
        cpu=$(ps -p $pid -o %cpu= | tr -d ' ')
        if [ ! -z "$cpu" ]; then
            total_cpu=$(echo "$total_cpu + $cpu" | bc)
            samples=$((samples + 1))
            echo "   Sample $i: ${cpu}%"
        fi
        sleep 1
    done
    
    # Kill the process
    kill $pid 2>/dev/null || true
    wait $pid 2>/dev/null || true
    
    # Calculate average
    if [ $samples -gt 0 ]; then
        avg_cpu=$(echo "scale=2; $total_cpu / $samples" | bc)
        echo "   ✅ Average CPU: ${avg_cpu}%"
    else
        echo "   ❌ Failed to get CPU measurements"
        avg_cpu="N/A"
    fi
    
    echo ""
    echo "$avg_cpu"
}

# Build current version (with optimizations)
echo "🔨 Building CURRENT version (with optimizations)..."
cargo build --release --quiet
cp target/release/splitrail-dashboard /tmp/splitrail-after
echo ""

# Checkout and build old version (before optimizations)
echo "🔨 Building OLD version (before optimizations)..."
git stash push -m "benchmark temp" --quiet 2>/dev/null || true
git checkout 1399ee9 --quiet
cargo build --release --quiet
cp target/release/splitrail-dashboard /tmp/splitrail-before
git checkout - --quiet
git stash pop --quiet 2>/dev/null || true
echo ""

# Run benchmarks
echo "🚀 Running benchmarks..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cpu_before=$(measure_cpu "1399ee9" "BEFORE optimization" "/tmp/splitrail-before")
cpu_after=$(measure_cpu "7fdebba" "AFTER optimization" "/tmp/splitrail-after")

# Calculate improvement
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📈 RESULTS SUMMARY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "BEFORE (commit 1399ee9):"
echo "  Average CPU: ${cpu_before}%"
echo ""
echo "AFTER (commit 7fdebba):"
echo "  Average CPU: ${cpu_after}%"
echo ""

if [ "$cpu_before" != "N/A" ] && [ "$cpu_after" != "N/A" ]; then
    reduction=$(echo "scale=2; $cpu_before - $cpu_after" | bc)
    percent_reduction=$(echo "scale=1; ($reduction / $cpu_before) * 100" | bc)
    echo "🎯 IMPROVEMENT:"
    echo "  Reduction: ${reduction}% CPU"
    echo "  Relative: ${percent_reduction}% improvement"
    echo ""
    
    if (( $(echo "$percent_reduction > 50" | bc -l) )); then
        echo "✨ Excellent! More than 50% CPU reduction achieved!"
    elif (( $(echo "$percent_reduction > 25" | bc -l) )); then
        echo "✅ Good! Significant CPU reduction achieved!"
    else
        echo "⚠️  Modest improvement. Consider Phase 3 optimizations."
    fi
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Cleanup
rm -f /tmp/splitrail-before /tmp/splitrail-after

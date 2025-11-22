#!/bin/bash

# Simple CPU monitor for Splitrail Dashboard
# Measures current version CPU usage

echo "=== Splitrail Dashboard CPU Monitor ==="
echo ""
echo "Starting dashboard and monitoring CPU for 30 seconds..."
echo ""

# Build and run current version
cargo build --release --quiet 2>/dev/null
./target/release/splitrail-dashboard &
pid=$!

# Give it time to initialize
sleep 3

echo "Dashboard PID: $pid"
echo ""
echo "Sampling CPU every second for 30 seconds:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

total_cpu=0
samples=0
max_cpu=0
min_cpu=999999

for i in {1..30}; do
    cpu=$(ps -p $pid -o %cpu= 2>/dev/null | tr -d ' ')
    if [ ! -z "$cpu" ]; then
        printf "Sample %2d: %6.2f%%\n" $i $cpu
        total_cpu=$(echo "$total_cpu + $cpu" | bc)
        samples=$((samples + 1))
        
        # Track min/max
        if (( $(echo "$cpu > $max_cpu" | bc -l) )); then
            max_cpu=$cpu
        fi
        if (( $(echo "$cpu < $min_cpu" | bc -l) )); then
            min_cpu=$cpu
        fi
    fi
    sleep 1
done

# Kill the process
kill $pid 2>/dev/null
wait $pid 2>/dev/null || true

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [ $samples -gt 0 ]; then
    avg_cpu=$(echo "scale=2; $total_cpu / $samples" | bc)
    echo "📊 CPU USAGE STATISTICS:"
    echo "  Average: ${avg_cpu}%"
    echo "  Minimum: ${min_cpu}%"
    echo "  Maximum: ${max_cpu}%"
    echo "  Samples: $samples"
    echo ""
    
    if (( $(echo "$avg_cpu < 10" | bc -l) )); then
        echo "✨ Excellent! Very low CPU usage (<10%)"
    elif (( $(echo "$avg_cpu < 25" | bc -l) )); then
        echo "✅ Good! Reasonable CPU usage (<25%)"
    elif (( $(echo "$avg_cpu < 50" | bc -l) )); then
        echo "⚠️  Moderate CPU usage (25-50%)"
    else
        echo "🔴 High CPU usage (>50%) - optimization needed"
    fi
else
    echo "❌ Failed to get CPU measurements"
fi

echo ""

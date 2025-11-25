#!/bin/bash
# Quick CPU measurement - just measure what's running now

set -e

echo "======================================"
echo "Quick CPU Measurement"
echo "======================================"
echo ""

# Find the running splitrail process
PID=$(pgrep -f "target/release/splitrail-dashboard" | head -1)

if [ -z "$PID" ]; then
    echo "❌ Error: splitrail-dashboard not running"
    echo ""
    echo "Please run in another terminal:"
    echo "  cargo run --release"
    echo ""
    exit 1
fi

echo "✅ Found splitrail-dashboard (PID: $PID)"
echo ""
echo "Measuring CPU for 15 seconds..."
echo "DO NOT INTERACT WITH THE APP"
echo ""

# Sample CPU every second
samples=0
total_cpu=0
max_cpu=0

for ((i=1; i<=15; i++)); do
    if ! ps -p $PID > /dev/null 2>&1; then
        echo ""
        echo "⚠️  Process died at ${i}s"
        break
    fi

    cpu=$(ps -p $PID -o %cpu= 2>/dev/null | awk '{print $1}' | sed 's/[^0-9.]//g')

    if [ -n "$cpu" ]; then
        total_cpu=$(echo "$total_cpu + $cpu" | bc)
        samples=$((samples + 1))

        if (( $(echo "$cpu > $max_cpu" | bc -l) )); then
            max_cpu=$cpu
        fi

        # Show progress
        printf "."
    fi

    sleep 1
done

echo ""
echo ""

if [ $samples -gt 0 ]; then
    avg_cpu=$(echo "scale=2; $total_cpu / $samples" | bc)

    echo "======================================"
    echo "Results"
    echo "======================================"
    echo "Average CPU: ${avg_cpu}%"
    echo "Peak CPU: ${max_cpu}%"
    echo ""

    # Interpretation
    if (( $(echo "$avg_cpu < 3" | bc -l) )); then
        echo "✅ EXCELLENT: Very low idle CPU (<3%)"
        echo "   Optimizations are working great!"
    elif (( $(echo "$avg_cpu < 5" | bc -l) )); then
        echo "✅ GOOD: Low idle CPU (3-5%)"
        echo "   Optimizations are helping"
    elif (( $(echo "$avg_cpu < 10" | bc -l) )); then
        echo "⚠️  MODERATE: Noticeable idle CPU (5-10%)"
        echo "   Some improvement but could be better"
    else
        echo "❌ HIGH: Significant idle CPU (>10%)"
        echo "   Optimizations may not be working as expected"
    fi
else
    echo "❌ No samples collected"
fi

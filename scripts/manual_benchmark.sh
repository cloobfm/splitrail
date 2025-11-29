#!/bin/bash

echo "🧪 MANUAL PERFORMANCE TEST"
echo "========================"

echo ""
echo "📊 Testing 1: Default Settings (5 seconds)"
echo "--------------------------------------------"

./target/release/splitrail-dashboard &
PID=$!
sleep 3

# Quick CPU measurement
cpu1=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu2=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu3=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)

kill $PID 2>/dev/null || true

avg1=$(( (cpu1 + cpu2 + cpu3) / 3 ))
echo "Default Settings CPU: ${avg1}%"

echo ""
echo "📊 Testing 2: Clock Disabled (5 seconds)"
echo "--------------------------------------------"

SPLITRAIL_DISABLE_CLOCK=1 ./target/release/splitrail-dashboard &
PID=$!
sleep 3

cpu1=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu2=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu3=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)

kill $PID 2>/dev/null || true

avg2=$(( (cpu1 + cpu2 + cpu3) / 3 ))
echo "Clock Disabled CPU: ${avg2}%"

echo ""
echo "📊 Testing 3: Both Optimizations (5 seconds)"
echo "--------------------------------------------"

SPLITRAIL_DISABLE_CLOCK=1 SPLITRAIL_POLL_INTERVAL_MS=1000 ./target/release/splitrail-dashboard &
PID=$!
sleep 3

cpu1=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu2=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
cpu3=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)

kill $PID 2>/dev/null || true

avg3=$(( (cpu1 + cpu2 + cpu3) / 3 ))
echo "Both Optimizations CPU: ${avg3}%"

echo ""
echo "📈 RESULTS SUMMARY"
echo "==================="
echo ""
printf "%-20s | %5s | %10s | %s\n" "Configuration" "CPU %" "Improvement" "Status"
echo "--------------------|-------|------------|--------"

calc_status() {
    local cpu="$1"
    local baseline="$2"
    local name="$3"
    
    if [ $cpu -le 5 ]; then
        status="🎉 Excellent"
    elif [ $cpu -le 10 ]; then
        status="✅ Good"
    elif [ $cpu -le 15 ]; then
        status="⚠️  Moderate"
    else
        status="❌ High"
    fi
    
    improvement=$((baseline - cpu))
    printf "%-20s | %5s | %10s | %s\n" "$name" "${cpu}%" "${improvement}%" "$status"
}

calc_status "$avg1" "$avg1" "Default Settings"
calc_status "$avg2" "$avg1" "Clock Disabled"
calc_status "$avg3" "$avg1" "Both Optimizations"

echo ""
echo "🎯 BEST CONFIGURATION:"
if [ $avg2 -lt $avg1 ] && [ $avg2 -le $avg3 ]; then
    echo "✅ Clock Disabled (${avg2}%)"
    echo "📝 Command: SPLITRAIL_DISABLE_CLOCK=1 ./target/release/splitrail-dashboard"
elif [ $avg3 -lt $avg1 ] && [ $avg3 -le $avg2 ]; then
    echo "✅ Both Optimizations (${avg3}%)"
    echo "📝 Command: SPLITRAIL_DISABLE_CLOCK=1 SPLITRAIL_POLL_INTERVAL_MS=1000 ./target/release/splitrail-dashboard"
else
    echo "⚠️  Default Settings (${avg1}%)"
    echo "📝 Command: ./target/release/splitrail-dashboard"
fi

echo ""
echo "💡 RECOMMENDATIONS:"
if [ $avg1 -le 10 ]; then
    echo "✅ Default performance is acceptable"
    echo "📝 No optimizations needed"
elif [ $avg2 -le 10 ] || [ $avg3 -le 10 ]; then
    echo "✅ Optimizations can achieve <10% CPU"
    echo "📝 Use recommended configuration above"
else
    echo "⚠️  All configurations show >10% CPU"
    echo "🔍 Further investigation needed with flamegraph"
fi
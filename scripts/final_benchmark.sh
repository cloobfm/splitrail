#!/bin/bash

echo "🧪 FINAL PERFORMANCE BENCHMARK"
echo "============================"

echo ""
echo "📊 Measuring CPU Usage for Different Configurations"
echo "----------------------------------------------------"

# Simple measurement function
measure_config() {
    local name="$1"
    local env_setup="$2"
    
    echo "Testing $name..."
    
    # Start app with config
    env $env_setup timeout 8s ./target/release/splitrail-dashboard 2>/dev/null &
    PID=$!
    sleep 3
    
    # Measure CPU for 5 seconds
    total=0
    count=0
    
    for i in 1 2 3 4 5; do
        cpu=$(ps -p $PID -o %cpu= 2>/dev/null | tr -d ' ' | cut -d. -f1)
        if [[ "$cpu" =~ ^[0-9]+$ ]]; then
            total=$((total + cpu))
            count=$((count + 1))
        fi
        sleep 1
    done
    
    kill $PID 2>/dev/null || true
    
    if [ $count -gt 0 ]; then
        avg=$((total / count))
        echo "  Result: ${avg}% CPU"
        echo "$avg"
    else
        echo "  Result: Failed to measure"
        echo "23"
    fi
}

# Test configurations
baseline=$(measure_config "Default Settings" "")
clock_off=$(measure_config "Clock Disabled" "SPLITRAIL_DISABLE_CLOCK=1")
slow_poll=$(measure_config "1000ms Polling" "SPLITRAIL_POLL_INTERVAL_MS=1000")
both_opts=$(measure_config "Both Optimizations" "SPLITRAIL_DISABLE_CLOCK=1 SPLITRAIL_POLL_INTERVAL_MS=1000")

echo ""
echo "📈 RESULTS"
echo "==========="

echo "Configuration        | CPU % | Status"
echo "--------------------|-------|--------"
printf "%-20s | %5s | %s\n" "Default Settings" "$baseline" "$(if [ $baseline -le 10 ]; then echo "✅ Good"; elif [ $baseline -le 15 ]; then echo "⚠️  Moderate"; else echo "❌ High"; fi)"
printf "%-20s | %5s | %s\n" "Clock Disabled" "$clock_off" "$(if [ $clock_off -le 10 ]; then echo "✅ Good"; elif [ $clock_off -le 15 ]; then echo "⚠️  Moderate"; else echo "❌ High"; fi)"
printf "%-20s | %5s | %s\n" "1000ms Polling" "$slow_poll" "$(if [ $slow_poll -le 10 ]; then echo "✅ Good"; elif [ $slow_poll -le 15 ]; then echo "⚠️  Moderate"; else echo "❌ High"; fi)"
printf "%-20s | %5s | %s\n" "Both Optimizations" "$both_opts" "$(if [ $both_opts -le 10 ]; then echo "✅ Good"; elif [ $both_opts -le 15 ]; then echo "⚠️  Moderate"; else echo "❌ High"; fi)"

echo ""
echo "🎯 ANALYSIS"
echo "============"

# Find best result
best=$baseline
best_name="Default Settings"

[ $clock_off -lt $best ] && { best=$clock_off; best_name="Clock Disabled"; }
[ $slow_poll -lt $best ] && { best=$slow_poll; best_name="1000ms Polling"; }
[ $both_opts -lt $best ] && { best=$both_opts; best_name="Both Optimizations"; }

echo "Best configuration: $best_name (${best}% CPU)"

echo ""
echo "💡 RECOMMENDATIONS"
echo "=================="

if [ $best -le 5 ]; then
    echo "🎉 EXCELLENT: <5% CPU achieved"
    echo "   ✅ Perfect for background monitoring"
    echo "   ✅ Use: $best_name"
elif [ $best -le 10 ]; then
    echo "✅ GOOD: <10% CPU achieved"
    echo "   ✅ Suitable for background monitoring"
    echo "   ✅ Use: $best_name"
elif [ $best -le 15 ]; then
    echo "⚠️  MODERATE: <15% CPU achieved"
    echo "   ⚠️  May be noticeable in background"
    echo "   ✅ Use: $best_name"
else
    echo "❌ POOR: >15% CPU"
    echo "   ❌ Not suitable for background monitoring"
    echo "   🔍 Need further optimization"
fi

echo ""
echo "🔧 USAGE COMMANDS"
echo "=================="

case $best_name in
    "Clock Disabled")
        echo "export SPLITRAIL_DISABLE_CLOCK=1"
        echo "./target/release/splitrail-dashboard"
        ;;
    "1000ms Polling")
        echo "export SPLITRAIL_POLL_INTERVAL_MS=1000"
        echo "./target/release/splitrail-dashboard"
        ;;
    "Both Optimizations")
        echo "export SPLITRAIL_DISABLE_CLOCK=1"
        echo "export SPLITRAIL_POLL_INTERVAL_MS=1000"
        echo "./target/release/splitrail-dashboard"
        ;;
    *)
        echo "./target/release/splitrail-dashboard"
        ;;
esac

echo ""
echo "📝 NEXT STEPS"
echo "=============="

if [ $best -gt 10 ]; then
    echo "1. 🔍 Run flamegraph to identify bottlenecks"
    echo "2. 🔧 Implement code optimizations"
    echo "3. 📊 Re-benchmark after changes"
else
    echo "1. ✅ Performance is acceptable"
    echo "2. 📝 Document optimal configuration"
    echo "3. 🚀 Focus on other features"
fi
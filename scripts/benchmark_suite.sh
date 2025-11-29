#!/bin/bash

# Performance Benchmarking Suite for Splitrail
# This script runs comprehensive benchmarks and documents baseline performance

set -e

echo "🚀 Splitrail Performance Benchmarking Suite"
echo "============================================"

# Create results directory
RESULTS_DIR="benchmark_results"
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
REPORT_DIR="$RESULTS_DIR/report_$TIMESTAMP"
mkdir -p "$REPORT_DIR"

echo "📊 Results will be saved to: $REPORT_DIR"

# Function to run a benchmark and save results
run_benchmark() {
    local bench_name=$1
    local description=$2
    
    echo ""
    echo "🔧 Running $bench_name..."
    echo "   $description"
    
    # Run the benchmark
    cargo bench --bench "$bench_name" -- --output-format html 2>&1 | tee "$REPORT_DIR/${bench_name}_output.log"
    
    # Copy HTML reports if they exist
    if [ -d "target/criterion" ]; then
        cp -r target/criterion "$REPORT_DIR/${bench_name}_criterion"
    fi
    
    echo "✅ $bench_name completed"
}

# Function to run tests
run_tests() {
    echo ""
    echo "🧪 Running test suite..."
    
    # Unit tests
    echo "   Running unit tests..."
    cargo test --lib 2>&1 | tee "$REPORT_DIR/unit_tests.log"
    
    # Integration tests
    echo "   Running integration tests..."
    cargo test --test integration_tests 2>&1 | tee "$REPORT_DIR/integration_tests.log"
    
    echo "✅ Tests completed"
}

# Function to gather system information
gather_system_info() {
    echo ""
    echo "📋 Gathering system information..."
    
    {
        echo "=== System Information ==="
        echo "Date: $(date)"
        echo "OS: $(uname -s)"
        echo "Kernel: $(uname -r)"
        echo "Architecture: $(uname -m)"
        echo ""
        
        echo "=== CPU Information ==="
        if command -v lscpu >/dev/null 2>&1; then
            lscpu
        elif command -v sysctl >/dev/null 2>&1; then
            sysctl -n machdep.cpu.brand_string
            sysctl -n hw.ncpu
        fi
        echo ""
        
        echo "=== Memory Information ==="
        if command -v free >/dev/null 2>&1; then
            free -h
        elif command -v vm_stat >/dev/null 2>&1; then
            vm_stat
        fi
        echo ""
        
        echo "=== Rust Information ==="
        rustc --version
        cargo --version
        echo ""
        
        echo "=== Git Information ==="
        git rev-parse HEAD
        git status --porcelain
        echo ""
        
    } > "$REPORT_DIR/system_info.txt"
    
    echo "✅ System information gathered"
}

# Function to profile CPU usage during typical operations
profile_cpu_usage() {
    echo ""
    echo "📈 Profiling CPU usage during typical operations..."
    
    # Create a simple profiling script
    cat > /tmp/splitrail_profile.sh << 'EOF'
#!/bin/bash
echo "Starting CPU profiling..."
echo "Timestamp,CPU_Usage,Memory_Usage" > /tmp/splitrail_usage.csv

# Monitor for 60 seconds
for i in {1..60}; do
    if command -v top >/dev/null 2>&1; then
        CPU=$(top -l 1 -n 0 | grep "CPU usage" | awk '{print $3}' | sed 's/%//')
        MEM=$(top -l 1 -n 0 | grep "PhysMem" | awk '{print $2}' | sed 's/M//')
    elif command -v ps >/dev/null 2>&1; then
        CPU=$(ps aux | grep 'splitrail' | awk '{sum+=$3} END {print sum}')
        MEM=$(ps aux | grep 'splitrail' | awk '{sum+=$4} END {print sum}')
    else
        CPU="0"
        MEM="0"
    fi
    
    echo "$(date +%s),$CPU,$MEM" >> /tmp/splitrail_usage.csv
    sleep 1
done
EOF
    
    chmod +x /tmp/splitrail_profile.sh
    
    # Start profiling in background
    /tmp/splitrail_profile.sh &
    PROFILE_PID=$!
    
    # Run splitrail for a minute to generate typical load
    echo "   Running splitrail under typical load..."
    timeout 60s cargo run 2>/dev/null || true
    
    # Stop profiling
    kill $PROFILE_PID 2>/dev/null || true
    wait $PROFILE_PID 2>/dev/null || true
    
    # Move results
    mv /tmp/splitrail_usage.csv "$REPORT_DIR/" 2>/dev/null || true
    
    echo "✅ CPU profiling completed"
}

# Function to generate summary report
generate_summary_report() {
    echo ""
    echo "📝 Generating summary report..."
    
    cat > "$REPORT_DIR/summary.md" << EOF
# Splitrail Performance Benchmark Report

**Generated:** $(date)  
**Commit:** $(git rev-parse --short HEAD 2>/dev/null || echo "unknown")

## Executive Summary

This report contains baseline performance measurements for the Splitrail application before optimization work.

## Test Environment

$(cat "$REPORT_DIR/system_info.txt")

## Benchmark Results

### 1. Core Performance Benchmarks
- **File:** \`performance_bench_output.log\`
- **HTML Report:** \`performance_bench_criterion/\`

Key metrics measured:
- Initial data loading time per analyzer
- Stats aggregation performance
- Hash computation performance
- File parsing (sequential vs parallel)
- Memory allocation patterns
- TUI rendering performance

### 2. Performance Regression Tests
- **File:** \`performance_regression_output.log\`
- **HTML Report:** \`performance_regression_criterion/\`

Regression tests ensure:
- Hash performance doesn't degrade
- Aggregation performance scales linearly
- Deduplication stays within acceptable bounds
- Memory allocation patterns remain efficient

### 3. Test Results
- **Unit Tests:** \`unit_tests.log\`
- **Integration Tests:** \`integration_tests.log\`

All tests should pass to ensure correctness is maintained.

### 4. CPU Usage Profile
- **File:** \`splitrail_usage.csv\`

CPU usage during typical 60-second operation.

## Key Performance Indicators

### Before Optimization (Baseline)
- **Expected CPU Spikes:** Up to 67% during data loading
- **Memory Usage:** High due to O(n²) deduplication
- **Hash Performance:** SHA256-based (10x slower than alternatives)
- **Parallel Processing:** Unbounded (causes system overload)

### Target After Optimization
- **CPU Spikes:** Under 25% (70% reduction)
- **Memory Usage:** 40-60% reduction
- **Hash Performance:** 10x faster with aHash/xxHash
- **Parallel Processing:** Bounded and efficient

## Next Steps

1. Review benchmark results to identify bottlenecks
2. Implement optimizations in priority order:
   - Fix O(n²) deduplication (highest impact)
   - Replace SHA256 with faster hash
   - Add bounded parallelism
3. Re-run benchmarks to measure improvements
4. Ensure all tests still pass

## Files Generated

- \`system_info.txt\` - System configuration
- \`*_output.log\` - Text output from benchmarks
- \`*_criterion/\` - HTML reports from Criterion
- \`unit_tests.log\` - Unit test results
- \`integration_tests.log\` - Integration test results
- \`splitrail_usage.csv\` - CPU usage over time
- \`summary.md\` - This summary file

EOF
    
    echo "✅ Summary report generated: $REPORT_DIR/summary.md"
}

# Main execution
main() {
    echo "Starting comprehensive benchmarking..."
    
    # Gather system info first
    gather_system_info
    
    # Run all tests to ensure correctness
    run_tests
    
    # Run performance benchmarks
    run_benchmark "performance_bench" "Core performance measurements"
    run_benchmark "performance_regression" "Regression test suite"
    
    # Profile real-world usage
    profile_cpu_usage
    
    # Generate summary
    generate_summary_report
    
    echo ""
    echo "🎉 Benchmarking completed successfully!"
    echo ""
    echo "📊 Results location: $REPORT_DIR"
    echo "📋 View summary: $REPORT_DIR/summary.md"
    echo ""
    echo "📈 Next steps:"
    echo "   1. Review the benchmark results"
    echo "   2. Implement optimizations based on findings"
    echo "   3. Re-run this suite to measure improvements"
    echo ""
    
    # Open summary if on macOS
    if command -v open >/dev/null 2>&1; then
        echo "🌐 Opening summary report..."
        open "$REPORT_DIR/summary.md"
    fi
}

# Check dependencies
check_dependencies() {
    echo "🔍 Checking dependencies..."
    
    if ! command -v cargo >/dev/null 2>&1; then
        echo "❌ Cargo not found. Please install Rust."
        exit 1
    fi
    
    if ! command -v git >/dev/null 2>&1; then
        echo "❌ Git not found. Please install Git."
        exit 1
    fi
    
    echo "✅ Dependencies OK"
}

# Run the script
check_dependencies
main
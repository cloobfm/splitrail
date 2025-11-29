#!/bin/bash

# Quick Baseline Performance Test
# This script runs a minimal set of benchmarks to establish baseline performance

set -e

echo "🔍 Splitrail Baseline Performance Test"
echo "======================================="

# Create results directory
RESULTS_DIR="baseline_results"
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

echo "📊 Results will be saved to: $RESULTS_DIR/baseline_$TIMESTAMP"

# Function to run a simple benchmark
run_simple_bench() {
    local name=$1
    local command=$2
    
    echo ""
    echo "🔧 Testing $name..."
    
    # Run with time measurement
    start_time=$(date +%s.%N)
    eval "$command" > "$RESULTS_DIR/${name}_${TIMESTAMP}.log" 2>&1
    end_time=$(date +%s.%N)
    
    duration=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
    echo "✅ $name completed in ${duration}s"
}

# Function to gather basic system info
gather_basic_info() {
    echo "📋 Gathering system information..."
    
    {
        echo "=== Basic System Info ==="
        echo "Date: $(date)"
        echo "OS: $(uname -s)"
        echo "CPU Cores: $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 'unknown')"
        echo "Memory: $(free -h 2>/dev/null | grep '^Mem:' || echo 'N/A')"
        echo ""
        echo "=== Rust Info ==="
        echo "Rustc: $(rustc --version)"
        echo "Cargo: $(cargo --version)"
        echo ""
        echo "=== Git Info ==="
        echo "Commit: $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
        echo "Branch: $(git branch --show-current 2>/dev/null || echo 'unknown')"
        echo ""
    } > "$RESULTS_DIR/system_info_${TIMESTAMP}.txt"
    
    echo "✅ System info gathered"
}

# Function to test hash performance
test_hash_performance() {
    echo "🔍 Testing hash performance..."
    
    # Create a simple test program
    cat > /tmp/hash_test.rs << 'EOF'
use std::time::Instant;
use sha2::{Digest, Sha256};

fn main() {
    let text = "x".repeat(1000);
    let iterations = 10000;
    
    let start = Instant::now();
    for _ in 0..iterations {
        let mut hasher = Sha256::new();
        hasher.update(&text);
        let _result = hasher.finalize();
    }
    let duration = start.elapsed();
    
    let avg_time_ns = duration.as_nanos() / iterations;
    let avg_time_us = avg_time_ns as f64 / 1000.0;
    
    println!("SHA256 hash performance:");
    println!("  Total time: {:?}", duration);
    println!("  Average per hash: {:.2} μs", avg_time_us);
    println!("  Hashes per second: {:.0}", 1_000_000.0 / avg_time_us);
}
EOF
    
    echo "🔧 Compiling hash test..."
    rustc /tmp/hash_test.rs -o /tmp/hash_test --extern sha2=~/.cargo/registry/src/*/sha2-*/src/lib.rs 2>/dev/null || {
        echo "⚠️  Could not compile standalone hash test, using built-in test instead"
        return
    }
    
    echo "🔧 Running hash test..."
    /tmp/hash_test > "$RESULTS_DIR/hash_performance_${TIMESTAMP}.txt" 2>&1
    echo "✅ Hash performance test completed"
    
    # Cleanup
    rm -f /tmp/hash_test /tmp/hash_test.rs
}

# Function to test basic build performance
test_build_performance() {
    echo "🔧 Testing build performance..."
    
    # Clean build
    echo "  Performing clean build..."
    start_time=$(date +%s.%N)
    cargo clean --quiet
    cargo build --quiet --release
    end_time=$(date +%s.%N)
    
    build_time=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
    echo "  Clean build time: ${build_time}s"
    
    # Incremental build
    echo "  Testing incremental build..."
    start_time=$(date +%s.%N)
    cargo build --quiet --release
    end_time=$(date +%s.%N)
    
    incremental_time=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
    echo "  Incremental build time: ${incremental_time}s"
    
    {
        echo "=== Build Performance ==="
        echo "Clean build time: ${build_time}s"
        echo "Incremental build time: ${incremental_time}s"
        echo "Binary size: $(ls -lh target/release/splitrail-dashboard | awk '{print $5}')"
    } > "$RESULTS_DIR/build_performance_${TIMESTAMP}.txt"
    
    echo "✅ Build performance test completed"
}

# Function to run basic unit tests
test_unit_performance() {
    echo "🧪 Running unit tests with timing..."
    
    start_time=$(date +%s.%N)
    cargo test --lib --quiet 2>&1 | tee "$RESULTS_DIR/unit_tests_${TIMESTAMP}.log"
    end_time=$(date +%s.%N)
    
    test_time=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
    echo "✅ Unit tests completed in ${test_time}s"
}

# Function to create baseline summary
create_summary() {
    echo "📝 Creating baseline summary..."
    
    cat > "$RESULTS_DIR/baseline_summary_${TIMESTAMP}.md" << EOF
# Splitrail Baseline Performance Summary

**Generated:** $(date)  
**Commit:** $(git rev-parse --short HEAD 2>/dev/null || echo "unknown")

## System Information

$(cat "$RESULTS_DIR/system_info_${TIMESTAMP}.txt")

## Performance Measurements

### Build Performance
$(cat "$RESULTS_DIR/build_performance_${TIMESTAMP}.txt")

### Hash Performance
$(cat "$RESULTS_DIR/hash_performance_${TIMESTAMP}.txt" 2>/dev/null || echo "Hash performance test not available")

### Test Results
- Unit test time: ${test_time:-"N/A"}
- Test log: \`unit_tests_${TIMESTAMP}.log\`

## Current Performance Characteristics

### Expected Issues (Before Optimization)
- **CPU Spikes:** Up to 67% during data loading
- **Hash Algorithm:** SHA256 (cryptographic overkill)
- **Deduplication:** O(n²) algorithm
- **Memory Usage:** High due to inefficient allocations

### Key Areas for Optimization
1. **Hash Function:** Replace SHA256 with aHash (10x faster expected)
2. **Deduplication:** Fix O(n²) to O(n) (50-90% improvement expected)
3. **Memory:** Pre-allocate collections (20-40% improvement expected)
4. **Parallel Processing:** Add bounded parallelism (15-25% improvement expected)

## Next Steps

1. Review baseline measurements
2. Implement optimizations in priority order
3. Re-run this test to measure improvements
4. Compare before/after results

## Files Generated

- \`system_info_${TIMESTAMP}.txt\` - System configuration
- \`build_performance_${TIMESTAMP}.txt\` - Build timing
- \`hash_performance_${TIMESTAMP}.txt\` - Hash performance
- \`unit_tests_${TIMESTAMP}.log\` - Unit test results
- \`baseline_summary_${TIMESTAMP}.md\` - This summary file

EOF
    
    echo "✅ Summary created: $RESULTS_DIR/baseline_summary_${TIMESTAMP}.md"
}

# Main execution
main() {
    echo "Starting baseline performance measurement..."
    
    # Gather basic info
    gather_basic_info
    
    # Test build performance
    test_build_performance
    
    # Test hash performance
    test_hash_performance
    
    # Run unit tests
    test_unit_performance
    
    # Create summary
    create_summary
    
    echo ""
    echo "🎉 Baseline performance test completed!"
    echo ""
    echo "📊 Results location: $RESULTS_DIR"
    echo "📋 View summary: $RESULTS_DIR/baseline_summary_${TIMESTAMP}.md"
    echo ""
    echo "📈 Ready to start optimization work!"
    
    # Open summary if on macOS
    if command -v open >/dev/null 2>&1; then
        echo "🌐 Opening summary report..."
        open "$RESULTS_DIR/baseline_summary_${TIMESTAMP}.md"
    fi
}

# Check for basic dependencies
check_dependencies() {
    if ! command -v cargo >/dev/null 2>&1; then
        echo "❌ Cargo not found. Please install Rust."
        exit 1
    fi
    
    echo "✅ Dependencies OK"
}

# Run the script
check_dependencies
main
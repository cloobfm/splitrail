# Splitrail Testing and Benchmarking Guide

This guide covers the comprehensive testing and benchmarking setup for Splitrail performance optimization.

## 🧪 Test Suite Overview

### 1. Unit Tests (`src/utils/tests.rs`)
- **Hash function correctness**: Verify hash consistency and uniqueness
- **Stats aggregation**: Test date-based aggregation with various scenarios
- **Number formatting**: Verify locale-aware formatting works correctly
- **Warning system**: Test deduplication of warnings

### 2. Integration Tests (`tests/integration_tests.rs`)
- **Analyzer registry**: Test analyzer discovery and registration
- **Data loading**: Test end-to-end stats loading pipeline
- **File parsing**: Test JSONL parsing with real data
- **Parallel vs sequential**: Compare parsing performance
- **Memory usage**: Test large dataset handling

### 3. Performance Benchmarks (`benches/performance_bench.rs`)
- **Initial load**: Time to load data from each analyzer
- **Stats aggregation**: Performance of date-based aggregation
- **Hash computation**: Performance across different text sizes
- **File parsing**: Sequential vs parallel parsing
- **Memory patterns**: Vec allocation and string operations
- **TUI rendering**: Sparkline cache and token calculations

### 4. Regression Tests (`benches/performance_regression.rs`)
- **Hash performance regression**: Ensure hash optimizations don't break
- **Aggregation scaling**: Verify linear performance scaling
- **Deduplication bounds**: Keep O(n²) behavior in check
- **Memory allocation**: Prevent allocation regressions
- **JSON parsing**: Ensure JSON performance doesn't degrade

## 🚀 Running the Test Suite

### Quick Test Run
```bash
# Run all tests
cargo test

# Run unit tests only
cargo test --lib

# Run integration tests
cargo test --test integration_tests

# Run with output
cargo test -- --nocapture
```

### Full Benchmark Suite
```bash
# Run the complete benchmarking suite
./scripts/benchmark_suite.sh

# Or run individual benchmarks
cargo bench --bench performance_bench
cargo bench --bench performance_regression
```

### Individual Benchmark Categories
```bash
# Test specific performance areas
cargo bench --bench performance_bench -- deduplication
cargo bench --bench performance_bench -- hash_computation
cargo bench --bench performance_bench -- file_parsing
```

## 📊 Understanding Benchmark Results

### Criterion Output Format
- **Time**: Average execution time per iteration
- **Throughput**: Elements/bytes processed per second
- **Variation**: Statistical confidence intervals
- **Comparison**: Before/after optimization comparisons

### Key Metrics to Watch
1. **Deduplication Time**: Should decrease from O(n²) to O(n)
2. **Hash Performance**: 10x improvement with aHash vs SHA256
3. **Memory Usage**: 40-60% reduction with pre-allocation
4. **Parallel Efficiency**: Better CPU utilization with bounded parallelism

### HTML Reports
- Location: `target/criterion/report/index.html`
- Interactive charts and detailed statistics
- Comparison views for multiple runs

## 🔧 Performance Testing Before Optimization

### Baseline Measurements
Run this before any optimizations to establish baseline:

```bash
# Clean build to ensure accurate measurements
cargo clean

# Run baseline benchmark
./scripts/benchmark_suite.sh

# Note the results directory
# Example: benchmark_results/report_20241129_143022/
```

### Expected Baseline Characteristics
- **CPU Spikes**: Up to 67% during data loading
- **Deduplication**: O(n²) behavior visible with large message counts
- **Hash Performance**: ~100-500ns for SHA256 with small strings
- **Memory Growth**: Linear with message count, but high overhead

## 🧪 Test Coverage Requirements

### Critical Path Coverage
1. **Message Processing Pipeline**
   - File discovery → Parsing → Deduplication → Aggregation
   - All error conditions and edge cases
   
2. **Hash Function Behavior**
   - Consistency across multiple calls
   - Collision resistance (basic testing)
   - Performance with various input sizes
   
3. **Stats Aggregation**
   - Empty datasets
   - Single messages
   - Large datasets (10k+ messages)
   - Multiple dates and models

### Performance Regression Guards
1. **Hash Function**: Must maintain < 1μs for 1KB strings
2. **Aggregation**: Must scale linearly (O(n)) with message count
3. **Deduplication**: Must not exceed O(n log n) complexity
4. **Memory**: Must not grow faster than O(n) with dataset size

## 📈 Continuous Performance Monitoring

### Automated Benchmarks
Add to CI/CD pipeline:
```yaml
- name: Run Performance Benchmarks
  run: |
    cargo bench --bench performance_regression -- --output-format json > benchmark_results.json
    # Compare with baseline and fail if regression detected
```

### Performance Budgets
Set maximum acceptable times:
```toml
# Example performance budgets in Cargo.toml
[package.metadata.performance]
hash_function_max_ns = 1000
aggregation_max_per_1000 = 10000
deduplication_max_per_1000 = 5000
```

## 🐛 Debugging Performance Issues

### Profiling Tools
```bash
# CPU profiling
cargo build --release
perf record --call-graph=dwarf ./target/release/splitrail-dashboard
perf report

# Memory profiling
valgrind --tool=massif ./target/release/splitrail-dashboard
ms_print massif.out.*

# Flame graphs
cargo install flamegraph
cargo flamegraph --bin splitrail-dashboard
```

### Common Performance Bottlenecks
1. **Excessive allocations**: Look for repeated Vec/String allocations
2. **Hash collisions**: Check if deduplication is working correctly
3. **I/O contention**: Too many parallel file reads
4. **JSON parsing**: Large documents or inefficient parsing

## ✅ Success Criteria

### Before Optimization (Baseline)
- Document current performance characteristics
- Identify specific bottlenecks
- Establish regression test baseline

### After Optimization
- **70-85% CPU reduction** during data loading
- **40-60% memory usage** reduction
- **10x faster hash** operations
- **Linear scaling** for all aggregation operations
- **All tests pass** without modification

### Validation Steps
1. Run complete test suite: `cargo test`
2. Run benchmark suite: `./scripts/benchmark_suite.sh`
3. Compare results with baseline
4. Verify no regressions in functionality
5. Check performance targets are met

## 📋 Test Checklist

Before Optimizing:
- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] Baseline benchmarks documented
- [ ] Performance bottlenecks identified
- [ ] Regression tests in place

After Optimizing:
- [ ] All tests still pass
- [ ] Benchmarks show improvement
- [ ] No performance regressions
- [ ] Memory usage reduced
- [ ] CPU spikes eliminated
- [ ] Documentation updated

This comprehensive testing approach ensures we can objectively measure improvements while maintaining correctness and reliability.
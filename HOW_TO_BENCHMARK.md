# How to Benchmark Splitrail Performance

## Quick CPU Measurement (Current Version)

**Terminal 1** - Run the app:
```bash
cargo build --release
cargo run --release
# Wait for TUI to appear, then DON'T TOUCH IT
```

**Terminal 2** - Measure CPU:
```bash
./scripts/measure_cpu.sh current_cpu.txt
# Press Enter when prompted
# Wait 15 seconds
# Results will be saved and displayed
```

## Compare Before/After Optimizations

Run the automated comparison:

```bash
./scripts/compare_performance.sh
```

This will:
1. Measure AFTER optimizations (current state)
2. Checkout baseline commit (before optimizations)
3. Measure BEFORE optimizations
4. Return to current branch
5. Show comparison with % improvement

**Expected flow**:
```
Step 1/2: Measuring AFTER optimizations
  [Terminal 1: Run current version]
  [Terminal 2: Press Enter, wait 15s]

Step 2/2: Measuring BEFORE optimizations
  [Script checks out baseline]
  [Terminal 1: Quit app, run cargo run --release again]
  [Terminal 2: Press Enter, wait 15s]

Results: X% CPU reduction!
```

## Manual Comparison

### Step 1: Measure AFTER (current)
```bash
# Terminal 1
cargo build --release
cargo run --release

# Terminal 2
./scripts/measure_cpu.sh after_results.txt
```

### Step 2: Measure BEFORE (baseline)
```bash
# Checkout before Phase 1 optimizations
git checkout 1565381

# Terminal 1
cargo build --release
cargo run --release

# Terminal 2
./scripts/measure_cpu.sh before_results.txt

# Return to dashboard branch
git checkout dashboard
```

### Step 3: Compare
```bash
# After
AFTER=$(cut -d',' -f1 after_results.txt)
echo "After: ${AFTER}%"

# Before
BEFORE=$(cut -d',' -f1 before_results.txt)
echo "Before: ${BEFORE}%"

# Improvement
echo "scale=2; (($BEFORE - $AFTER) / $BEFORE) * 100" | bc
```

## Understanding Results

**Excellent** (>50% reduction):
- Before: 8-10% idle CPU
- After: 2-3% idle CPU
- Claimed: ~75% reduction ✅

**Good** (25-50% reduction):
- Before: 6-8% idle CPU
- After: 3-5% idle CPU
- Claimed: Optimizations helped 👍

**Moderate** (10-25% reduction):
- Before: 5-7% idle CPU
- After: 4-6% idle CPU
- Claimed: Minor improvement ⚠️

**Minimal** (<10% reduction):
- Before: 4-5% idle CPU
- After: 4-4.5% idle CPU
- Claimed: Claims were exaggerated ❌

## What We're Measuring

**Idle CPU**: CPU usage when app is running but user is not interacting
- **Before optimizations**: Fixed 250ms polling (4 Hz)
- **After optimizations**: Adaptive 1000ms polling when idle (1 Hz)
- **Expected**: ~75% reduction (4x less frequent polling)

**Why this matters**:
- App suitable for always-on background monitoring
- Lower power consumption on laptops
- Less thermal impact
- Doesn't slow down other applications

## Troubleshooting

**"Process not found"**:
- Make sure app is running in Terminal 1
- Run `pgrep -f splitrail-dashboard` to verify

**High variance**:
- Close other applications
- Don't move mouse during measurement
- Run multiple times, average results

**App won't start**:
- Check: `cargo run --release` works manually
- TUI requires actual terminal (can't redirect output)
- Needs data in `~/.claude/`, `~/.kilocode/`, etc.

## Clean Up

Remove test files:
```bash
rm -f *_results.txt cpu_results.txt test_results.txt
```

#!/bin/bash
# Compare CPU performance before and after optimizations

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "======================================"
echo "Splitrail Performance Comparison"
echo "======================================"
echo ""
echo "This will compare CPU usage before and after optimizations"
echo ""

# Check git status
cd "$PROJECT_DIR"
CURRENT_BRANCH=$(git branch --show-current)
echo "Current branch: $CURRENT_BRANCH"
echo ""

# Step 1: Measure AFTER optimizations (current state)
echo "Step 1/2: Measuring AFTER optimizations (current)"
echo "================================================"
echo ""

if [ ! -f "target/release/splitrail-dashboard" ]; then
    echo "Building current version..."
    cargo build --release --quiet
fi

chmod +x "$SCRIPT_DIR/measure_cpu.sh"
"$SCRIPT_DIR/measure_cpu.sh" "after_results.txt"
AFTER_AVG=$(cut -d',' -f1 after_results.txt)
AFTER_PEAK=$(cut -d',' -f2 after_results.txt)

echo ""
echo "Press Enter to continue to baseline measurement..."
read

# Step 2: Checkout before optimizations and measure
echo ""
echo "Step 2/2: Measuring BEFORE optimizations (baseline)"
echo "==================================================="
echo ""

# Find the commit before Phase 1 optimizations (a26bb26)
BASELINE_COMMIT="1565381"  # Commit right before a26bb26

echo "Checking out baseline commit: $BASELINE_COMMIT"
git checkout $BASELINE_COMMIT --quiet

echo "Building baseline version..."
cargo build --release --quiet

"$SCRIPT_DIR/measure_cpu.sh" "before_results.txt"
BEFORE_AVG=$(cut -d',' -f1 before_results.txt)
BEFORE_PEAK=$(cut -d',' -f2 before_results.txt)

# Return to original branch
echo ""
echo "Returning to $CURRENT_BRANCH..."
git checkout $CURRENT_BRANCH --quiet

# Compare results
echo ""
echo "======================================"
echo "Performance Comparison"
echo "======================================"
echo ""
echo "BEFORE optimizations:"
echo "  Average idle CPU: ${BEFORE_AVG}%"
echo "  Peak CPU: ${BEFORE_PEAK}%"
echo ""
echo "AFTER optimizations:"
echo "  Average idle CPU: ${AFTER_AVG}%"
echo "  Peak CPU: ${AFTER_PEAK}%"
echo ""

# Calculate improvement
if [ -n "$BEFORE_AVG" ] && [ -n "$AFTER_AVG" ]; then
    if (( $(echo "$BEFORE_AVG > 0" | bc -l) )); then
        REDUCTION=$(echo "scale=2; (($BEFORE_AVG - $AFTER_AVG) / $BEFORE_AVG) * 100" | bc)
        ABS_REDUCTION=$(echo "scale=2; $BEFORE_AVG - $AFTER_AVG" | bc)

        echo "IMPROVEMENT:"
        echo "  Absolute reduction: ${ABS_REDUCTION}% CPU"
        echo "  Relative reduction: ${REDUCTION}%"
        echo ""

        if (( $(echo "$REDUCTION > 50" | bc -l) )); then
            echo "✅ EXCELLENT: >50% CPU reduction achieved!"
        elif (( $(echo "$REDUCTION > 25" | bc -l) )); then
            echo "✅ GOOD: >25% CPU reduction achieved"
        elif (( $(echo "$REDUCTION > 10" | bc -l) )); then
            echo "✅ MODERATE: >10% CPU reduction achieved"
        elif (( $(echo "$REDUCTION > 0" | bc -l) )); then
            echo "⚠️  MINOR: Small improvement (<10%)"
        else
            echo "❌ NO IMPROVEMENT: CPU usage increased or unchanged"
        fi
    fi
fi

echo ""
echo "Results saved:"
echo "  Before: before_results.txt"
echo "  After: after_results.txt"

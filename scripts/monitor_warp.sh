#!/bin/bash
# Continuous Warp Log Monitor for Splitrail
# Tracks new log entries and appends to daily capture files

WARP_LOG="/Users/bzl/Library/Application Support/dev.warp.Warp-Stable/warp_network.log"
CAPTURE_DIR="$HOME/Projects/splitrail/schemas/warp/samples"
OFFSET_FILE="$CAPTURE_DIR/.warp_offset"

# Create capture directory
mkdir -p "$CAPTURE_DIR"

# Initialize offset tracking
if [ ! -f "$OFFSET_FILE" ]; then
    # Get current file size to start monitoring from now
    wc -c < "$WARP_LOG" > "$OFFSET_FILE"
    echo "Initialized monitoring from current position"
fi

# Get current date for filename
DATE=$(date "+%Y%m%d")
DAILY_CAPTURE="$CAPTURE_DIR/warp_daily_${DATE}.log"

# Read last offset
LAST_OFFSET=$(cat "$OFFSET_FILE")

# Get current file size
CURRENT_SIZE=$(wc -c < "$WARP_LOG")

if [ "$CURRENT_SIZE" -gt "$LAST_OFFSET" ]; then
    # Calculate bytes to read
    BYTES_TO_READ=$((CURRENT_SIZE - LAST_OFFSET))

    # Extract new content and append to daily capture
    tail -c "+$((LAST_OFFSET + 1))" "$WARP_LOG" | head -c "$BYTES_TO_READ" >> "$DAILY_CAPTURE"

    # Update offset
    echo "$CURRENT_SIZE" > "$OFFSET_FILE"

    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Captured $BYTES_TO_READ bytes to $DAILY_CAPTURE"
else
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] No new data (log size: $CURRENT_SIZE, offset: $LAST_OFFSET)"
fi

# Show summary of captured data
echo ""
echo "=== Today's Capture Summary ==="
if [ -f "$DAILY_CAPTURE" ]; then
    echo "File: $DAILY_CAPTURE"
    echo "Size: $(du -h "$DAILY_CAPTURE" | cut -f1)"

    # Count telemetry events
    EVENT_COUNT=$(grep -c '"event":' "$DAILY_CAPTURE" 2>/dev/null || echo "0")
    echo "Telemetry events: $EVENT_COUNT"

    # Count GraphQL queries
    QUERY_COUNT=$(grep -c '"query":' "$DAILY_CAPTURE" 2>/dev/null || echo "0")
    echo "GraphQL queries: $QUERY_COUNT"

    # List unique events
    echo ""
    echo "Unique event types:"
    grep -o '"event": "[^"]*"' "$DAILY_CAPTURE" 2>/dev/null | sort -u | sed 's/"event": /  - /'
else
    echo "No data captured yet today"
fi

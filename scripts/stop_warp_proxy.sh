#!/bin/bash
# Stop the background mitmproxy process

PID_FILE="$HOME/.mitmproxy/mitmdump.pid"

if [ ! -f "$PID_FILE" ]; then
    echo "No mitmproxy PID file found. Is it running?"
    exit 1
fi

PID=$(cat "$PID_FILE")

if ps -p "$PID" > /dev/null 2>&1; then
    echo "Stopping mitmproxy (PID: $PID)..."
    kill "$PID"
    rm "$PID_FILE"
    echo "✓ mitmproxy stopped"
else
    echo "mitmproxy process (PID: $PID) is not running"
    rm "$PID_FILE"
fi

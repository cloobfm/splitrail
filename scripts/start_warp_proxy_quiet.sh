#!/bin/bash
# Start mitmproxy in background mode (mitmdump - no UI)

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
INTERCEPTOR_SCRIPT="$SCRIPT_DIR/warp_interceptor.py"
MITM_DIR="$HOME/.mitmproxy"
PID_FILE="$HOME/.mitmproxy/mitmdump.pid"

# Create mitmproxy directory if it doesn't exist
mkdir -p "$MITM_DIR"

# Check if already running
if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if ps -p "$PID" > /dev/null 2>&1; then
        echo "mitmproxy is already running (PID: $PID)"
        echo "Run 'scripts/stop_warp_proxy.sh' to stop it first"
        exit 1
    fi
fi

echo "Starting mitmproxy in background mode..."
echo "Captured data: ~/Projects/splitrail/schemas/warp/captured/"
echo ""

# Start mitmdump in background
mitmdump \
    -s "$INTERCEPTOR_SCRIPT" \
    -p 8080 \
    --set confdir="$MITM_DIR" \
    > "$MITM_DIR/mitmdump.log" 2>&1 &

# Save PID
echo $! > "$PID_FILE"

echo "✓ mitmproxy started (PID: $(cat $PID_FILE))"
echo ""
echo "To view logs: tail -f ~/.mitmproxy/mitmdump.log"
echo "To stop: scripts/stop_warp_proxy.sh"
echo ""
echo "IMPORTANT: Configure Warp to use proxy http://localhost:8080"

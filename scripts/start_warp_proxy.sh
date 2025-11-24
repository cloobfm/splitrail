#!/bin/bash
# Start mitmproxy to intercept Warp traffic

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
INTERCEPTOR_SCRIPT="$SCRIPT_DIR/warp_interceptor.py"
MITM_DIR="$HOME/.mitmproxy"

# Create mitmproxy directory if it doesn't exist
mkdir -p "$MITM_DIR"

echo "Starting mitmproxy for Warp traffic interception..."
echo "=================================================="
echo ""
echo "Interceptor script: $INTERCEPTOR_SCRIPT"
echo "Captured data will be saved to: ~/Projects/splitrail/schemas/warp/captured/"
echo ""
echo "IMPORTANT: You must configure Warp to use this proxy!"
echo "See the instructions below after starting the proxy."
echo ""
echo "Press Ctrl+C to stop the proxy"
echo ""

# Start mitmproxy with the interceptor script
# -s = script to run
# -p = port (8080)
# --set confdir = config directory
mitmproxy \
    -s "$INTERCEPTOR_SCRIPT" \
    -p 8080 \
    --set confdir="$MITM_DIR" \
    --set console_eventlog_verbosity=info

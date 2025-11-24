#!/bin/bash
# One-command setup for Warp mitmproxy interception

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

echo "============================================"
echo "Warp mitmproxy Setup"
echo "============================================"
echo ""

# Step 1: Generate certificate
echo "Step 1/3: Generating mitmproxy certificate..."
echo "-------------------------------------------"
"$SCRIPT_DIR/start_warp_proxy_quiet.sh"
sleep 3
"$SCRIPT_DIR/stop_warp_proxy.sh"
echo ""

# Step 2: Install certificate
echo "Step 2/3: Installing certificate to macOS Keychain..."
echo "----------------------------------------------------"
echo "(This will ask for your sudo password)"
"$SCRIPT_DIR/install_mitmproxy_cert.sh"
echo ""

# Step 3: Instructions
echo "Step 3/3: Configure Warp to use proxy"
echo "--------------------------------------"
echo ""
echo "⚠️  IMPORTANT: You must configure macOS to use the proxy:"
echo ""
echo "1. Open System Settings → Network"
echo "2. Select your active network (Wi-Fi or Ethernet)"
echo "3. Click 'Details...'"
echo "4. Go to 'Proxies' tab"
echo "5. Check 'Web Proxy (HTTP)' and 'Secure Web Proxy (HTTPS)'"
echo "6. For both, set:"
echo "   - Server: 127.0.0.1"
echo "   - Port: 8080"
echo "7. Click 'OK'"
echo ""
echo "============================================"
echo "Setup complete!"
echo "============================================"
echo ""
echo "Next steps:"
echo ""
echo "1. Configure system proxy (see above)"
echo "2. Start proxy: ./scripts/start_warp_proxy_quiet.sh"
echo "3. Use Warp normally"
echo "4. View data: cat ~/Projects/splitrail/schemas/warp/captured/usage_data_*.jsonl | jq"
echo "5. Stop proxy: ./scripts/stop_warp_proxy.sh"
echo "6. DISABLE system proxy when done!"
echo ""
echo "Full guide: schemas/warp/MITMPROXY_SETUP.md"

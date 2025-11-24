#!/bin/bash
# Install mitmproxy certificate to macOS Keychain

MITM_DIR="$HOME/.mitmproxy"
CERT_FILE="$MITM_DIR/mitmproxy-ca-cert.pem"

echo "mitmproxy Certificate Installation"
echo "==================================="
echo ""

# Check if certificate exists
if [ ! -f "$CERT_FILE" ]; then
    echo "Certificate not found. You need to start mitmproxy first to generate it."
    echo ""
    echo "Run: ./scripts/start_warp_proxy_quiet.sh"
    echo "Wait a few seconds, then run: ./scripts/stop_warp_proxy.sh"
    echo "Then run this script again."
    exit 1
fi

echo "Found certificate: $CERT_FILE"
echo ""
echo "This will install the mitmproxy certificate to your macOS Keychain"
echo "and mark it as trusted for SSL connections."
echo ""
read -p "Continue? (y/n) " -n 1 -r
echo ""

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 1
fi

# Add certificate to keychain
echo "Adding certificate to System keychain (requires sudo)..."
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "$CERT_FILE"

if [ $? -eq 0 ]; then
    echo ""
    echo "✓ Certificate installed successfully!"
    echo ""
    echo "The mitmproxy certificate is now trusted by macOS."
    echo "Warp should now accept connections through the proxy."
else
    echo ""
    echo "✗ Failed to install certificate"
    echo ""
    echo "You can try installing manually:"
    echo "1. Open Keychain Access app"
    echo "2. Drag $CERT_FILE to the System keychain"
    echo "3. Double-click the certificate"
    echo "4. Expand 'Trust' section"
    echo "5. Set 'When using this certificate' to 'Always Trust'"
fi

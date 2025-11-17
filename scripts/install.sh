#!/bin/bash

# Splitrail Dashboard Installation Script
# This script downloads and installs a precompiled Splitrail binary

set -e  # Exit on any error

echo "🔍 Installing Splitrail Dashboard..."

# Detect the operating system and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Map architecture names to Rust-compatible names
case $ARCH in
    x86_64)
        ARCH="x86_64"
        ;;
    aarch64|arm64)
        ARCH="aarch64"
        ;;
    *)
        echo "❌ Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

# For Darwin (macOS), use apple-darwin
if [ "$OS" = "darwin" ]; then
    OS="apple-darwin"
    PLATFORM_NAME="macOS"
elif [ "$OS" = "linux" ]; then
    OS="unknown-linux-musl"
    PLATFORM_NAME="Linux"
else
    echo "❌ Unsupported operating system: $(uname -s)"
    exit 1
fi

BINARY_NAME="splitrail-$ARCH-$OS"
DOWNLOAD_URL="https://github.com/cloobfm/splitrail/releases/download/v2.0.0/$BINARY_NAME"

echo "📦 Detected platform: $PLATFORM_NAME ($ARCH)"

# Create temporary directory
TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

echo " ↓ Downloading Splitrail Dashboard binary..."

# Download the binary
if command -v curl &> /dev/null; then
    curl -L -o splitrail "$DOWNLOAD_URL" || {
        echo "❌ Failed to download binary from $DOWNLOAD_URL"
        echo "💡 Precompiled binaries are available at: https://github.com/cloobfm/splitrail/releases"
        echo "💡 Or build from source following the README instructions"
        exit 1
    }
elif command -v wget &> /dev/null; then
    wget -O splitrail "$DOWNLOAD_URL" || {
        echo "❌ Failed to download binary from $DOWNLOAD_URL"
        echo "💡 Precompiled binaries are available at: https://github.com/cloobfm/splitrail/releases"
        echo "💡 Or build from source following the README instructions"
        exit 1
    }
else
    echo "❌ Neither curl nor wget is available. Please install one and try again."
    exit 1
fi

# Make the binary executable
chmod +x splitrail

echo "💾 Installing Splitrail Dashboard to /usr/local/bin..."

# Install to system
if [ -w /usr/local/bin ]; then
    sudo cp splitrail /usr/local/bin/
else
    # If we can't write to /usr/local/bin, suggest user installation
    echo "⚠️  Cannot write to /usr/local/bin. Installing to ~/.local/bin instead..."
    mkdir -p ~/.local/bin
    cp splitrail ~/.local/bin/
    echo "💡 Add ~/.local/bin to your PATH to use splitrail:"
    echo "   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
    echo "   source ~/.zshrc"
fi

# Clean up
rm -rf "$TEMP_DIR"

echo "✅ Splitrail Dashboard installed successfully!"
echo ""
echo "🚀 Quick start:"
echo "   splitrail                    # Start the dashboard"
echo "   splitrail config init        # Initialize configuration"
echo "   splitrail --help            # Show available options"
#!/bin/bash

# Splitrail Dashboard Release Build Script
# Builds binaries for multiple platforms for distribution

set -e

echo "🔧 Building Splitrail Dashboard Release Binaries..."

# Check if Rust is installed
if ! command -v rustc &> /dev/null; then
    echo "❌ Rust is not installed. Please install Rust first from https://rust-lang.org/tools/install"
    exit 1
fi

# Check if rustup is available
if ! command -v rustup &> /dev/null; then
    echo "❌ rustup is not available. Please install Rust from https://rust-lang.org/tools/install"
    exit 1
fi

# Create releases directory
mkdir -p releases

# Get current platform information
ARCH=$(uname -m)
OS=$(uname -s)

# Determine the current platform binary name
if [ "$OS" = "Darwin" ]; then
    PLATFORM="macos"
    if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
        TARGET="aarch64-apple-darwin"
    else
        TARGET="x86_64-apple-darwin"
    fi
    BINARY_NAME="splitrail-dashboard-macos-$ARCH"
elif [ "$OS" = "Linux" ]; then
    PLATFORM="linux"
    if [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
        TARGET="aarch64-unknown-linux-musl"
    else
        TARGET="x86_64-unknown-linux-musl"
    fi
    BINARY_NAME="splitrail-dashboard-linux-$ARCH"
else
    echo "❌ Unsupported operating system: $OS"
    exit 1
fi

echo "📦 Detected platform: $PLATFORM ($ARCH)"
echo "🎯 Building for target: $TARGET"

# Install the target for current platform only
rustup target add $TARGET

echo "🔨 Building for current platform ($TARGET)..."
cargo +nightly build --release --target $TARGET
cp target/$TARGET/release/splitrail-dashboard "releases/$BINARY_NAME"

echo "✅ Splitrail Dashboard binary built successfully!"
echo "   Binary location: releases/$BINARY_NAME"

# Check if GitHub CLI is available and create release
if command -v gh &> /dev/null; then
    echo "🔗 GitHub CLI found. Creating release..."

    # Get version from Cargo.toml
    VERSION=$(grep -m 1 "version = " Cargo.toml | cut -d '"' -f 2)

    echo "🏷️  Creating release with version: $VERSION"

    gh release create "v$VERSION" "releases/$BINARY_NAME" \
        --title "Splitrail Dashboard v$VERSION" \
        --notes "Custom dashboard version with enhanced features for real-time token usage tracking."

    echo "🎉 GitHub release created successfully!"
    echo "   Available at: https://github.com/cloobfm/splitrail/releases/tag/v$VERSION"
else
    echo "💡 GitHub CLI not found. To create release manually:"
    echo "   1. Install GitHub CLI: brew install gh"
    echo "   2. Authenticate: gh auth login"
    echo "   3. Run: gh release create v2.0.0-dashboard.1 releases/$BINARY_NAME --title \"Splitrail Dashboard v2.0.0-dashboard.1\" --notes \"Custom dashboard version with enhanced features for real-time token usage tracking.\""
fi

echo ""
echo "💡 To build for additional platforms, install the required tools first:"
echo "   For Linux targets from macOS: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
echo "   You may need to install cross-compilation tools like: rust-musl-cross"
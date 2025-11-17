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

# Install cross-compilation targets if needed
echo "📦 Installing cross-compilation targets..."

# Install targets for different platforms
rustup target add x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl

# Create releases directory
mkdir -p releases

echo "🔨 Building for macOS (x86_64)..."
cargo +nightly build --release --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/splitrail-dashboard releases/splitrail-dashboard-macos-x86_64

echo "🔨 Building for macOS (aarch64)..."
cargo +nightly build --release --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/splitrail-dashboard releases/splitrail-dashboard-macos-aarch64

echo "🔨 Building for Linux (x86_64)..."
cargo +nightly build --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/splitrail-dashboard releases/splitrail-dashboard-linux-x86_64

echo "🔨 Building for Linux (aarch64)..."
cargo +nightly build --release --target aarch64-unknown-linux-musl
cp target/aarch64-unknown-linux-musl/release/splitrail-dashboard releases/splitrail-dashboard-linux-aarch64

echo "✅ Splitrail Dashboard release binaries built successfully!"
echo ""
echo "📁 Release binaries are located in the 'releases' directory:"
echo "   releases/splitrail-dashboard-macos-x86_64"
echo "   releases/splitrail-dashboard-macos-aarch64"
echo "   releases/splitrail-dashboard-linux-x86_64"
echo "   releases/splitrail-dashboard-linux-aarch64"
echo ""
echo "📦 To upload to GitHub Releases:"
echo "   Use GitHub CLI: gh release create v2.0.0-dashboard.1 releases/*"
echo "   Or upload manually to: https://github.com/cloobfm/splitrail/releases"
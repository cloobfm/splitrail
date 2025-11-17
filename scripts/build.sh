#!/bin/bash

# Splitrail Dashboard Build Script
# Builds the dashboard version with custom naming

set -e

echo "🔧 Building Splitrail Dashboard..."

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

# Build with nightly toolchain (as specified in rust-toolchain.toml)
echo "🔨 Building Splitrail Dashboard with nightly toolchain..."
cargo +nightly build --release

# Copy the binary with dashboard-specific naming
BINARY_NAME="splitrail-dashboard"
cp target/release/splitrail target/release/$BINARY_NAME

echo "✅ Splitrail Dashboard built successfully!"
echo "   Binary location: target/release/$BINARY_NAME"
echo ""
echo "🚀 To install system-wide:"
echo "   sudo cp target/release/$BINARY_NAME /usr/local/bin/"
echo ""
echo "📦 To create portable binary for distribution:"
echo "   cp target/release/$BINARY_NAME ."
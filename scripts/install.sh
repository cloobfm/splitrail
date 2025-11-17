#!/bin/bash

# Splitrail Dashboard Installation Script
# This script builds and installs Splitrail from source

set -e  # Exit on any error

echo "🔍 Installing Splitrail Dashboard..."

# Check if Rust is installed
if ! command -v rustc &> /dev/null; then
    echo "❌ Rust is not installed. Please install Rust first:"
    echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check if Git is installed
if ! command -v git &> /dev/null; then
    echo "❌ Git is not installed. Please install Git first."
    exit 1
fi

# Ensure nightly toolchain is installed
echo "🔧 Checking for nightly Rust toolchain..."
if ! rustup toolchain list | grep -q "nightly"; then
    echo "📦 Installing nightly Rust toolchain..."
    rustup toolchain install nightly
fi

# Create temporary directory
TEMP_DIR=$(mktemp -d)
echo "📦 Cloning Splitrail Dashboard repository..."

# Clone the repository
git clone https://github.com/cloobfm/splitrail.git -b dashboard "$TEMP_DIR/splitrail"

cd "$TEMP_DIR/splitrail"

echo "🔨 Building Splitrail Dashboard with nightly toolchain (this may take a few minutes)..."

# Build in release mode using nightly toolchain (as specified in rust-toolchain.toml)
rustup run nightly cargo build --release

echo "💾 Installing Splitrail Dashboard to /usr/local/bin..."

# Install to system
if [ -w /usr/local/bin ]; then
    sudo cp target/release/splitrail /usr/local/bin/
else
    # If we can't write to /usr/local/bin, suggest user installation
    echo "⚠️  Cannot write to /usr/local/bin. Installing to ~/.local/bin instead..."
    mkdir -p ~/.local/bin
    cp target/release/splitrail ~/.local/bin/
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
echo ""
echo "💡 Note: This version requires Rust nightly toolchain (as configured in rust-toolchain.toml)."
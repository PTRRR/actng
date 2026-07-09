#!/bin/bash
set -e

# Determine installation directory
INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"

echo "Building actng..."
cargo build --release

echo "Installing binary to $INSTALL_DIR..."
cp target/release/actng-cli "$INSTALL_DIR/actng"

# Check if directory is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo ""
    echo "Warning: $INSTALL_DIR is not in your PATH."
    echo "Please add it by adding this line to your shell profile (.zshrc or .bashrc):"
    echo "export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

echo "Installation complete! You can now use 'actng'."

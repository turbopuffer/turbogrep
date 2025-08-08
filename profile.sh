#!/bin/bash

set -e

# Check if cargo-flamegraph is installed
if ! command -v cargo &>/dev/null || ! cargo --list | grep -q flamegraph; then
    echo "Installing cargo-flamegraph..."
    cargo install flamegraph
fi

# Check if test repositories exist
VENDOR_DIR="tests/vendor"
REPO_NAME="rails"
REPO_PATH="$VENDOR_DIR/$REPO_NAME"

if [ ! -d "$REPO_PATH" ]; then
    echo "Test repository not found at $REPO_PATH"
    echo "Please run 'cargo test' first to clone the test repositories."
    echo "This will set up the vendor directories needed for profiling."
    exit 1
else
    echo "Using test repository: $REPO_PATH"
fi

# Run cargo flamegraph on the chunking operation
echo "Running flamegraph on chunking operation..."
echo "This will profile: cargo run --release -- --chunk-only $REPO_PATH"

# Run flamegraph and capture the SVG filename
sudo cargo flamegraph --bin turbogrep -- --chunk-only "$REPO_PATH"

# Find the most recent flamegraph file
FLAMEGRAPH_FILE=$(ls -t flamegraph*.svg 2>/dev/null | head -n1)

if [ -n "$FLAMEGRAPH_FILE" ] && [ -f "$FLAMEGRAPH_FILE" ]; then
    echo "Flamegraph generated: $FLAMEGRAPH_FILE"

    # Open the flamegraph in the browser
    if command -v open &>/dev/null; then
        # macOS
        echo "Opening flamegraph in browser..."
        open "$FLAMEGRAPH_FILE"
    elif command -v xdg-open &>/dev/null; then
        # Linux
        echo "Opening flamegraph in browser..."
        xdg-open "$FLAMEGRAPH_FILE"
    else
        echo "Please open $FLAMEGRAPH_FILE in your browser manually"
    fi
else
    echo "Error: Could not find flamegraph output file"
    exit 1
fi

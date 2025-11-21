#!/bin/bash
set -e

echo "🔨 Building Servo release artifacts..."

# Build all crates in release mode
cargo build --release --all

echo "📦 Building CLI binary..."
cargo build --release --bin servo

echo "✅ Release build complete!"
echo ""
echo "Artifacts:"
echo "  CLI: target/release/servo"

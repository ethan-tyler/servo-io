#!/bin/bash
set -euo pipefail

echo "Building Servo release artifacts..."

cargo build --release --workspace

echo "Release build complete."
echo "Artifacts:"
echo "  CLI binary: target/release/servo"

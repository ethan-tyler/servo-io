#!/bin/bash
set -euo pipefail

echo "Running Servo test suite..."

export DATABASE_URL=${DATABASE_URL:-postgresql://servo:servo@localhost:5432/servo_dev}

echo "Executing Rust tests..."
cargo test --workspace --all-features

echo "Test suite completed successfully."

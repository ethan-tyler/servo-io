#!/bin/bash
set -e

echo "🧪 Running Servo test suite..."

# Export test database URL
export DATABASE_URL=postgresql://servo:servo@localhost:5432/servo_dev

# Rust tests
echo "📦 Running Rust tests..."
cargo test --all-features

echo "✅ All tests passed!"

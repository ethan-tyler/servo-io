#!/bin/bash
set -e

echo "🚀 Setting up Servo development environment..."

# Check prerequisites
command -v rustc >/dev/null 2>&1 || { echo "❌ Rust not found. Install from https://rustup.rs/"; exit 1; }
command -v docker >/dev/null 2>&1 || { echo "⚠️  Docker not found. PostgreSQL setup will be skipped."; }

echo "✅ Prerequisites check passed"

# Install Rust components
echo "📦 Installing Rust components..."
rustup component add rustfmt clippy

# Build Rust workspace
echo "🔨 Building Rust workspace..."
cargo build

# Setup PostgreSQL
if command -v docker >/dev/null 2>&1; then
    echo "🐘 Starting PostgreSQL in Docker..."
    docker run -d \
        --name servo-postgres \
        -e POSTGRES_PASSWORD=servo \
        -e POSTGRES_USER=servo \
        -e POSTGRES_DB=servo_dev \
        -p 5432:5432 \
        postgres:15 || echo "PostgreSQL container already running"

    # Wait for PostgreSQL to be ready
    echo "⏳ Waiting for PostgreSQL..."
    sleep 5

    # Run migrations
    echo "📊 Running database migrations..."
    export DATABASE_URL=postgresql://servo:servo@localhost:5432/servo_dev
    cargo run --bin servo -- init --database-url $DATABASE_URL
fi

echo "✅ Development environment setup complete!"
echo ""
echo "Next steps:"
echo "  1. Run tests: ./scripts/run-tests.sh"
echo "  2. Build CLI: cargo build --release --bin servo"
echo "  3. Start coding! 🎉"

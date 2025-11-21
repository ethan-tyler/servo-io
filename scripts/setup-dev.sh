#!/bin/bash
set -euo pipefail

echo "Setting up the Servo development environment..."

command -v rustc >/dev/null 2>&1 || { echo "Rust not found. Install from https://rustup.rs/"; exit 1; }
if ! command -v docker >/dev/null 2>&1; then
    echo "Docker not found. PostgreSQL setup will be skipped. Set DATABASE_URL manually if you want to run migrations locally."
fi

echo "Prerequisites check passed."

echo "Installing Rust components..."
rustup component add rustfmt clippy

echo "Building Rust workspace..."
cargo build --workspace

if command -v docker >/dev/null 2>&1; then
    echo "Starting PostgreSQL in Docker..."
    docker run -d \
        --name servo-postgres \
        -e POSTGRES_PASSWORD=servo \
        -e POSTGRES_USER=servo \
        -e POSTGRES_DB=servo_dev \
        -p 5432:5432 \
        postgres:15 >/dev/null 2>&1 || echo "PostgreSQL container already running."

    echo "Waiting for PostgreSQL to become available..."
    ready=0
    for _ in {1..10}; do
        if docker exec servo-postgres pg_isready -U servo -d servo_dev >/dev/null 2>&1; then
            ready=1
            break
        fi
        sleep 2
    done

    if [ "$ready" -ne 1 ]; then
        echo "PostgreSQL did not become ready. Check the container logs and rerun the script."
        exit 1
    fi

    echo "Running database migrations..."
    export DATABASE_URL=postgresql://servo:servo@localhost:5432/servo_dev
    cargo run --bin servo -- init --database-url "$DATABASE_URL"
fi

echo "Development environment setup complete."
echo
echo "Next steps:"
echo "  1. Run tests: ./scripts/run-tests.sh"
echo "  2. Build CLI: cargo build --release --bin servo"
echo "  3. Start coding!"

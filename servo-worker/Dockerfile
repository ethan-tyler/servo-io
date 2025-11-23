# Multi-stage Docker build for Servo Cloud Run Worker
# Optimized for small image size and fast cold starts

# Stage 1: Build dependencies (cached layer)
FROM rust:1.85-slim AS chef
WORKDIR /app
RUN cargo install cargo-chef

# Stage 2: Prepare recipe (dependency caching)
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Build dependencies (cached)
FROM chef AS builder-deps
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Stage 4: Build application
FROM rust:1.85-slim AS builder
WORKDIR /app

# Install build dependencies
RUN apt-get update && \
    apt-get install -y \
        pkg-config \
        libssl-dev \
        && \
    rm -rf /var/lib/apt/lists/*

# Copy dependency artifacts from cache
COPY --from=builder-deps /app/target target
COPY --from=builder-deps /usr/local/cargo /usr/local/cargo

# Copy source code
COPY . .

# Build only the worker binary
RUN cargo build --release --bin servo-worker

# Stage 5: Runtime image (minimal)
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y \
        ca-certificates \
        libssl3 \
        && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 servo && \
    mkdir -p /app && \
    chown -R servo:servo /app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/servo-worker /app/servo-worker

# Switch to non-root user
USER servo

# Cloud Run expects PORT environment variable
ENV PORT=8080
EXPOSE 8080

# Health check (optional, Cloud Run has built-in health checks)
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:${PORT}/health || exit 1

# Run the worker
CMD ["/app/servo-worker"]

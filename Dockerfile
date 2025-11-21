# Multi-stage Dockerfile for Servo
# Build stage
FROM rust:1.75-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# Copy all crate manifests to enable dependency caching
COPY servo-core/Cargo.toml ./servo-core/
COPY servo-runtime/Cargo.toml ./servo-runtime/
COPY servo-lineage/Cargo.toml ./servo-lineage/
COPY servo-storage/Cargo.toml ./servo-storage/
COPY servo-cloud-gcp/Cargo.toml ./servo-cloud-gcp/
COPY servo-cli/Cargo.toml ./servo-cli/

# Create dummy source files to build dependencies
RUN mkdir -p servo-core/src servo-runtime/src servo-lineage/src \
    servo-storage/src servo-cloud-gcp/src servo-cli/src && \
    echo "fn main() {}" > servo-cli/src/main.rs && \
    echo "pub fn dummy() {}" > servo-core/src/lib.rs && \
    echo "pub fn dummy() {}" > servo-runtime/src/lib.rs && \
    echo "pub fn dummy() {}" > servo-lineage/src/lib.rs && \
    echo "pub fn dummy() {}" > servo-storage/src/lib.rs && \
    echo "pub fn dummy() {}" > servo-cloud-gcp/src/lib.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release --package servo-cli && \
    rm -rf servo-*/src

# Copy actual source code
COPY servo-core ./servo-core
COPY servo-runtime ./servo-runtime
COPY servo-lineage ./servo-lineage
COPY servo-storage ./servo-storage
COPY servo-cloud-gcp ./servo-cloud-gcp
COPY servo-cli ./servo-cli

# Build the actual application
RUN cargo build --release --package servo-cli

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 servo

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/servo /usr/local/bin/servo

# Copy migrations if they exist
COPY --chown=servo:servo servo-storage/migrations ./migrations

USER servo

EXPOSE 8080

ENTRYPOINT ["servo"]
CMD ["--help"]

# Metadata
LABEL org.opencontainers.image.title="Servo"
LABEL org.opencontainers.image.description="Serverless Data Orchestration with Asset Lineage"
LABEL org.opencontainers.image.url="https://github.com/ethan-tyler/servo-io"
LABEL org.opencontainers.image.documentation="https://docs.servo.dev"
LABEL org.opencontainers.image.source="https://github.com/ethan-tyler/servo-io"
LABEL org.opencontainers.image.licenses="Apache-2.0"

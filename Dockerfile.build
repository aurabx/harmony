# Build stage
FROM rust:1.87-bookworm AS builder

WORKDIR /app

# Copy Cargo files first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Copy source code
COPY src/ ./src/

# Build release binary
RUN cargo build --release --bin harmony

# Runtime stage
FROM debian:bookworm-slim

# Install minimal dependencies
RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Create directories
RUN mkdir -p /etc/harmony /var/log/harmony /tmp/harmony

# Copy binary from builder
COPY --from=builder /app/target/release/harmony /usr/local/bin/harmony

# Expose ports
EXPOSE 8080 9090

# Default command
CMD ["harmony", "--config", "/etc/harmony/config.toml"]
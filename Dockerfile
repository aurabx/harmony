# syntax=docker/dockerfile:1
FROM debian:stable-slim

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libdbus-1-3 && rm -rf /var/lib/apt/lists/*

# Create required directories for Harmony operation
# Note: RUNBEAM_ENCRYPTION_KEY should be provided via environment variable
# for secure token storage (encrypted filesystem). Without this variable,
# Harmony will auto-generate an encryption key (not persistent across restarts).
# For production: set RUNBEAM_ENCRYPTION_KEY to ensure tokens persist.
# See docs/security.md for key generation and deployment examples.
RUN mkdir -p /etc/harmony /var/log/harmony /tmp/harmony

# Copy in prebuilt binary (for CI or local use)
ARG TARGETARCH
COPY harmony-${TARGETARCH} /usr/local/bin/harmony

EXPOSE 8080 9090
CMD ["harmony", "--config", "/etc/harmony/config.toml"]

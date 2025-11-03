# syntax=docker/dockerfile:1
FROM debian:bookworm-slim

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

# Create required directories for Harmony operation
# Note: RUNBEAM_ENCRYPTION_KEY can be provided via environment variable
# for secure token storage when OS keyring is unavailable (typical in containers).
# Without this variable, Harmony will auto-generate an encryption key stored
# in the container filesystem (not persistent across container restarts).
# For production: set RUNBEAM_ENCRYPTION_KEY to ensure tokens persist.
# See docs/security.md for key generation and deployment examples.
RUN mkdir -p /etc/harmony /var/log/harmony /tmp/harmony

# Copy in prebuilt binary (for CI or local use)
ARG TARGETARCH
COPY harmony-${TARGETARCH} /usr/local/bin/harmony

EXPOSE 8080 9090
CMD ["harmony", "--config", "/etc/harmony/config.toml"]

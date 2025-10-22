# syntax=docker/dockerfile:1
FROM debian:bookworm-slim

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /etc/harmony /var/log/harmony /tmp/harmony

# Copy in prebuilt binary (for CI or local use)
ARG TARGETARCH
COPY harmony-${TARGETARCH} /usr/local/bin/harmony

EXPOSE 8080 9090
CMD ["harmony", "--config", "/etc/harmony/config.toml"]

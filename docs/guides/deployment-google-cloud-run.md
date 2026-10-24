---
title: Deploying Harmony Proxy to Google Cloud Run
description: Step-by-step guide to deploying Harmony on Google Cloud Run.
status: active
updated: 2026-10-24
type: Guide
---

# Deploying Harmony Proxy to Google Cloud Run

This guide covers deploying Harmony Proxy to Google Cloud Run, a fully managed serverless container platform.

## Prerequisites

- Google Cloud project with billing enabled
- `gcloud` CLI installed and authenticated
- Docker (if building custom images)
- `age-keygen` (for generating encryption keys): `brew install age` (macOS) or `apt-get install age` (Linux)

## Quick Start

The fastest way to deploy Harmony to Cloud Run:

```bash
# Set your project and region
gcloud config set project YOUR_PROJECT_ID
export REGION=us-central1

# Create a basic configuration file
cat > config.toml <<EOF
[proxy]
id = "harmony-cloudrun"
log_level = "info"

[storage]
backend = "filesystem"
path = "/tmp/harmony"

[network.external]
enable_wireguard = false
interface = "wg0"

[network.external.http]
bind_address = "0.0.0.0"
bind_port = 8080

[pipelines.echo]
description = "Basic echo pipeline"
networks = ["external"]
endpoints = ["http_echo"]
backends = ["echo_backend"]
middleware = []

[endpoints.http_echo]
service = "http"
[endpoints.http_echo.options]
path_prefix = "/echo"

[backends.echo_backend]
service = "echo"

[services.http]
module = ""
[services.echo]
module = ""
EOF

# Store config in Secret Manager
gcloud secrets create harmony-config --data-file=config.toml

# Generate and store encryption key
age-keygen | base64 | tr -d '\n' | gcloud secrets create harmony-encryption-key --data-file=-

# Deploy
gcloud run deploy harmony-proxy \
  --image ghcr.io/aurabx/harmony:latest \
  --region $REGION \
  --platform managed \
  --port 8080 \
  --allow-unauthenticated \
  --memory 1Gi \
  --cpu 2 \
  --min-instances 0 \
  --max-instances 10 \
  --set-env-vars "RUST_LOG=harmony=info" \
  --set-secrets "RUNBEAM_ENCRYPTION_KEY=harmony-encryption-key:latest" \
  --update-secrets "/etc/harmony/config.toml=harmony-config:latest"

# Test the deployment
SERVICE_URL=$(gcloud run services describe harmony-proxy --region $REGION --format 'value(status.url)')
curl -i $SERVICE_URL/echo
```

## Deployment Options

### Option 1: Deploy from GitHub Container Registry (Recommended)

Use the pre-built images published to GitHub Container Registry:

```bash
gcloud run deploy harmony-proxy \
  --image ghcr.io/aurabx/harmony:latest \
  --region us-central1 \
  --platform managed \
  --port 8080 \
  --allow-unauthenticated \
  --memory 512Mi \
  --cpu 1 \
  --min-instances 0 \
  --max-instances 10
```

**Pros:**
- Fastest deployment
- Uses official builds with verified binaries
- Automatic updates available by redeploying with `:latest`

**Cons:**
- Requires internet access to GHCR
- No custom modifications

### Option 2: Build and Deploy from Source

Build using Google Cloud Build and deploy to Google Container Registry:

```bash
# Clone the repository
git clone https://github.com/aurabx/harmony.git
cd harmony

# Build using Cloud Build
gcloud builds submit --tag gcr.io/YOUR_PROJECT_ID/harmony-proxy

# Deploy
gcloud run deploy harmony-proxy \
  --image gcr.io/YOUR_PROJECT_ID/harmony-proxy \
  --region us-central1 \
  --platform managed \
  --port 8080
```

**Pros:**
- Full control over build process
- Can apply custom patches
- Image stored in your GCP project

**Cons:**
- Longer build times
- Requires Cloud Build API enabled
- Additional storage costs

### Option 3: Build Custom Image with Embedded Config

For simpler deployments with static configuration:

```bash
# Create a custom Dockerfile
cat > Dockerfile.cloudrun <<EOF
FROM ghcr.io/aurabx/harmony:latest
COPY config.toml /etc/harmony/config.toml
COPY transforms/ /etc/harmony/transforms/
EOF

# Build and push
docker build -f Dockerfile.cloudrun -t gcr.io/YOUR_PROJECT_ID/harmony-proxy .
docker push gcr.io/YOUR_PROJECT_ID/harmony-proxy

# Deploy
gcloud run deploy harmony-proxy \
  --image gcr.io/YOUR_PROJECT_ID/harmony-proxy \
  --region us-central1 \
  --port 8080
```

**Pros:**
- Simple configuration management
- No external dependencies at runtime
- Good for static configs

**Cons:**
- Requires rebuild for config changes
- Larger image size
- Less flexible

## Google Cloud Run: Configuration

How to manage configuration, environment variables, and Cloud Run settings for Harmony. See [Google Cloud Run: Configuration](deployment-google-cloud-run-configuration.md).

## Google Cloud Run: Networking and IAM

How to set up VPC networking, authentication, and IAM for Harmony on Cloud Run. See [Google Cloud Run: Networking and IAM](deployment-google-cloud-run-networking.md).

## Google Cloud Run: Production

Monitoring, a complete production example, troubleshooting, and cost guidance for Cloud Run. See [Google Cloud Run: Production](deployment-google-cloud-run-production.md).

## Next Steps

- Review [Configuration Documentation](../configuration/configuration-file.md) for advanced options
- See [Security Documentation](../security/overview.md) for production hardening
- Explore [Management API](../api/management-api.md) for operational endpoints

## Additional Resources

- [Google Cloud Run Documentation](https://cloud.google.com/run/docs)
- [Secret Manager Documentation](https://cloud.google.com/secret-manager/docs)
- [Cloud Run Pricing](https://cloud.google.com/run/pricing)
- [Harmony Proxy Documentation](../../readme.md)

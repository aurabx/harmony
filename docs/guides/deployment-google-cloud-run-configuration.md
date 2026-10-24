---
title: Google Cloud Run: Configuration
description: How to manage configuration, environment variables, and Cloud Run settings for Harmony.
status: active
updated: 2026-10-24
type: Guide
related:
  - guides/deployment-google-cloud-run.md
---

# Google Cloud Run: Configuration

How to manage configuration, environment variables, and Cloud Run settings for Harmony. This page is part of [Deploying Harmony Proxy to Google Cloud Run](deployment-google-cloud-run.md).

## Configuration Management

Harmony requires a TOML configuration file. Choose the approach that fits your workflow:

### Method 1: Secret Manager (Recommended for Production)

Store configuration in Google Cloud Secret Manager for secure, centralized management:

```bash
# Create secret from file
gcloud secrets create harmony-config --data-file=config.toml

# Update secret
gcloud secrets versions add harmony-config --data-file=config.toml

# Deploy with secret mounted as file
gcloud run deploy harmony-proxy \
  --image ghcr.io/aurabx/harmony:latest \
  --region us-central1 \
  --update-secrets "/etc/harmony/config.toml=harmony-config:latest"
```

**Pros:**
- Secure storage with access controls
- Version history
- Easy updates without redeployment
- Audit logging

**Cons:**
- Additional GCP service to manage
- Slight overhead at container startup

### Method 2: Cloud Storage Volume Mount

Mount configuration from a Cloud Storage bucket:

```bash
# Create bucket and upload config
gsutil mb gs://YOUR_BUCKET_NAME-harmony-config
gsutil cp config.toml gs://YOUR_BUCKET_NAME-harmony-config/config.toml
gsutil cp -r transforms/ gs://YOUR_BUCKET_NAME-harmony-config/transforms/

# Deploy with volume mount
gcloud run deploy harmony-proxy \
  --image ghcr.io/aurabx/harmony:latest \
  --region us-central1 \
  --add-volume name=config,type=cloud-storage,bucket=YOUR_BUCKET_NAME-harmony-config \
  --add-volume-mount volume=config,mount-path=/etc/harmony
```

**Pros:**
- Easy to update (just upload new files)
- Can store multiple files and directories
- Good for large transform specifications

**Cons:**
- Bucket permissions management
- Requires FUSE (enabled by default on Cloud Run)
- Slight mount overhead

### Method 3: Embedded in Image

For static configurations that rarely change:

```dockerfile
FROM ghcr.io/aurabx/harmony:latest
COPY config.toml /etc/harmony/config.toml
```

**Pros:**
- Simplest deployment
- Fastest cold start
- No runtime dependencies

**Cons:**
- Requires image rebuild for changes
- Not suitable for dynamic configurations

## Environment Variables

### Required Variables

Set essential environment variables for production:

```bash
gcloud run deploy harmony-proxy \
  --set-env-vars "RUST_LOG=harmony=info" \
  --set-secrets "RUNBEAM_ENCRYPTION_KEY=harmony-encryption-key:latest"
```

### Encryption Key Setup

Harmony requires an encryption key for secure token storage. Generate and store it:

```bash
# Generate key and store in Secret Manager
age-keygen | base64 | tr -d '\n' | gcloud secrets create harmony-encryption-key --data-file=-

# Deploy with secret
gcloud run deploy harmony-proxy \
  --set-secrets "RUNBEAM_ENCRYPTION_KEY=harmony-encryption-key:latest"
```

### Runbeam Cloud Integration

For Runbeam Cloud integration, add the JWT secret:

```bash
# Store JWT secret
echo -n "your-jwt-secret" | gcloud secrets create harmony-jwt-secret --data-file=-

# Deploy with Runbeam Cloud support
gcloud run deploy harmony-proxy \
  --set-env-vars "RUST_LOG=harmony=info" \
  --set-secrets "RUNBEAM_ENCRYPTION_KEY=harmony-encryption-key:latest" \
  --set-secrets "RUNBEAM_JWT_SECRET=harmony-jwt-secret:latest"
```

Update your configuration to enable Runbeam integration:

```toml
[runbeam]
enabled = true
cloud_api_base_url = "https://api.runbeam.cloud"
poll_interval_secs = 30
```

### All Available Environment Variables

| Variable | Purpose | Required | Example |
|----------|---------|----------|---------|
| `RUST_LOG` | Logging verbosity | No | `harmony=info` |
| `RUNBEAM_ENCRYPTION_KEY` | Token encryption key | Yes (production) | Base64-encoded age key |
| `RUNBEAM_JWT_SECRET` | JWT validation secret | With Runbeam Cloud | Your secret key |
| `RUNBEAM_MACHINE_TOKEN` | Pre-provisioned token | Headless deployments | JSON token |
| `RUNBEAM_DISABLE_KEYRING` | Force filesystem storage | Testing only | `1` |

See [Security Documentation](../security/environment-variables.md#environment-variables) for complete details.

## Cloud Run Specific Configuration

### Port Configuration

Cloud Run requires your service to listen on the port specified by the `PORT` environment variable (default 8080). Ensure your config matches:

```toml
[network.external.http]
bind_address = "0.0.0.0"
bind_port = 8080
```

### Storage Backend

Cloud Run provides ephemeral storage at `/tmp`. Configure Harmony accordingly:

```toml
[storage]
backend = "filesystem"
path = "/tmp/harmony"
```

**Important:** Files in `/tmp` are lost when the container stops. For persistent storage:
- Use Cloud Storage buckets with volume mounts
- Use database-backed storage (future feature)
- Ensure critical data is stored externally

### Resource Limits

Configure appropriate resources based on your workload:

```bash
gcloud run deploy harmony-proxy \
  --memory 1Gi \        # 256Mi, 512Mi, 1Gi, 2Gi, 4Gi, 8Gi
  --cpu 2 \             # 1, 2, 4, 6, 8
  --timeout 300 \       # Request timeout (max 3600s)
  --concurrency 80      # Max concurrent requests per instance
```

**Recommendations:**
- **Light workload** (basic HTTP proxy): 512Mi memory, 1 CPU
- **Medium workload** (FHIR/transforms): 1Gi memory, 2 CPUs
- **Heavy workload** (DICOM/large files): 2-4Gi memory, 2-4 CPUs

### Scaling Configuration

Control how Cloud Run scales your service:

```bash
gcloud run deploy harmony-proxy \
  --min-instances 1 \      # Keep warm instances (0 for scale-to-zero)
  --max-instances 100 \    # Maximum instances
  --concurrency 80         # Requests per instance
```

**Scaling strategies:**
- **Development/testing:** `--min-instances 0` (scale to zero)
- **Production (latency-sensitive):** `--min-instances 1-5` (always warm)
- **Production (cost-optimized):** `--min-instances 0` with appropriate timeouts
- **High traffic:** `--max-instances 100+` with load testing

### Management API

If using the management API, note that Cloud Run exposes only one port externally. Options:

**Option 1: Use the same port with path-based routing**

```toml
[network.external]
[network.external.http]
bind_address = "0.0.0.0"
bind_port = 8080

[management]
enabled = true
base_path = "admin"
network = "external"
```

Access: `https://your-service-url.run.app/admin/health`

**Option 2: Deploy separate service for management**

Deploy two services: one for client traffic, one for management:

```bash
# Main service
gcloud run deploy harmony-proxy \
  --image ghcr.io/aurabx/harmony:latest \
  --allow-unauthenticated

# Management service (restricted)
gcloud run deploy harmony-proxy-admin \
  --image ghcr.io/aurabx/harmony:latest \
  --no-allow-unauthenticated \
  --update-secrets "/etc/harmony/config-admin.toml=harmony-config-admin:latest"
```

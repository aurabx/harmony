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

See [Security Documentation](../security.md#environment-variables) for complete details.

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

## VPC and Networking

### VPC Access

If Harmony needs to access resources in your VPC (databases, internal services):

```bash
# Create VPC connector (one-time setup)
gcloud compute networks vpc-access connectors create harmony-connector \
  --region us-central1 \
  --subnet YOUR_SUBNET_NAME \
  --subnet-project YOUR_PROJECT_ID

# Deploy with VPC access
gcloud run deploy harmony-proxy \
  --vpc-connector harmony-connector \
  --vpc-egress all-traffic  # or private-ranges-only
```

### Ingress Control

Restrict which traffic can reach your service:

```bash
# Allow all (default)
gcloud run deploy harmony-proxy --ingress all

# Allow only from VPC or Cloud Load Balancer
gcloud run deploy harmony-proxy --ingress internal-and-cloud-load-balancing

# Allow only from VPC
gcloud run deploy harmony-proxy --ingress internal
```

### Custom Domain

Map a custom domain to your service:

```bash
# Map domain
gcloud run domain-mappings create --service harmony-proxy --domain api.example.com --region us-central1

# Verify DNS settings
gcloud run domain-mappings describe --domain api.example.com --region us-central1
```

## Authentication and IAM

### Public Access (Testing/Development)

```bash
gcloud run deploy harmony-proxy --allow-unauthenticated
```

### Restricted Access (Production)

```bash
# Require authentication
gcloud run deploy harmony-proxy --no-allow-unauthenticated

# Grant access to specific service account
gcloud run services add-iam-policy-binding harmony-proxy \
  --region us-central1 \
  --member="serviceAccount:caller@project.iam.gserviceaccount.com" \
  --role="roles/run.invoker"

# Grant access to specific user
gcloud run services add-iam-policy-binding harmony-proxy \
  --region us-central1 \
  --member="user:user@example.com" \
  --role="roles/run.invoker"
```

### Identity and Access Management

For production deployments, use a custom service account:

```bash
# Create service account
gcloud iam service-accounts create harmony-proxy-sa \
  --display-name "Harmony Proxy Service Account"

# Grant necessary permissions
gcloud secrets add-iam-policy-binding harmony-config \
  --member="serviceAccount:harmony-proxy-sa@PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/secretmanager.secretAccessor"

gcloud storage buckets add-iam-policy-binding gs://YOUR_BUCKET_NAME \
  --member="serviceAccount:harmony-proxy-sa@PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/storage.objectViewer"

# Deploy with custom service account
gcloud run deploy harmony-proxy \
  --service-account harmony-proxy-sa@PROJECT_ID.iam.gserviceaccount.com
```

## Monitoring and Logging

### View Logs

```bash
# Stream logs in real-time
gcloud run services logs tail harmony-proxy --region us-central1

# View recent logs
gcloud run services logs read harmony-proxy --region us-central1 --limit 100

# Filter by severity
gcloud run services logs read harmony-proxy --region us-central1 --log-filter="severity>=ERROR"
```

### Cloud Logging Integration

Harmony's structured logs are automatically sent to Cloud Logging. Use Log Explorer:

```
resource.type="cloud_run_revision"
resource.labels.service_name="harmony-proxy"
severity>=ERROR
```

### Metrics and Monitoring

View metrics in Cloud Console:
- Request count and latency
- Instance count
- CPU and memory utilization
- Error rates

Access at: `https://console.cloud.google.com/run/detail/REGION/harmony-proxy/metrics`

### Create Alerts

```bash
# Example: Alert on error rate
gcloud alpha monitoring policies create \
  --notification-channels=CHANNEL_ID \
  --display-name="Harmony Error Rate" \
  --condition-display-name="Error rate > 5%" \
  --condition-expression='
    resource.type = "cloud_run_revision"
    AND metric.type = "run.googleapis.com/request_count"
    AND metric.labels.response_code_class = "5xx"
  '
```

## Complete Production Example

Here's a complete script for production deployment:

```bash
#!/bin/bash
set -euo pipefail

# Configuration
PROJECT_ID="your-project-id"
REGION="us-central1"
SERVICE_NAME="harmony-proxy"
SERVICE_ACCOUNT="harmony-proxy-sa"
MIN_INSTANCES=1
MAX_INSTANCES=10
MEMORY="1Gi"
CPU=2

# Set project
gcloud config set project $PROJECT_ID

# Enable required APIs
gcloud services enable \
  run.googleapis.com \
  secretmanager.googleapis.com \
  compute.googleapis.com

# Create service account
gcloud iam service-accounts create $SERVICE_ACCOUNT \
  --display-name "Harmony Proxy Service Account" \
  --project $PROJECT_ID || true

# Generate and store encryption key
echo "Generating encryption key..."
age-keygen | base64 | tr -d '\n' | \
  gcloud secrets create harmony-encryption-key --data-file=- --project $PROJECT_ID || \
  (age-keygen | base64 | tr -d '\n' | \
   gcloud secrets versions add harmony-encryption-key --data-file=- --project $PROJECT_ID)

# Store JWT secret (replace with your actual secret)
echo -n "your-production-jwt-secret" | \
  gcloud secrets create harmony-jwt-secret --data-file=- --project $PROJECT_ID || \
  (echo -n "your-production-jwt-secret" | \
   gcloud secrets versions add harmony-jwt-secret --data-file=- --project $PROJECT_ID)

# Store configuration
gcloud secrets create harmony-config --data-file=config.toml --project $PROJECT_ID || \
  gcloud secrets versions add harmony-config --data-file=config.toml --project $PROJECT_ID

# Grant service account access to secrets
for SECRET in harmony-config harmony-encryption-key harmony-jwt-secret; do
  gcloud secrets add-iam-policy-binding $SECRET \
    --member="serviceAccount:${SERVICE_ACCOUNT}@${PROJECT_ID}.iam.gserviceaccount.com" \
    --role="roles/secretmanager.secretAccessor" \
    --project $PROJECT_ID
done

# Deploy service
gcloud run deploy $SERVICE_NAME \
  --image ghcr.io/aurabx/harmony:latest \
  --region $REGION \
  --platform managed \
  --port 8080 \
  --no-allow-unauthenticated \
  --memory $MEMORY \
  --cpu $CPU \
  --min-instances $MIN_INSTANCES \
  --max-instances $MAX_INSTANCES \
  --timeout 300 \
  --concurrency 80 \
  --service-account "${SERVICE_ACCOUNT}@${PROJECT_ID}.iam.gserviceaccount.com" \
  --set-env-vars "RUST_LOG=harmony=info" \
  --set-secrets "RUNBEAM_ENCRYPTION_KEY=harmony-encryption-key:latest" \
  --set-secrets "RUNBEAM_JWT_SECRET=harmony-jwt-secret:latest" \
  --update-secrets "/etc/harmony/config.toml=harmony-config:latest" \
  --project $PROJECT_ID

# Get service URL
SERVICE_URL=$(gcloud run services describe $SERVICE_NAME \
  --region $REGION \
  --project $PROJECT_ID \
  --format 'value(status.url)')

echo ""
echo "✅ Deployment complete!"
echo "Service URL: $SERVICE_URL"
echo ""
echo "To test (requires authentication):"
echo "  gcloud run services proxy $SERVICE_NAME --region $REGION --project $PROJECT_ID"
echo ""
echo "To view logs:"
echo "  gcloud run services logs tail $SERVICE_NAME --region $REGION --project $PROJECT_ID"
```

Save this as `deploy-cloudrun.sh`, make it executable, and run:

```bash
chmod +x deploy-cloudrun.sh
./deploy-cloudrun.sh
```

## Troubleshooting

### Container fails to start

**Check logs:**
```bash
gcloud run services logs read harmony-proxy --region us-central1 --limit 50
```

**Common causes:**
- Missing or invalid configuration file
- Incorrect port configuration (must be 8080 or match `PORT` env var)
- Missing required secrets/environment variables
- Insufficient memory allocation

### Configuration not loading

**Verify secret mount:**
```bash
# Check secret exists
gcloud secrets describe harmony-config

# Verify service account has access
gcloud secrets get-iam-policy harmony-config

# Check revision for secret mounts
gcloud run revisions describe REVISION_NAME --region us-central1
```

### High latency / cold starts

**Solutions:**
- Set `--min-instances 1` or higher to keep instances warm
- Increase memory allocation
- Use startup CPU boost: `--cpu-boost` (preview feature)
- Optimize configuration loading

### Out of memory errors

**Increase memory allocation:**
```bash
gcloud run services update harmony-proxy \
  --memory 2Gi \
  --region us-central1
```

### VPC connectivity issues

**Verify VPC connector:**
```bash
gcloud compute networks vpc-access connectors describe harmony-connector \
  --region us-central1

# Check service configuration
gcloud run services describe harmony-proxy \
  --region us-central1 \
  --format="value(spec.template.spec.containers[0].resources)"
```

### Permission denied errors

**Check service account permissions:**
```bash
# List IAM bindings for secret
gcloud secrets get-iam-policy harmony-config

# Grant access if needed
gcloud secrets add-iam-policy-binding harmony-config \
  --member="serviceAccount:SERVICE_ACCOUNT@PROJECT_ID.iam.gserviceaccount.com" \
  --role="roles/secretmanager.secretAccessor"
```

## Best Practices

1. **Use Secret Manager** for configuration and sensitive data
2. **Set appropriate resource limits** based on load testing
3. **Use custom service accounts** with least-privilege IAM
4. **Enable request logging** for audit trails
5. **Configure health checks** via management API
6. **Use min-instances** for latency-sensitive workloads
7. **Set up monitoring alerts** for errors and performance
8. **Use VPC connectors** for private resource access
9. **Deploy to multiple regions** for high availability
10. **Version your configuration** in Secret Manager

## Cost Optimization

- **Scale to zero** (`--min-instances 0`) for dev/test environments
- **Right-size resources** - don't over-provision memory/CPU
- **Use execution log retention policies** to control storage costs
- **Set appropriate timeouts** to avoid hanging requests
- **Use committed use discounts** for predictable workloads
- **Monitor and optimize cold start times** to reduce billable time

## Next Steps

- Review [Configuration Documentation](../configuration.md) for advanced options
- See [Security Documentation](../security.md) for production hardening
- Explore [Management API](../management-api.md) for operational endpoints
- Check [Monitoring Guide](../monitoring.md) for observability setup (if available)

## Additional Resources

- [Google Cloud Run Documentation](https://cloud.google.com/run/docs)
- [Secret Manager Documentation](https://cloud.google.com/secret-manager/docs)
- [Cloud Run Pricing](https://cloud.google.com/run/pricing)
- [Harmony Proxy Documentation](../../readme.md)

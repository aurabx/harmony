---
title: Google Cloud Run: Production
description: Monitoring, a complete production example, troubleshooting, and cost guidance for Cloud Run.
status: active
updated: 2026-10-24
type: Guide
related:
  - guides/deployment-google-cloud-run.md
---

# Google Cloud Run: Production

Monitoring, a complete production example, troubleshooting, and cost guidance for Cloud Run. This page is part of [Deploying Harmony Proxy to Google Cloud Run](deployment-google-cloud-run.md).

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

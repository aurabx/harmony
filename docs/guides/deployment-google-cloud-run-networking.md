---
title: Google Cloud Run: Networking and IAM
description: How to set up VPC networking, authentication, and IAM for Harmony on Cloud Run.
status: active
updated: 2026-10-24
type: Guide
related:
  - guides/deployment-google-cloud-run.md
---

# Google Cloud Run: Networking and IAM

How to set up VPC networking, authentication, and IAM for Harmony on Cloud Run. This page is part of [Deploying Harmony Proxy to Google Cloud Run](deployment-google-cloud-run.md).

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

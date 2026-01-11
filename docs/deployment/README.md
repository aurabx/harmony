# Deployment Guides

This directory contains deployment guides for running Harmony Proxy on various platforms.

## Available Guides

### Cloud Platforms

- **[Google Cloud Run](google-cloud-run.md)** - Deploy Harmony as a fully managed serverless container on Google Cloud Platform
  - Quick start with pre-built images
  - Configuration management options (Secret Manager, Cloud Storage)
  - Production deployment scripts
  - VPC networking and IAM setup
  - Monitoring and troubleshooting

### Coming Soon

- **AWS ECS/Fargate** - Deploy on Amazon's container orchestration services
- **Azure Container Instances** - Deploy on Microsoft Azure
- **Kubernetes** - Deploy to any Kubernetes cluster (GKE, EKS, AKS, self-hosted)
- **Docker Compose** - Multi-container local or VM deployments
- **Systemd Service** - Run as a native Linux service
- **Fly.io** - Deploy on Fly.io's edge platform

## Quick Reference

### Docker Images

Pre-built images are available from GitHub Container Registry:

```bash
# Latest stable
docker pull ghcr.io/aurabx/harmony:latest

# Specific version
docker pull ghcr.io/aurabx/harmony:v0.4.0
```

### Basic Docker Deployment

See the main [README.md](../../readme.md#docker) for Docker and Docker Compose instructions.

### Build from Source

See [Getting Started](../getting-started.md) for building from source.

## Platform Selection Guide

| Platform | Best For | Pros | Cons |
|----------|----------|------|------|
| **Cloud Run** | Serverless, auto-scaling | Fully managed, pay-per-use, zero ops | GCP-specific, stateless |
| **ECS/Fargate** | AWS ecosystem | AWS integration, managed | AWS-specific, more complex |
| **Kubernetes** | Enterprise, multi-cloud | Portable, full control, advanced features | Complex, requires k8s expertise |
| **Docker Compose** | Development, simple production | Easy setup, local-friendly | Manual scaling, single-host |
| **Systemd** | Traditional VMs, dedicated servers | Simple, no container overhead | Manual management, less portable |
| **Fly.io** | Edge computing, global distribution | Multi-region, simple deployment | Smaller ecosystem |

## General Deployment Considerations

### Environment Variables

All platforms require these environment variables:

- `RUST_LOG` - Logging level (default: `info`)
- `RUNBEAM_ENCRYPTION_KEY` - Encryption key for token storage (required in production)
- `RUNBEAM_JWT_SECRET` - JWT secret for Runbeam Cloud integration (if enabled)

See [Security Documentation](../security.md#environment-variables) for details.

### Configuration File

Harmony requires a TOML configuration file. Common options:

1. **Mount as volume** - Most flexible, easy updates
2. **Embed in image** - Simplest, requires rebuild for changes
3. **Secret manager** - Most secure, platform-specific

See [Configuration Documentation](../configuration.md) for structure and options.

### Persistent Storage

Harmony stores temporary files and token cache. Options:

- **Ephemeral** (`/tmp`) - Good for stateless deployments with `RUNBEAM_ENCRYPTION_KEY`
- **Volume mounts** - Persistent storage across restarts
- **Cloud storage** - Managed, scalable storage

### Networking

Consider these networking aspects:

- **Bind address** - Use `0.0.0.0` for container deployments
- **Port** - Default 8080 (configurable)
- **VPC/Private networking** - For backend connectivity
- **TLS termination** - Usually at load balancer/ingress

### Health Checks

Enable the management API for health checks:

```toml
[management]
enabled = true
base_path = "admin"
network = "external"
```

Health endpoint: `GET /admin/health`

## Security Best Practices

1. **Never commit secrets** - Use environment variables or secret managers
2. **Use encryption keys** - Set `RUNBEAM_ENCRYPTION_KEY` in production
3. **Restrict network access** - Use VPC/firewall rules
4. **Enable authentication** - Require IAM/auth for sensitive endpoints
5. **Use custom service accounts** - Follow principle of least privilege
6. **Enable audit logging** - Track access and changes
7. **Keep images updated** - Regularly update to latest version

See [Security Documentation](../security.md) for comprehensive guidance.

## Monitoring and Observability

Key metrics to monitor:

- Request rate and latency
- Error rates (4xx, 5xx)
- CPU and memory utilization
- Instance count (for auto-scaling platforms)
- Backend connectivity

Harmony supports structured logging compatible with:

- Google Cloud Logging
- AWS CloudWatch
- Azure Monitor
- ELK/EFK Stack
- Splunk
- Datadog, New Relic, etc.

## Contributing

Have experience deploying Harmony on other platforms? We welcome contributions!

Please see [CONTRIBUTING.md](../../CONTRIBUTING.md) for guidelines on submitting deployment guides.

## Support

- Documentation: [docs/](../)
- Issues: [GitHub Issues](https://github.com/aurabx/harmony/issues)
- Email: hello@aurabox.cloud

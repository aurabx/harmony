<div align="center">
  <h1>Harmony Proxy</h1>
  <p>
    A secure, pluggable proxy for data meshes — with first-class healthcare support (FHIR, DICOM/DICOMweb, JMIX).
  </p>
  <p>
    <a href="https://github.com/aurabx/harmony/actions/workflows/rust.yml">
      <img alt="Rust CI" src="https://github.com/aurabx/harmony/actions/workflows/rust.yml/badge.svg" />
    </a>
  </p>
</div>

## Overview

Harmony Proxy is a production-ready, extensible data mesh proxy/gateway for heterogeneous systems. It routes requests through configurable endpoints, middleware, and services/backends to connect systems that speak HTTP/JSON, FHIR, DICOM/DICOMweb, and JMIX.

Highlights:
- Multi-protocol: HTTP/JSON passthrough, FHIR, DICOM/DICOMweb (QIDO-RS/WADO-RS), JMIX
- Configurable pipelines: endpoints + ordered middleware + services/backends
- Authentication: JWT (recommend RS256 in production), optional Basic
- Transformations: JSON transforms (JOLT), DICOM↔DICOMweb bridging, JMIX packaging
- Runbeam Cloud Integration: Gateway authorization for autonomous API access with 30-day machine tokens
- Operationally sound: structured logging, local ./tmp storage convention, file-system storage backend

Status: under active development. For more information, visit https://harmonyproxy.com.

## Who is this for?
- Platform teams building data meshes or integration hubs (healthcare and beyond)
- Developers integrating HTTP/JSON services and healthcare protocols (FHIR, DICOM/DICOMweb)
- Operators who need auditable, configurable request/response pipelines

## Quick start

### Local development

Prerequisites:
- Rust (stable) via rustup
- macOS or Linux

Build and run with the example configuration:

```bash
# Build
cargo build

# Run the basic echo example
cargo run -- --config examples/basic-echo/config.toml
```

Try the basic echo endpoint:

```bash
# In another shell
curl -i http://127.0.0.1:8080/echo
```

If configured, you should receive an echoed response from the sample backend. Explore more examples under the `examples/` directory (each has its own README).

### Docker

#### Option 1 – Run with Docker Compose (recommended)

For local development or quick testing:

```bash
# Build and start containers
docker compose up

# Alternatively, to rebuild and restart cleanly
docker compose up --build --force-recreate -d

# Test the service
curl -i http://localhost:8080/echo
```

Compose uses the included `Dockerfile.build` so everything builds from source inside Docker—no Rust toolchain required on the host.

**Ports**

* **8080** – Main service endpoints
* **9090** – Management API (if enabled)

---

#### Option 2 – Build and run manually (from prebuilt or local binaries)

If you have prebuilt binaries or are running from CI output, use the lean runtime image (`Dockerfile`):

```bash
# Build image from prebuilt binaries (fast path)
docker build -t harmony-proxy .

# Run with default config
docker run -p 8080:8080 -p 9090:9090 \
  -v $(pwd)/config:/etc/harmony:ro \
  harmony-proxy

# Run with example config
docker run -p 8080:8080 \
  -v $(pwd)/examples:/examples:ro \
  harmony-proxy --config /examples/basic-echo/config.toml
```

If you’d rather build everything from scratch (no prebuilt binaries), specify the full build image explicitly:

```bash
docker build -f Dockerfile.build -t harmony-proxy .
docker run -p 8080:8080 harmony-proxy
```

---

#### Option 3 – Use the published image

Once your CI workflow pushes to GHCR:

```bash
docker pull ghcr.io/aurabx/harmony:latest
docker run -p 8080:8080 -p 9090:9090 ghcr.io/aurabx/harmony:latest
```

---

This layout clarifies:

* **Compose / Dockerfile.build** → full source build (developer-friendly)
* **Dockerfile** → prebuilt-binary runtime (used by CI and GHCR images)
* **Published image** → fastest start for end-users


## Configuration
Harmony's configuration is file-based (TOML) and can include additional pipeline/transform files from a directory.

Examples (each with README, config, and pipelines):
- `examples/basic-echo/` - Simple HTTP passthrough
- `examples/fhir/` - FHIR with authentication
- `examples/transform/` - JOLT transformations
- `examples/fhir-to-dicom/` - Protocol translation
- `examples/jmix/` - JMIX packaging
- `examples/dicom-backend/` - DICOM SCU operations
- `examples/dicom-scp/` - DICOM SCP endpoint
- `examples/dicomweb/` - DICOMweb support
- `examples/jmix-to-dicom/` - JMIX to DICOM workflow

Core building blocks:
- Networks: bind addresses/ports and optional WireGuard
- Endpoints: public-facing routes (HTTP/FHIR/DICOMweb)
- Middleware: ordered request/response modifiers (e.g., JWT auth, transforms)
- Services/Backends: where work is performed (e.g., DICOMweb client, echo service)
- Storage: project-local filesystem path (./tmp by default)

See [docs/configuration.md](docs/configuration.md), [docs/endpoints.md](docs/endpoints.md), [docs/middleware.md](docs/middleware.md), and [docs/backends.md](docs/backends.md) for details.

## Documentation
- Docs index: [docs/README.md](docs/README.md)
- Getting started: [docs/getting-started.md](docs/getting-started.md)
- Configuration: [docs/configuration.md](docs/configuration.md)
- Endpoints: [docs/endpoints.md](docs/endpoints.md)
- Middleware: [docs/middleware.md](docs/middleware.md)
- Backends: [docs/backends.md](docs/backends.md)
- Router: [docs/router.md](docs/router.md)
- Envelope model: [docs/envelope.md](docs/envelope.md)
- Management API: [docs/management-api.md](docs/management-api.md) (includes Runbeam Cloud authorization)
- DIMSE integration: [docs/dimse-integration.md](docs/dimse-integration.md)
- Testing: [docs/testing.md](docs/testing.md)
- Security: [docs/security.md](docs/security.md)
- System description: [docs/system-description.md](docs/system-description.md)

## System requirements
- Rust (stable)
- macOS or Linux runtime environment
- DCMTK (required if you use DICOM DIMSE features)
  - macOS (Homebrew): `brew install dcmtk`
  - Debian/Ubuntu: `sudo apt-get install dcmtk`

## Security
- JWT: prefer RS256 with strict algorithm enforcement; validate exp/nbf/iat and iss/aud where applicable
- Encryption: where applicable, AES-256-GCM with ephemeral public key, IV, and authentication tag encoded in base64
- Secrets: do not commit secrets; use environment variables or secret managers
- Temporary files: prefer ./tmp within the working directory
See docs/security.md for guidance.

## Contributing
We welcome issues and pull requests! Please read [CONTRIBUTING.md](CONTRIBUTING.md) and [DEVELOPER.md](DEVELOPER.md) for workflow and development standards. Our community guidelines are defined in [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License and commercial use
Harmony Proxy is licensed under the Apache License, Version 2.0.

Important: You may freely download, use, and modify Harmony Proxy for internal use and self-hosted deployments. Reselling Harmony Proxy as a hosted service or embedding it in a commercial offering requires a commercial licence from Aurabox Pty Ltd. Contact support@aurabox.cloud for enquiries.

## Support
- General questions and support: hello@aurabox.cloud
- Security or conduct concerns: hello@aurabox.cloud

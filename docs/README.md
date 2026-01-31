# Documentation Index

**Last Updated**: 2025-01-31

Welcome to Harmony Proxy's documentation. Harmony is a general-purpose data mesh proxy with first-class support for medical data (FHIR, DICOM/DICOMweb, JMIX). Start here to explore concepts, configuration, and usage.

## Getting Started
- **[getting-started.md](getting-started.md)** - Build, run, and first steps with environment variables
- **[configuration.md](configuration.md)** - Configuration structure, hot reload, and validation
- **[testing.md](testing.md)** - Testing strategy, running tests, and verifying examples

## Core Architecture & Concepts
- **[system-description.md](system-description.md)** - High-level system overview and Runbeam architecture
- **[router.md](router.md)** - Pipeline architecture and request flow (Protocol Adapter → PipelineExecutor → Protocol Adapter)
- **[adapters.md](adapters.md)** - Protocol adapter guide (HTTP, HTTP/3, DIMSE, how to implement new protocols)
- **[extensions.md](extensions.md)** - Extension system for custom services, middleware, and plugins
- **[envelope.md](envelope.md)** - Core Envelope struct for data exchange

## Configuration Reference
- **[schema.md](schema.md)** - Configuration schemas (mesh, config, pipeline, remote-ingress)
- **[mesh.md](mesh.md)** - Data mesh networking and ingress/egress definitions
- **[providers.md](providers.md)** - Provider configuration for resource resolution and cloud polling
- **[resource-references.md](resource-references.md)** - Reference syntax for cross-provider resources
- **[endpoints.md](endpoints.md)** - Endpoint types (HTTP, FHIR, JMIX, DICOMweb)
- **[backends.md](backends.md)** - Backend types and target communication
- **[middleware.md](middleware.md)** - Middleware reference (authentication, transforms, path filtering)
- **[content-types.md](content-types.md)** - Multi-content-type support (JSON, XML, CSV, form data, multipart, binary)

## Healthcare Protocols
- **[dimse-integration.md](dimse-integration.md)** - DICOM DIMSE SCU/SCP operations (requires DCMTK)
- **[fhir_imagingstudy.md](fhir_imagingstudy.md)** - FHIR ImagingStudy resource integration

## Advanced Features
- **[config-reload.md](config-reload.md)** - Hot configuration reload with zero-downtime updates
- **[management-api.md](management-api.md)** - Management API for monitoring and administration
- **[encryption-key-management.md](encryption-key-management.md)** - Encryption key management and token storage
- **[security.md](security.md)** - Security best practices and environment variables
- **[policies-middleware.md](policies-middleware.md)** - Policy-based middleware configuration
- **[policy-rules-reference.md](policy-rules-reference.md)** - Policy rules reference
- **[transforms.md](Transforms)** - How to use the Jolt transform syntax

## Deployment
- **[deployment/README.md](deployment/README.md)** - Deployment overview
- **[deployment/google-cloud-run.md](deployment/google-cloud-run.md)** - Google Cloud Run deployment

## Architecture
- **[architecture/diagrams.md](architecture/diagrams.md)** - System architecture diagrams

## Quick Links
- **Example configurations**: See `examples/` directory in repository root
  - `examples/basic-echo/` - Simple HTTP passthrough
  - `examples/fhir/` - FHIR with authentication
  - `examples/transform/` - JOLT transformations
  - `examples/fhir-to-dicom/` - Protocol translation
  - `examples/jmix/` - JMIX packaging
  - `examples/dicom-backend/` - DICOM SCU operations
  - `examples/dicom-scp/` - DICOM SCP endpoint
  - `examples/dicomweb/` - DICOMweb support
  - `examples/jmix-to-dicom/` - JMIX to DICOM workflow

## Common Tasks
**Need help getting started?** Start with [getting-started.md](getting-started.md)

**Building a pipeline?** Read [configuration.md](configuration.md) and [router.md](router.md)

**Working with healthcare data?** See [dimse-integration.md](dimse-integration.md) and [fhir_imagingstudy.md](fhir_imagingstudy.md)

**Deploying to production?** Check [security.md](security.md) and [deployment/](deployment/)

**Testing your setup?** Use [testing.md](testing.md)

## Development Conventions
- **Temporary files**: Use `./tmp` within the working directory (not system `/tmp`)
- **Secrets**: Never commit secrets; load via environment variables or secret managers (see [security.md](security.md))
- **Code blocks in examples**: Use text blocks to avoid compilation issues
- **All cross-references**: Use relative paths (e.g., `[file.md](file.md)`)

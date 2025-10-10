# Documentation Index

Welcome to Harmony Proxy’s documentation. Harmony is a general-purpose data mesh proxy with first-class support for medical data (FHIR, DICOM/DICOMweb, JMIX). Start here to explore concepts, configuration, and usage.

- Getting started: getting-started.md
- Configuration: configuration.md
- Endpoints: endpoints.md
- Middleware: middleware.md
- Backends: backends.md
- Router: router.md
- Envelope: envelope.md
- DIMSE integration (DICOM SCU/SCP): dimse-integration.md
- Testing (including DCMTK verbosity): testing.md
- Security: security.md
- System description: system-description.md

Quick links:
- Example configuration (default): examples/default/config.toml (see repo root)
- Example pipelines: examples/default/pipelines/

Conventions:
- Temporary files: prefer ./tmp within the working directory
- Secrets: avoid committing; load via environment variables or secret managers
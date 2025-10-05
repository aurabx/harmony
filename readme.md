# Harmony Proxy

This project is alpha quality and under active development.

Overview
- Harmony is a proxy/gateway for healthcare systems. It routes requests through endpoints, middleware, and backends, with support for FHIR, JMIX, DICOM/DICOMweb, and JWT-based auth.

Quick start
- Build: cargo build
- Run (example config): cargo run -- --config examples/default/config.toml
- Test: cargo test

Documentation
- Getting started: docs/getting-started.md
- Configuration: docs/configuration.md
- Middleware (JWT, Basic): docs/middleware.md
- Testing: docs/testing.md
- Security: docs/security.md
- Architecture overview: docs/system-description.md
- Router: docs/router.md

Notes
- Prefer using ./tmp for temporary files within the working directory
- See the examples/default directory for a working configuration layout

Licence and Use
Harmony Proxy is licensed under the Apache License, Version 2.0.

Important: You may freely download, use, and modify Harmony Proxy for internal use and self-hosted deployments. Reselling Harmony Proxy as a hosted service or embedding it in a commercial offering requires a commercial licence from Aurabox Pty Ltd. Contact support@aurabox.cloud for enquiries.

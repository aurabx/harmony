# Harmony Proxy

This project is alpha quality and under active development.

## Overview
- Harmony is a proxy/gateway for secure data meshes. It routes requests through endpoints, middleware, and backends, with support for FHIR, JMIX, DICOM/DICOMweb, and JWT-based auth.

## Quick start
- Build: cargo build
- Run (example config): cargo run -- --config examples/default/config.toml
- Test: cargo test

## Documentation
- Getting started: docs/getting-started.md
- Configuration: docs/configuration.md
- Middleware (JWT, Basic): docs/middleware.md
- Testing: docs/testing.md
- Security: docs/security.md
- Architecture overview: docs/system-description.md
- Router: docs/router.md

## Notes
- Prefer using ./tmp for temporary files within the working directory
- See the examples/default directory for a working configuration layout

## Test artifacts in ./tmp

- This repository writes test artifacts under `./tmp` instead of `/tmp`.
- For DICOM integration tests using DCMTK, you will typically see:
  - `./tmp/qrscp/dcmqrscp.cfg` — generated dcmqrscp configuration
  - `./tmp/qrscp/seed*.dcm` — seeded Part 10 files stored via storescu
  - `./tmp/dcmtk_find_<uuid>/rspXXXX.dcm` — extracted C-FIND responses (preserved)
  - `./tmp/dcmtk_get_<uuid>/*` — C-GET received objects (DCMTK may not use `.dcm` extension)
  - `./tmp/dcmtk_move_<uuid>/*` — C-MOVE received objects (DCMTK may not use `.dcm` extension)
  - `./tmp/movescu_last.json` — last movescu args/stdout/stderr and status for debugging

Cleanup helpers:

```bash
# Remove all DCMTK artifacts
rm -rf ./tmp/dcmtk_find_* ./tmp/dcmtk_get_* ./tmp/dcmtk_move_* ./tmp/movescu_last.json
```

Run focused tests:

```bash
cargo test --no-fail-fast --test dimse_scp_starts -- --nocapture
cargo test --no-fail-fast --test dicom_find_qrscp -- --nocapture
cargo test --no-fail-fast --test dicom_get_qrscp -- --nocapture
HARMONY_TEST_DEBUG=1 cargo test --no-fail-fast --test dicom_move_qrscp -- --nocapture
```

## Licence and Use
Harmony Proxy is licensed under the Apache License, Version 2.0.

Important: You may freely download, use, and modify Harmony Proxy for internal use and self-hosted deployments. Reselling Harmony Proxy as a hosted service or embedding it in a commercial offering requires a commercial licence from Aurabox Pty Ltd. Contact support@aurabox.cloud for enquiries.

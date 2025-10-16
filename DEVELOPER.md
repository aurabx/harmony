# DEVELOPER.md

## Overview
- Harmony Proxy is a Rust workspace providing a configurable data mesh proxy/gateway for heterogeneous systems, with first-class healthcare protocols (FHIR, JMIX, DICOM/DICOMweb). It is part of the Runbeam ecosystem.
- Key concepts: endpoints, middleware, services/backends, pipelines, and storage. See [docs/README.md](docs/README.md) for full architecture.

## Prerequisites
- Rust (stable) via rustup
- macOS or Linux
- Optional: DCMTK tools for DIMSE integration tests (e.g., macOS: `brew install dcmtk`, Debian/Ubuntu: `sudo apt-get install dcmtk`)

## Repository layout
- Root crate: harmony (src/lib.rs, src/main.rs)
- Workspace crates: crates/dimse, crates/dicom_json_tool, crates/transform
- Examples: examples/config (config.toml, pipelines/, transforms/), examples/custom_endpoint (example crate)
- Docs: docs/* (getting-started, configuration, endpoints, middleware, backends, router, envelope, dimse-integration, testing, security, system-description)
- Samples: samples/jolt/*; additional DICOM samples in dev/samples
- Temporary outputs: prefer ./tmp (gitignored) over /tmp

## Build, run, test
- Build (debug): `cargo build`
- Build (release): `cargo build --release`
- Run with example config: `cargo run -- --config examples/config/config.toml`
- Run tests: `cargo test`
- With logs: `RUST_LOG=harmony=debug cargo test -- --nocapture`
- Format: `cargo fmt --all`
- Lint: `cargo clippy --all-targets -- -D warnings`

## Configuration model (high level)
- Config is loaded from the provided TOML file and can merge additional files from proxy.pipelines_path and proxy.transforms_path relative to the base config directory.
- Major sections: proxy, network, pipelines, endpoints, backends, middleware/middleware_types, services, targets, storage, transforms.
- Validation enforces: proxy ID and log level; per-network interface/HTTP constraints; pipeline endpoints exist; middleware_types are known or specified via module; targets have non-empty URLs; storage backend options are valid.
- See: src/config/config.rs and [docs/configuration.md](docs/configuration.md).

## Pipelines and middleware
- Pipelines bind networks to endpoints and an ordered set of middleware/services.
- Built-in middleware types include: jwtauth, auth, connect, passthru, json_extractor, jmix_builder, dicomweb_to_dicom, dicomweb, transform.
- JWT guidance: prefer RS256 in production, enforce algorithm, validate exp/nbf/iat, iss/aud as applicable. HS256 is for development/tests only.

## JMIX and schema path
- The JMIX schema directory used by validation or transforms is configurable and may be placed outside the repo (e.g., ../jmix). Ensure paths in config reference your chosen location.

## Storage and tmp
- Filesystem storage defaults to a project-local ./tmp directory. Favor ./tmp for all temporary files. The path may be configurable via storage options.

## Testing notes
- Strategy: unit/integration tests should be fast and deterministic. See [docs/testing.md](docs/testing.md).
- DIMSE integration: DCMTK processes may be spawned; artifacts are written under ./tmp with per-test UUIDs. Enable verbose DIMSE output with `HARMONY_TEST_VERBOSE_DCMTK=1`. Additional debugging: `HARMONY_TEST_DEBUG=1`.
- Sample data: samples/ and dev/samples/ contain example inputs.

## Security considerations
- Encryption: the project uses AES-256-GCM where applicable, with ephemeral public key, IV, and authentication tag encoded in base64.
- Credentials and secrets: do not commit. Load via environment variables or secret managers. Prefer least-privilege file permissions.
- JWT: in production, require RS256 with strict algorithm checks and claim validation.
- Temporary files: prefer ./tmp within the working directory.
- See [docs/security.md](docs/security.md) for more detail.

## Commit and PR hygiene
- Use Conventional Commits for messages (e.g., feat:, fix:, docs:, refactor:, test:, build:, ci:, chore:, perf:, style:, revert:). Keep PRs focused; avoid mixing unrelated changes.
- Trunk-based development: open feature branches targeting main.
- Tests, lint, and fmt should pass locally before opening a PR.

## Quick links
- README: [README.md](README.md)
- Docs index: [docs/README.md](docs/README.md)
- Getting started: [docs/getting-started.md](docs/getting-started.md)
- Configuration: [docs/configuration.md](docs/configuration.md)
- Middleware: [docs/middleware.md](docs/middleware.md)
- Security: [docs/security.md](docs/security.md)
- Testing: [docs/testing.md](docs/testing.md)
- Router: [docs/router.md](docs/router.md)

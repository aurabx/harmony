# Contributing to Harmony Proxy

Thank you for your interest in contributing! This document explains how to propose changes, the development workflow, coding standards, and community expectations.

## Code of Conduct
- By participating, you agree to abide by our Code of Conduct. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Scope of contributions
- We welcome bug reports, feature requests, documentation improvements, performance optimizations, and test coverage enhancements.

## Getting started
- Read [DEVELOPER.md](DEVELOPER.md) for local setup, repository layout, commands, and testing guidance.
- Review the docs index at [docs/README.md](docs/README.md) for architecture, configuration, middleware, and testing references.

## Workflow
- Trunk-based development: use feature branches and open pull requests against main.
- Keep PRs small and focused. Avoid mixing unrelated refactors, formatting, or lint changes with functional changes.
- Link issues in PR descriptions when applicable. Describe the problem, approach, and trade-offs.

## Commit messages and PR titles
- Use Conventional Commits and prefer aligning PR titles similarly:
  - feat: add DIMSE store-and-forward support
  - fix: handle JWKS rotation error on startup
  - docs: update configuration examples
  - refactor: extract DICOMweb client
  - test: add pipeline validation cases
  - perf: reduce clone in router
  - ci: enable tests in GitHub Actions
- Follow Semantic Versioning. Breaking changes should be marked with ! (e.g., feat!: change endpoint config).

## Coding standards and gates
- Format code: `cargo fmt --all`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Build: `cargo build`
- Test: `cargo test` (add `RUST_LOG=harmony=debug` and `-- --nocapture` for verbose output)
- These gates must pass locally before opening a PR; CI will run them as well.

## Tests
- Prefer focused and deterministic unit/integration tests.
- Use ./tmp for temporary artifacts; do not rely on /tmp.
- DIMSE/DCMTK integration tests:
  - Default runs are quiet. Enable verbose logs with `HARMONY_TEST_VERBOSE_DCMTK=1`.
  - Additional debug behavior: `HARMONY_TEST_DEBUG=1`.
- Use the samples/ directory for test inputs where relevant; keep tests hermetic.

## Documentation
- Update docs/ and example directories (examples/*/) when behavior or configuration changes.
- Each example should have its own README with usage notes and prerequisites.
- Include security implications where applicable.

## Security guidance
- Do not commit secrets. Use environment variables or secret managers; restrict permissions to least privilege.
- JWT in production should use RS256 with strict algorithm enforcement and claim validation (exp, nbf, iat, iss, aud).
- Encryption: where used, AES-256-GCM with ephemeral public key, IV, and authentication tag encoded in base64.
- Temporary files: prefer ./tmp within the working directory.
- See [docs/security.md](docs/security.md) for additional guidance.

## Licensing of contributions
- By contributing, you agree your contributions are licensed under the Apache License, Version 2.0, and acknowledge the project’s commercialization restriction described in [README.md](README.md) (hosted resale/embedding requires a commercial licence from Aurabox Pty Ltd).
- No CLA or DCO is presently required.

## Issue triage and labels
- Use labels such as bug, enhancement, documentation, help wanted, and good first issue where applicable.

## Contact
- General questions, security or conduct concerns: hello@aurabox.cloud
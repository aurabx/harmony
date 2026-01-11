# Testing

**Last Updated**: 2025-11-30

## Strategy
- Unit and integration tests should be fast and deterministic
- Avoid spawning real HTTP listeners in tests
- Build the network router and use Router::oneshot for request handling in tests
- Documentation examples should use text blocks (with path/start metadata) to avoid compilation issues

## Running Tests

Commands
- Run all tests: cargo test
- Focused test: cargo test <name>
- With logging: RUST_LOG=harmony=debug cargo test -- --nocapture

## Installing Development Build to PATH

For testing the built binary in your PATH (useful for integration testing and manual verification):

```bash
# Build and install to ~/.local/bin (release mode)
./install-dev.sh

# Build and install with profiling symbols
./install-dev.sh --profile profiling
```

The script:
- Builds harmony using the specified profile (default: release)
- Installs the binary to `~/.local/bin/harmony`
- Verifies the installation and warns if PATH issues are detected
- Makes the binary executable

This replaces any existing harmony binary in your PATH, allowing you to test the current development version system-wide.

Environment variables (tests)
- HARMONY_TEST_VERBOSE_DCMTK=1: Enable verbose DCMTK logs in DIMSE-related integration tests (show child stdout/stderr and add `-d` to dcmqrscp). Default is quiet.
- HARMONY_TEST_DEBUG=1: Enable additional debug behavior in some tests (e.g., attach movescu args/stdout/stderr to responses).

Examples
```bash
# Quiet (default)
cargo test -- --nocapture

# DCMTK verbose logs for tests that spawn dcmqrscp/storescu
HARMONY_TEST_VERBOSE_DCMTK=1 cargo test -- --nocapture

# Combine with additional debug behavior
HARMONY_TEST_VERBOSE_DCMTK=1 HARMONY_TEST_DEBUG=1 cargo test -- --nocapture
```

Notes
- Prefer fixture configs under examples/default/pipelines or tests/data
- For JWT tests, explicitly choose RS256 or HS256 mode and sign tokens accordingly
- Consider adding end-to-end tests against a full server only in separate, slower suites
- JMIX (dev): Development testing documentation for JMIX API is available in the project's dev directory

## Mocking External Services

### Mock DICOM Backend
For testing DICOM pipelines without a real PACS, use the `mock_dicom` backend service. It simulates a PACS that responds to C-FIND, C-GET, and C-ECHO requests with built-in sample data.

See [Backends - Mock DICOM](backends.md#mock-dicom) for configuration details.

## Verifying Examples

It is recommended to run the provided examples to ensure end-to-end functionality in a real environment.

```bash
# Run all examples interactively
./scripts/run-example.sh

# Run specific example directly
./scripts/run-example.sh http-backend
```

This script builds the project and runs the selected example's `demo.sh`, which typically starts the proxy and performs integration tests against it.

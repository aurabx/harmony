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

## Verifying Examples

It is recommended to run the provided examples to ensure end-to-end functionality in a real environment.

```bash
# Run all examples interactively
./scripts/run-example.sh

# Run specific example directly
./scripts/run-example.sh http-backend
```

This script builds the project and runs the selected example's `demo.sh`, which typically starts the proxy and performs integration tests against it.

# Testing

Strategy
- Unit and integration tests should be fast and deterministic
- Avoid spawning real HTTP listeners in tests
- Build the network router and use Router::oneshot for request handling in tests

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
- JMIX (dev): See [jmix-dev-testing.md](../dev/jmix-dev-testing.md) for JMIX API development testing

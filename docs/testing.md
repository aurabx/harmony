# Testing

Strategy
- Unit and integration tests should be fast and deterministic
- Avoid spawning real HTTP listeners in tests
- Build the network router and use Router::oneshot for request handling in tests

Commands
- Run all tests: cargo test
- Focused test: cargo test <name>
- With logging: RUST_LOG=harmony=debug cargo test -- --nocapture

Notes
- Prefer fixture configs under examples/default/pipelines or tests/data
- For JWT tests, explicitly choose RS256 or HS256 mode and sign tokens accordingly
- Consider adding end-to-end tests against a full server only in separate, slower suites
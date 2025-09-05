Project: Harmony Proxy (Rust, Cargo)

This document captures project-specific development notes to speed up onboarding and reduce friction for day-to-day work.

Build and Configuration
- Toolchain: Rust 1.87 (edition 2021). Tokio is used with full feature set; axum 0.8.x; tower 0.5.x.
- Crate layout: library crate `harmony` (src/lib.rs) with binaries under src/bin (main service), plus an `examples/custom_endpoint` example crate.
- Dependencies relevant to runtime: axum-server for HTTP, tracing + tracing-subscriber for logs, libloading for dynamic loading of custom endpoints/middleware, jsonwebtoken + base64 for JWT processing.
- Features: No Cargo features are defined at present. Build profiles are default.
- Build commands:
  - Debug: cargo build
  - Release: cargo build --release
  - Run main service: cargo run -- --config examples/test-config.toml
  - Run with alternate config: cargo run -- --config <path/to/config.toml>
- Configuration model:
  - Top-level Config comprises: proxy, network (HashMap<String, NetworkConfig>), groups, endpoints, backends, middleware, logging.
  - Validation pipeline: Config::validate() enforces:
    - Proxy: ProxyConfig::validate() (see src/config/config.rs).
    - Network presence and HTTP sub-config validity (bind_address, bind_port; wireguard interface required if enable_wireguard=true).
    - Groups presence and network references.
    - Cross-references: groups -> endpoints/backends/middleware. DICOM-specific endpoint/backend validation hooks in src/backends/dicom validate_*.
  - Examples for configuration live in examples/ (incoming.toml, outgoing.toml, test-config.toml). Prefer using these when iterating locally.

Testing
- Test harness: cargo test. No external services are required; all config validation tests use in-memory TOML strings and the serde+toml parser.
- Current tests: tests/config_validation.rs validates the configuration validator behavior. These tests currently fail on main at the time of this writing because the test fixtures do not include required groups/endpoints/middleware references mandated by the current validator. Run them to see current failures.
  - Run full suite: cargo test
  - Run a focused test file: cargo test --test config_validation
  - Enable logs in tests: RUST_LOG=debug cargo test -- --nocapture
- Adding a new test:
  - Integration tests go into tests/*.rs and link against the library crate name `harmony`.
  - Keep tests hermetic; use TOML strings or files under tests/data if needed. Avoid relying on OS state.
  - Example minimal integration test (not checked in; for reference only). Note: Group requires a description string and a [groups.<name>.middleware] table (even if empty):
    use harmony::config::{Config, ConfigError};
    #[test]
    fn parse_minimal_network() {
        let toml = r#"
            [proxy]
            id = "demo"
            log_level = "info"
            store_dir = "/tmp"

            [network.default]
            enable_wireguard = false
            interface = "wg0"

            [network.default.http]
            bind_address = "127.0.0.1"
            bind_port = 8080

            [groups.core]
            networks = ["default"]
            endpoints = []
            backends = []
            [groups.core.middleware]
            incoming = []
            outgoing = []
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.validate().is_ok());
    }
- Updating/creating tests that involve groups/endpoints/middleware:
  - The validator requires at least one group and that each group declare networks. If a group lists middleware names like "jwt_auth", the corresponding middleware config must also be present at top-level [middleware.jwt_auth]. Unknown middleware names cause UnknownMiddleware error.
  - Endpoint/backend names referenced under a group must exist in [endpoints.*]/[backends.*], and DICOM-specific validators will run for type="dicom".

Developer Tips and Conventions
- Use Test Driven Development (TDD) when iterating.
- Code style: Use rustfmt (default) and clippy for linting. Suggested commands:
  - cargo fmt --all
  - cargo clippy --all-targets -- -D warnings
- Error handling: ConfigError is a central enum (src/config/mod.rs). When extending validation, add specific error variants as needed and update tests. Prefer structured errors over strings.
- Validation expectations:
  - Networks: must not be empty; each network must have http.bind_address and non-zero http.bind_port. If enable_wireguard=true, interface must be non-empty.
  - Groups: at least one group; each group must name at least one existing network; endpoint/backend names in a group must exist; middleware names must be recognized and configured if used.
- Adding middleware identifiers:
  - If you add a new middleware name that can appear in groups.middleware.{incoming,outgoing}, extend validate_middleware_references with match arm(s) and add the optional config to MiddlewareConfig, plus tests covering both presence and missing-config error cases.
- Running the service locally:
  - Use examples/test-config.toml as a baseline. Ensure it declares at least one group mapping to an existing network and references endpoints/backends that exist, or keep them empty with proper structure.
  - Binary accepts --config; default path is /etc/harmony/harmony-config.toml per Cli in src/config/mod.rs.
- Logging: tracing with env-filter is available. For local runs/tests: RUST_LOG=harmony=debug,info.
- Dynamic loading: libloading is included to support custom endpoints/middleware (see examples/custom_endpoint). When iterating there, build and run within that sub-crate; it is independent of the main crate.

Known Pitfalls
- The existing tests in tests/config_validation.rs may fail against current validator requirements. When modifying validator behavior, update these tests accordingly or adjust TOML fixtures to include groups and middleware configs as required.
- Unknown middleware names cause immediate failure; use only jwt_auth, auth_sidecar, aurabox_connect unless you also extend the configuration and validator.
- For WireGuard, network.interface must be set if enable_wireguard=true.

Reproducible Test Demo (what we ran locally)
- To run tests:
  - cargo test
- Observed (as of 2025-09-06): 4 failing tests in tests/config_validation.rs due to stricter current validation. Use this as a starting point when updating tests or validator behavior.

Housekeeping
- Keep examples/ TOML in sync with the validator rules to make cargo run work out-of-the-box.
- Prefer adding small, focused integration tests under tests/ instead of complex end-to-end fixtures.

Other documentation:
- /docs/router.md describes the router's behavior.
# Getting Started

Status: alpha-quality software under active development. Some features are placeholders.

Prerequisites
- Rust (stable; repository currently targets recent stable toolchains)
- macOS or Linux
- Optional: WireGuard kernel module if you plan to use WireGuard features

Build
- Debug: cargo build
- Release: cargo build --release

Run
- Using the default example configuration:
  - cargo run -- --config examples/default/config.toml
- The default config references pipeline files under examples/default/pipelines

Conventions
- Temporary files: prefer ./tmp within the working directory over /tmp
- Logging: use RUST_LOG=harmony=debug,info for local debugging

Next steps
- Read Configuration for how the top-level config and pipeline files fit together
- See Middleware for auth and transforms (including real JWT verification)
- See Testing for how to run fast, deterministic tests

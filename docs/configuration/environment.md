---
title: Environment Variables
description: Environment variables that Harmony reads at startup.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/configuration-file.md
---

# Environment Variables

Environment variables that Harmony reads at startup. This page is part of [Configuration](configuration-file.md).

## Environment Variables

Harmony supports several environment variables that affect runtime behavior. Most of these are **runtime settings** rather than configuration overrides - they don't replace TOML configuration but provide additional context for security, logging, and storage.

### Configuration-Affecting Variables

#### RUNBEAM_ENCRYPTION_KEY

**Purpose**: Provides encryption key for secure machine token storage (encrypted filesystem).

**Interaction with Configuration**:
- Does not override TOML configuration
- Affects how machine tokens are stored (see Management API authorization)
- Recommended for persistent token storage across restarts
- Typical in container environments

**When to Set**:
- Production container deployments (recommended)
- Headless/CI environments
- When persistent token storage is needed

**See**: [Security Documentation](../security/environment-variables.md#runbeam_encryption_key) for generation examples and best practices.

#### RUNBEAM_JWT_SECRET

**Purpose**: Shared secret for validating JWT tokens from Runbeam Cloud during gateway authorization.

**Interaction with Configuration**:
- Does not override TOML configuration
- Used by Management API `/authorize` endpoint
- Falls back to development default if not set (logs warning)

**When to Set**:
- Required for production Runbeam Cloud integration
- Must match secret configured in Runbeam Cloud

**See**: [Security Documentation](../security/environment-variables.md#runbeam_jwt_secret) for generation and rotation procedures.

#### RUST_LOG

**Purpose**: Controls logging verbosity via tracing filter directives.

**Interaction with Configuration**:
- **Overrides** `logging.log_level` setting in TOML configuration
- Environment variable takes precedence when both are set
- More flexible than TOML (supports per-module filtering)

**Common Values**:
```bash
# Override log level for all Harmony modules
export RUST_LOG=harmony=debug

# Per-module filtering (overrides TOML log_level)
export RUST_LOG=harmony::router=trace,harmony::middleware=debug,harmony=info

# Global debug (very verbose)
export RUST_LOG=debug
```

**Precedence**: `RUST_LOG` environment variable > `logging.log_level` in TOML.


### Environment Variable Precedence Rules

**Settings with Environment Variable Override**:
- **Logging**: `RUST_LOG` environment variable **overrides** `logging.log_level` in TOML

**Settings Without Override** (environment variables are supplemental):
- **Network configuration**: TOML only (bind addresses, ports, WireGuard settings)
- **Pipeline definitions**: TOML only (endpoints, backends, middleware chains)
- **Service configuration**: TOML only (service types, options)
- **Storage backend**: TOML only (filesystem path, backend type)
- **Token encryption**: `RUNBEAM_ENCRYPTION_KEY` supplements TOML (provides encryption key)
- **JWT validation**: `RUNBEAM_JWT_SECRET` supplements TOML (provides validation secret)

### Best Practices

**Development**:
```bash
# Override log level for detailed debugging
export RUST_LOG=harmony=debug,info

# Use TOML for all other configuration
cargo run -- --config examples/basic-echo/config.toml
```

**Production Containers**:
```bash
# Set security-sensitive values via environment
export RUNBEAM_ENCRYPTION_KEY=$(cat /run/secrets/encryption-key)
export RUNBEAM_JWT_SECRET=$(cat /run/secrets/jwt-secret)
export RUST_LOG=harmony=info

# Use TOML for application configuration
./harmony --config /etc/harmony/config.toml
```

**Why This Design?**
- **Security**: Secrets in environment variables (not committed to version control)
- **Configuration**: Application structure in TOML (version controlled, hot-reloadable)
- **Flexibility**: Log levels easily adjustable without config file changes
- **12-Factor App**: Environment-specific config via environment, app config via files

---

Notes
- Prefer ./tmp for temporary files rather than /tmp
- For realistic JWT auth configuration, see docs/configuration/middleware.md
- For comprehensive environment variable documentation, see [Security Documentation](../security/environment-variables.md#environment-variables)

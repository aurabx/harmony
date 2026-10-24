---
title: Environment Variables
description: Every environment variable Harmony reads, with type, default, and precedence rules.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/configuration-file.md
  - configuration/environment-substitution.md
  - security/secrets.md
---

# Environment Variables

Harmony reads the environment variables below. This page is the single reference for them. This page is part of [Configuration](configuration-file.md).

For how to generate, store, and rotate the secret values, see [Managing Secrets](../security/secrets.md).

## Reference

| Variable | Type | Default | Required | Description |
|----------|------|---------|----------|-------------|
| `RUST_LOG` | Tracing filter directive, such as `harmony=debug,info` | Unset | No | Overrides `logging.log_level` when set to a non-blank value. If the value is not a valid filter, Harmony prints a warning to stderr and uses `logging.log_level`. `logging.log_level` defaults to `error`. |
| `RUNBEAM_MACHINE_TOKEN` | JSON machine token | Unset | No | Pre-provisioned machine token for headless deployments. Read at startup only when cloud integration is enabled. Harmony uses it before any stored token and saves it to token storage. If the JSON does not parse, Harmony logs a warning and uses the stored token. |
| `RUNBEAM_PUSH_CONFIG_ON_STARTUP` | Boolean: `true` (any case) or `1` | `false` | No | When cloud polling starts, push the local configuration to Runbeam Cloud. Any other value counts as false. |
| `RUNBEAM_CLOUD_API_BASE_URL` | URL | Unset | No | Runbeam Cloud API base URL used by `POST /{base_path}/update` and the mesh authentication middleware. Takes priority over the stored CLI setting and `runbeam.cloud_api_base_url`. `POST /{base_path}/token` also sets it for the running process, but only when the request includes `api_base_url` and saving that value to `~/.runbeam/<proxy_id>/config.json` succeeds. |
| `RUNBEAM_ENCRYPTION_KEY` | age X25519 identity: raw `AGE-SECRET-KEY-1...` or base64 of that string | Unset | No (recommended in containers) | Key that encrypts the stored machine token. When unset, the SDK generates a key at `~/.runbeam/<proxy_id>/encryption.key`. The `POST /{base_path}/token` endpoint sets it for the running process when the request includes `encryption_key`. |
| `RUNBEAM_ACCEPT_INVALID_CERTS` | Boolean: exactly `true` or `false` | `false` | No | Disables TLS certificate checks on Runbeam Cloud API calls. Development only. Values such as `1` or `TRUE` do not enable it. |

Any other variable is read only if your configuration file references it. See the next two sections.

## Variables in the Configuration File

You can reference any environment variable in a TOML value with `$VAR_NAME`. Write `$$` for a literal `$`. If a referenced variable is not set, Harmony replaces it with an empty string and logs a warning. See [Environment Variable Substitution](environment-substitution.md).

## Required Variables

List variable names in `proxy.required_env_vars` to make them mandatory. If any listed variable is not set, Harmony refuses to start and names the missing variable.

```toml
[proxy]
id = "harmony-proxy"
required_env_vars = ["API_KEY", "DATABASE_PASSWORD"]
```

See [Environment Variable Substitution](environment-substitution.md) for more examples.

## Precedence

- **TOML only.** Networks, pipelines, endpoints, backends, middleware, services, and storage come from the configuration file. No environment variable overrides them, except through `$VAR` substitution.
- **Logging.** A valid, non-blank `RUST_LOG` wins over `logging.log_level`. Otherwise `logging.log_level` applies.
- **Runbeam Cloud API base URL**, for `POST /{base_path}/update` and mesh authentication:
  1. `RUNBEAM_CLOUD_API_BASE_URL`
  2. `api_base_url` in `~/.runbeam/<proxy_id>/config.json`, saved by the CLI through `POST /{base_path}/token`
  3. `runbeam.cloud_api_base_url` in the TOML file
  4. `https://api.runbeam.cloud`

  Startup cloud polling and `/authorize` do not use this order. They use the `api` value of the primary provider, which defaults to `https://api.runbeam.cloud`. See [Providers](providers.md).
- **Machine token at startup**, when cloud integration is enabled:
  1. `RUNBEAM_MACHINE_TOKEN`, if it parses
  2. The stored token at `~/.runbeam/<proxy_id>/runbeam/auth.json`
  3. No token. Harmony waits for a call to `/authorize`.
- **Encryption key.** See [Encryption Key Management](../security/encryption-key-management.md).

## Examples

Local development:

```bash
export RUST_LOG=harmony=debug,info
cargo run -- --config path/to/config.toml
```

Production container:

```bash
export RUNBEAM_ENCRYPTION_KEY=$(cat /run/secrets/encryption-key)
export RUST_LOG=harmony=info
./harmony --config /etc/harmony/config.toml
```

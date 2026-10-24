---
title: Environment Variable Substitution
description: How to reference environment variables inside configuration files.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/configuration-file.md
---

# Environment Variable Substitution

How to reference environment variables inside configuration files. This page is part of [Configuration](configuration-file.md).

## Environment Variable Substitution

Harmony supports environment variable substitution in all TOML configuration values. This allows you to externalize sensitive data, deployment-specific settings, and other configuration that varies across environments.

### Syntax

Use `$VARIABLE_NAME` to reference environment variables in any TOML value:

```toml
[proxy]
id = "$PROXY_ID"

[network.default.http]
bind_address = "$BIND_ADDRESS"
bind_port = $BIND_PORT

[backends.remote]
service = "http"

[backends.remote.options]
url = "https://$REMOTE_HOST:$REMOTE_PORT/api"
```

### Variable Names

Valid variable names must:
- Start with a letter or underscore
- Contain only alphanumeric characters and underscores
- Examples: `$VAR`, `$VAR_NAME`, `$_VAR123`
- Invalid: `$123VAR` (starts with number), `$MY-VAR` (contains hyphen)

### Behavior

**If variable is set**: Replaced with its value
```bash
export HOST=example.com
# $HOST in config becomes: example.com
```

**If variable is not set**: Replaced with empty string and warning logged
```toml
url = "https://$UNDEFINED_VAR/api"  # becomes: https:///api (warning logged)
```

**Escaped dollar signs**: Use `$$` to represent a literal `$` character
```toml
description = "Cost is $$100"  # becomes: Cost is $100
```

### Examples

#### Externalize Secrets

```toml
[backends.production]
service = "http"

[backends.production.options]
url = "https://api.example.com"
api_key = "$API_KEY"
```

Set at runtime:
```bash
export API_KEY="secret_key_xyz"
cargo run -- --config config/config.toml
```

#### Deployment-Specific Configuration

```toml
[network.default.http]
bind_address = "$BIND_ADDRESS"
bind_port = $BIND_PORT

[proxy]
id = "harmony-$ENVIRONMENT"
```

Run different environments:
```bash
# Development
export BIND_ADDRESS=127.0.0.1 BIND_PORT=8080 ENVIRONMENT=dev
cargo run -- --config config/config.toml

# Production
export BIND_ADDRESS=0.0.0.0 BIND_PORT=443 ENVIRONMENT=prod
./harmony --config /etc/harmony/config.toml
```

#### Multiple Variables

```toml
url = "$PROTOCOL://$HOST:$PORT/path"
# becomes: https://example.com:443/path (if PROTOCOL=https, HOST=example.com, PORT=443)
```

### Best Practices

1. **Use meaningful variable names**: `$API_KEY` instead of `$K`, `$BIND_ADDRESS` instead of `$BA`
2. **Document required variables**: Include a `.env.example` or README section listing all expected environment variables
3. **Use default values in scripts**: Wrap your startup in a shell script that sets defaults
   ```bash
   #!/bin/bash
   export BIND_ADDRESS=${BIND_ADDRESS:-127.0.0.1}
   export BIND_PORT=${BIND_PORT:-8080}
   exec /path/to/harmony --config config.toml
   ```
4. **Never commit secrets**: Use `.gitignore` to exclude `.env` files
5. **Log warnings**: Check logs during startup to catch missing variables early
6. **Mark critical variables as required**: Use `required_env_vars` in `[proxy]` section to enforce presence of critical variables
   ```toml
   [proxy]
   id = "harmony"
   required_env_vars = ["API_KEY", "DATABASE_PASSWORD"]
   ```
7. **Use systemd EnvironmentFile or similar**: Instead of exporting secrets in shell, use secure credential management systems
8. **Rotate secrets regularly**: Check your secret management and rotation policies
9. **Validate logs**: Ensure sensitive values don't leak into logs (only variable names are logged)

### Security Considerations

**Metadata Disclosure**
- Environment variable references are visible in your config files
- Restrict config file permissions: `chmod 600 config.toml` or `chmod 400 config.toml` (read-only after deployment)
- Configuration files should not be committed to version control if they contain `$VAR_NAME` references for sensitive values

**Missing Variables**
- By default, missing environment variables are replaced with empty strings
- Check startup logs for warnings about missing variables: `Environment variable 'X' not found`
- Use `required_env_vars` in your config to enforce critical variables and fail fast if they're missing

**Value Validation**
- Environment variable values are substituted as-is into TOML
- Ensure your values don't contain TOML metacharacters (newlines, unescaped quotes) or they may break configuration
- Startup validation (see below) can help catch malformed values

**Process Environment Visibility**
- On some systems, environment variables are visible to other users via `ps aux` or `/proc`
- For highly sensitive secrets, consider using:
  - systemd `EnvironmentFile` with restricted permissions
  - HashiCorp Vault or cloud provider secret managers (AWS Secrets Manager, Azure Key Vault)
  - Docker secrets or Kubernetes secrets for containerized deployments

**Audit Trail**
- Substituted variable names are logged at INFO level during startup
- Missing variables are logged at WARN level during startup
- Example logs:
  ```
  [INFO] Substituted 3 environment variables: API_KEY, DATABASE_PASSWORD, JWT_SECRET
  [WARN] 1 environment variable not found: MISSING_CONFIG_VAR
  ```
- Review startup logs to verify which variables were actually substituted

**Validation Command**

Test your configuration without starting the server:

```bash
./harmony --config config.toml --validate-config
# Output:
# ✓ Configuration is valid
# ✓ All required environment variables present
# ✓ Substituted 3 environment variables
# Exit code: 0
```

This validates that:
- All required variables are present
- Configuration can be parsed after substitution
- No risky values detected in environment variables

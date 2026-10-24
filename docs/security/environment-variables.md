---
title: Security Environment Variables
description: Security-related environment variables and how to manage them safely.
status: active
updated: 2026-10-24
type: Security
related:
  - security/overview.md
---

# Security Environment Variables

Security-related environment variables and how to manage them safely. This page is part of [Security](overview.md).

## Environment Variables

Harmony uses environment variables for security-sensitive configuration and runtime settings. These variables should be set in your deployment environment and never committed to version control.

### RUNBEAM_ENCRYPTION_KEY

**Purpose**: Provides encryption key for secure token storage (encrypted filesystem storage).

**When Used**: 
- Recommended for all production container deployments to ensure consistent encryption across restarts
- Required for headless/CI environments where persistent token storage is needed
- Optional for local development (auto-generates key if not set)

**Format**: Base64-encoded age X25519 identity key

**Generation**:

```bash
# Linux
export RUNBEAM_ENCRYPTION_KEY=$(age-keygen | base64 -w 0)

# macOS
export RUNBEAM_ENCRYPTION_KEY=$(age-keygen | base64 | tr -d '\n')

# For persistent storage (DO NOT commit to version control)
age-keygen | base64 | tr -d '\n' > .secrets/encryption.key
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/encryption.key)
```

**Security Implications**:
- Without this variable, Harmony auto-generates a key stored at `~/.runbeam/<instance_id>/encryption.key`
- Auto-generated keys are not persistent in ephemeral containers (tokens lost on restart)
- Use different keys for different environments (development, staging, production)
- Rotate keys periodically and migrate tokens using backup/restore procedures
- Store in secret management systems (see Production Deployment Examples below)

**Token Storage Behavior**:
1. **RUNBEAM_ENCRYPTION_KEY Set**: Encrypted filesystem storage with provided key
2. **No Environment Variable**: Encrypted filesystem storage with auto-generated key

See [runbeam-sdk documentation](https://github.com/runbeam/runbeam-sdk) for technical details on the encryption implementation (age X25519 with AES-256-GCM).

### RUNBEAM_JWT_SECRET

**Purpose**: Shared secret for validating JWT tokens from Runbeam Cloud during gateway authorization.

**When Used**:
- Required for production deployments using Runbeam Cloud integration
- Used by Management API `/authorize` endpoint to validate user JWT tokens
- Must match the secret configured in Runbeam Cloud

**Format**: String (recommend 32+ character random value)

**Generation**:

```bash
# Generate strong random secret (Linux/macOS)
export RUNBEAM_JWT_SECRET=$(openssl rand -base64 32)

# Or use a password manager/secret generator
export RUNBEAM_JWT_SECRET="your-secure-secret-from-runbeam-cloud"
```

**Security Implications**:
- Falls back to development default if not set (logs warning - NOT for production)
- Secret must match between Harmony and Runbeam Cloud
- Rotate periodically and update both sides simultaneously
- Never log or expose this value

### RUST_LOG

**Purpose**: Controls logging verbosity for Harmony and dependencies.

**Format**: Tracing filter directive (e.g., `harmony=debug,info`)

**Common Values**:
```bash
# Production (minimal logging)
export RUST_LOG=harmony=info

# Development (detailed Harmony logs)
export RUST_LOG=harmony=debug,info

# Debugging (verbose all modules)
export RUST_LOG=debug

# Trace-level for specific module
export RUST_LOG=harmony::router=trace,harmony=debug
```

**Security Note**: Avoid trace-level logging in production as it may expose sensitive request/response data.

## Environment Variable Best Practices

### Key Rotation and Migration

**RUNBEAM_ENCRYPTION_KEY Rotation**:
1. Generate new encryption key
2. Back up existing encrypted tokens (if needed)
3. Update environment variable with new key
4. Re-authorize gateway to generate new encrypted tokens
5. Verify token storage and retrieval work with new key

**RUNBEAM_JWT_SECRET Rotation**:
1. Coordinate with Runbeam Cloud team for synchronized rotation
2. Update secret in Runbeam Cloud first
3. Update RUNBEAM_JWT_SECRET in Harmony deployment
4. Verify authorization flow works with new secret
5. Monitor for failed authorization attempts

### Secret Management Systems

**HashiCorp Vault**:
```bash
# Store secrets
vault kv put secret/harmony/production \
  encryption_key="$(age-keygen | base64 | tr -d '\n')" \
  jwt_secret="$(openssl rand -base64 32)"

# Retrieve at runtime
export RUNBEAM_ENCRYPTION_KEY=$(vault kv get -field=encryption_key secret/harmony/production)
export RUNBEAM_JWT_SECRET=$(vault kv get -field=jwt_secret secret/harmony/production)
```

**AWS Secrets Manager**:
```bash
# Store secret
aws secretsmanager create-secret \
  --name harmony/production/encryption-key \
  --secret-string "$(age-keygen | base64 -w 0)"

# Retrieve at runtime (or use IAM role in ECS task definition)
export RUNBEAM_ENCRYPTION_KEY=$(aws secretsmanager get-secret-value \
  --secret-id harmony/production/encryption-key \
  --query SecretString --output text)
```

**Azure Key Vault**:
```bash
# Store secret
az keyvault secret set \
  --vault-name harmony-vault \
  --name encryption-key \
  --value "$(age-keygen | base64 -w 0)"

# Retrieve at runtime
export RUNBEAM_ENCRYPTION_KEY=$(az keyvault secret show \
  --vault-name harmony-vault \
  --name encryption-key \
  --query value -o tsv)
```

### Environment-Specific Keys

Use different encryption keys per environment:

```bash
# Development
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/dev-encryption.key)
export RUNBEAM_JWT_SECRET=$(cat .secrets/dev-jwt-secret.txt)

# Staging
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/staging-encryption.key)
export RUNBEAM_JWT_SECRET=$(cat .secrets/staging-jwt-secret.txt)

# Production
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/prod-encryption.key)
export RUNBEAM_JWT_SECRET=$(cat .secrets/prod-jwt-secret.txt)
```

### Container Deployment Considerations

**Persistent vs Ephemeral Keys**:
- **Persistent**: Set RUNBEAM_ENCRYPTION_KEY for tokens to survive container restarts
- **Ephemeral**: Omit variable; tokens lost on restart (acceptable for development)

**Volume Considerations**:
- Auto-generated keys stored at `~/.runbeam/<instance_id>/encryption.key`
- Mount persistent volume at `~/.runbeam` to preserve auto-generated keys
- Alternatively, use RUNBEAM_ENCRYPTION_KEY for explicit key management

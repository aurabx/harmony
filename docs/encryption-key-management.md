# Encryption Key Management

**Last Updated**: 2025-11-30

This document describes how encryption keys are managed across the Runbeam ecosystem for securing machine tokens.

## Overview

Machine tokens issued by Runbeam Cloud are sensitive credentials that grant autonomous API access. The ecosystem provides multiple encryption key management strategies to secure these tokens at rest.

## Architecture

### Components

1. **runbeam-sdk** - Core library with encrypted storage backends
2. **runbeam-cli** - User-facing CLI that can manage keys for Harmony instances
3. **harmony** - Gateway that uses SDK storage for machine tokens

### Storage Backend (runbeam-sdk)

The SDK provides an encrypted storage backend:

#### EncryptedFilesystemStorage
- Stores encrypted files at `~/.runbeam/<instance_id>/auth.json`
- Uses age X25519 encryption
- Encryption key sources (priority order):
  1. `RUNBEAM_ENCRYPTION_KEY` environment variable (base64-encoded)
  2. Auto-generated key at `~/.runbeam/<instance_id>/encryption.key` (0600 permissions)

## Encryption Key Management Strategies

### Strategy 1: CLI-Managed Keys (Recommended for Multiple Instances)

**Use Case:** Managing multiple Harmony instances with centralized key control

**How It Works:**

1. CLI sends keys to Harmony during authorization via `/admin/authorize` endpoint
2. SDK's `save_token_with_key()` function uses the provided key directly
3. No environment variable manipulation - key is passed explicitly to SDK
4. Each instance can have its own encryption key for isolation

**CLI Commands:**

```bash
# Add instance with optional encryption key
runbeam harmony:add -i 127.0.0.1 -p 8081 -l prod-harmony

# Add instance with custom key
runbeam harmony:add -i harmony.prod.example.com -p 443 \
  -l prod-harmony -k AGE-SECRET-KEY-1ABC...

# Set/update key for existing instance
runbeam harmony:set-key --id abc123 -k AGE-SECRET-KEY-1ABC...

# Retrieve key from storage
runbeam harmony:show-key --id abc123

# Delete key from storage
runbeam harmony:delete-key --id abc123

# Authorize instance (uses stored key automatically if available)
runbeam harmony:authorize --id abc123
# Or by label:
runbeam harmony:authorize -l prod-harmony
```

**Authorization Request Format:**

```json
{
  "gateway_code": "my-gateway-001",
  "encryption_key": "QUdFLVNFQ1JFVC1LRVktMTIzNDU2Nzg5MA=="
}
```

### Strategy 2: Environment Variable (Recommended for Containers)

**Use Case:** Containerized deployments, consistent key across restarts

**How It Works:**

1. Set `RUNBEAM_ENCRYPTION_KEY` in container environment
2. SDK automatically uses this key for all token operations
3. Consistent encryption across container restarts

**Docker Example:**

```dockerfile
# Dockerfile
ENV RUNBEAM_ENCRYPTION_KEY=QUdFLVNFQ1JFVC1LRVktMTIzNDU2Nzg5MA==

# Or via docker-compose.yml
services:
  harmony:
    image: harmony:latest
    environment:
      - RUNBEAM_ENCRYPTION_KEY=${RUNBEAM_ENCRYPTION_KEY}
```

**Kubernetes Example:**

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: harmony-secrets
type: Opaque
data:
  encryption-key: QUdFLVNFQ1JFVC1LRVktMTIzNDU2Nzg5MA==
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: harmony
spec:
  template:
    spec:
      containers:
        - name: harmony
          env:
            - name: RUNBEAM_ENCRYPTION_KEY
              valueFrom:
                secretKeyRef:
                  name: harmony-secrets
                  key: encryption-key
```

### Strategy 3: Auto-Generated Keys (Development/Testing Only)

⚠️ **WARNING: NOT SUITABLE FOR PRODUCTION USE**

**Use Case:** Local development, testing, proof-of-concept only

**How It Works:**

1. SDK detects no `RUNBEAM_ENCRYPTION_KEY` is set and no CLI-provided key
2. Generates a new age X25519 identity on first token save
3. Stores at `~/.runbeam/<instance_id>/encryption.key` (0600 permissions)
4. Uses this key for all subsequent token operations

**Critical Limitations:**

- **Key loss = connectivity loss**: If the auto-generated key file is deleted, corrupted, or inaccessible, the encrypted machine token cannot be decrypted
- **No key recovery**: Auto-generated keys have no recovery mechanism
- **Restart risk**: Container restarts without persistent volumes will lose the key file
- **Migration impossible**: Cannot move Harmony instance to another machine without re-authorization
- **Requires re-authorization**: Any key loss scenario requires calling `/admin/authorize` again to get a new machine token

**When This Fails:**

- Docker containers without volume mounts for `~/.runbeam/`
- Ephemeral cloud instances (AWS Fargate, Google Cloud Run, etc.)
- Kubernetes pods without PersistentVolumeClaims
- File system corruption or accidental deletion
- Permission changes to the key file

**For production deployments, use Strategy 1 (CLI-managed keys) or Strategy 2 (environment variables) instead.**

**Key Generation:**

```bash
# Generate key manually (optional)
age-keygen -o encryption.key

# Base64 encode for use in environment variable
base64 encryption.key
```

### Strategy 4: Pre-Provisioned Tokens (Fully Automated Deployments)

**Use Case:** CI/CD, Infrastructure as Code, completely headless deployments

**How It Works:**

For scenarios where you cannot call `/admin/authorize` at all (e.g., immutable infrastructure, automated provisioning), provide the machine token directly via environment variable:

```bash
# Set machine token (JSON format)
export RUNBEAM_MACHINE_TOKEN='{"machine_token":"mt_abc123...","expires_at":"2025-12-31T23:59:59Z","gateway_id":"gw-123","gateway_code":"prod-gateway","abilities":["gateway:read","gateway:write"],"issued_at":"2025-01-01T00:00:00Z"}'

# Optionally set encryption key (for persistence to storage)
export RUNBEAM_ENCRYPTION_KEY=AGE-SECRET-KEY-...

./harmony --config config.toml
```

**Kubernetes Example:**

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: harmony-tokens
type: Opaque
stringData:
  machine-token: |
    {"machine_token":"mt_abc123...","expires_at":"2025-12-31T23:59:59Z","gateway_id":"gw-123","gateway_code":"prod-gateway","abilities":[],"issued_at":"2025-01-01T00:00:00Z"}
  encryption-key: AGE-SECRET-KEY-...
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: harmony-proxy
spec:
  template:
    spec:
      containers:
      - name: harmony
        env:
        - name: RUNBEAM_MACHINE_TOKEN
          valueFrom:
            secretKeyRef:
              name: harmony-tokens
              key: machine-token
        - name: RUNBEAM_ENCRYPTION_KEY
          valueFrom:
            secretKeyRef:
              name: harmony-tokens
              key: encryption-key
```

**Priority:**
- If `RUNBEAM_MACHINE_TOKEN` is set, Harmony uses it immediately
- Otherwise, Harmony loads from storage (if previously saved)
- Otherwise, waits for `/admin/authorize` call

## Priority Order

When saving machine tokens, the SDK resolves encryption keys in this order:

1. **CLI-provided key** (via `encryption_key` in `/admin/authorize` request)
2. **Environment variable** (`RUNBEAM_ENCRYPTION_KEY`)
3. **Auto-generated key** (created on first use)

## Security Considerations

### Key Storage

**Auto-Generated Keys:**
- File permissions: 0600 (owner read/write only)
- Location: `~/.runbeam/<instance_id>/encryption.key`
- Not suitable for production - use explicit keys instead

### Key Rotation

To rotate encryption keys:

```bash
# 1. Generate new key and save to file
age-keygen -o new-encryption.key

# 2. Extract the secret key (starts with AGE-SECRET-KEY-1...)
NEW_KEY=$(grep AGE-SECRET-KEY new-encryption.key)

# 3. Set new key for instance
runbeam harmony:set-key --id abc123de -k "$NEW_KEY"

# 4. Re-authorize gateway with new key
runbeam harmony:authorize --id abc123de

# 5. Verify token is encrypted with new key
# (Old token is overwritten, automatically uses new key)

# 6. Securely delete the temporary key file
shred -u new-encryption.key
```

### Multi-Instance Isolation

Each Harmony instance uses its `proxy.id` (from config) as the SDK's `instance_id`:

```rust
// In authorize handler
let proxy_id = crate::globals::get_config()
    .map(|config| config.proxy.id.clone())
    .unwrap_or_else(|| "harmony".to_string());

save_token(&proxy_id, &machine_token).await?;
```

This ensures tokens for different instances are stored separately:
- `~/.runbeam/harmony-prod/auth.json`
- `~/.runbeam/harmony-staging/auth.json`
- `~/.runbeam/harmony-dev/auth.json`

## Implementation Details

### Harmony Authorization Flow

```rust
// src/models/services/types/management/authorize.rs

pub async fn handle_authorize(/* ... */) -> Result<serde_json::Value, (u16, String)> {
    // ... JWT validation ...
    
    // Get proxy ID for instance isolation
    let proxy_id = crate::globals::get_config()
        .map(|config| config.proxy.id.clone())
        .unwrap_or_else(|| "harmony".to_string());

    // Use appropriate save function based on whether encryption key was provided
    if let Some(ref encryption_key) = request.encryption_key {
        // CLI provided a key - use it directly
        save_token_with_key(&proxy_id, &machine_token, encryption_key).await?;
    } else {
        // No key provided - SDK uses env var or auto-generates
        save_token(&proxy_id, &machine_token).await?;
    }
}
```

### SDK Storage Resolution

```rust
// runbeam-sdk/src/runbeam_api/token_storage.rs

async fn get_storage_backend(instance_id: &str) -> Result<StorageBackendType, StorageError> {
    // Use encrypted filesystem storage
    // (uses RUNBEAM_ENCRYPTION_KEY or auto-generates)
    let encrypted = EncryptedFilesystemStorage::new_with_instance(instance_id).await?;
    Ok(StorageBackendType::Encrypted(encrypted))
}
```

## Testing

### Unit Tests

```bash
# Test SDK token storage
cd runbeam-sdk
cargo test token_storage

# Test Harmony authorization
cd harmony-proxy
cargo test models::services::types::management::authorize
```

### Integration Testing

```bash
# Test CLI-managed keys
runbeam harmony:add -i 127.0.0.1 -p 9090 -l test-instance
runbeam harmony:show-key --id <instance-id>
runbeam harmony:authorize -l test-instance

# Test environment variable approach
# First generate a key to file
age-keygen -o test-key.age
# Extract and base64 encode the secret key line
ENCRYPTION_KEY=$(grep AGE-SECRET-KEY test-key.age)
export RUNBEAM_ENCRYPTION_KEY="$ENCRYPTION_KEY"
./harmony --config config.toml

# Verify token encryption
ls -la ~/.runbeam/harmony/auth.json
# Should show encrypted data, not plaintext JSON
```

## Troubleshooting

### Key Not Found

```
Error: Failed to load encryption key for instance 'prod-harmony'
```

**Solution:** Set the key using `runbeam harmony:set-key prod-harmony AGE-SECRET-KEY-...`

### Permission Denied

```
Error: Permission denied: ~/.runbeam/harmony/encryption.key
```

**Solution:** Check file permissions: `chmod 600 ~/.runbeam/harmony/encryption.key`

### Keyring Unavailable

```
Warning: Keyring storage unavailable, falling back to encrypted filesystem
```

**Expected on:**
- Linux without Secret Service API
- Headless servers
- Container environments

**Solution:** Set `RUNBEAM_ENCRYPTION_KEY` explicitly

## Best Practices

1. **Production Containers/K8s:** Use Strategy 2 (environment variables) with secret management (Kubernetes Secrets, AWS Secrets Manager, HashiCorp Vault, etc.)
2. **Production Multiple Instances:** Use Strategy 1 (CLI-managed keys) for centralized control with OS keyring backup
3. **Fully Automated Deployments:** Use Strategy 4 (pre-provisioned tokens) for immutable infrastructure
4. **Development/Testing Only:** Strategy 3 (auto-generated keys) acceptable with understanding that key loss requires re-authorization
5. **Key Rotation:** Rotate keys periodically (90 days recommended), especially after team member changes or security incidents
6. **Backup Keys:** Store production encryption keys in a secure password manager or secrets vault - key loss means connectivity loss
7. **Never Use Auto-Generated Keys in Production:** They cannot survive container restarts, migrations, or filesystem issues
8. **Audit:** Log key usage, rotation events, and authorization attempts for security compliance

## Related Documentation

- [Management API](./management-api.md) - `/admin/authorize` endpoint details
- [Security](./security.md) - Overall security architecture
- [runbeam-cli README](../../runbeam-cli/README.md) - CLI usage and commands
- [runbeam-sdk WARP.md](../../runbeam-sdk/WARP.md) - SDK architecture and storage backends

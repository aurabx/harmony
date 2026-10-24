---
title: Encryption Key Management
description: How Harmony stores and resolves encryption keys and machine tokens.
status: active
updated: 2026-10-24
type: Security
---

# Encryption Key Management

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

The strategies Harmony supports for storing and supplying encryption keys. See [Encryption Key Management Strategies](encryption-key-strategies.md).

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

- [Management API](../api/management-api.md) - `/admin/authorize` endpoint details
- [Security](overview.md) - Overall security architecture
- [runbeam-cli README](../../../runbeam-cli/README.md) - CLI usage and commands
- `runbeam-sdk` repository README - SDK architecture and storage backends (separate repository)

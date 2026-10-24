---
title: Encryption Key Management Strategies
description: The strategies Harmony supports for storing and supplying encryption keys.
status: active
updated: 2026-10-24
type: Security
related:
  - security/encryption-key-management.md
---

# Encryption Key Management Strategies

The strategies Harmony supports for storing and supplying encryption keys. This page is part of [Encryption Key Management](encryption-key-management.md).

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

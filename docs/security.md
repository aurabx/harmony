# Security

**Last Updated**: 2025-11-30

## Gateway Authentication (Middleware)

JWT authentication for gateway middleware:
- Prefer RS256 with an RSA public key (PEM). Do not use HS256 in production
- Enforce algorithm strictly; never accept a token signed with a different algorithm
- Validate exp, nbf, iat with minimal leeway (e.g., 60 seconds)
- Validate iss and aud where applicable
- Map all verification failures to HTTP 401 Unauthorized
- Startup safety: the middleware panics if RS256 keys are missing and HS256 is not explicitly enabled

## Runbeam Cloud Authorization

The Management API `/authorize` endpoint uses a different JWT flow for authorizing Harmony with Runbeam Cloud:

### JWT Validation Flow
1. **User Authentication**: User authenticates via `runbeam login` CLI command and receives a JWT token
2. **Token Validation**: Harmony validates the JWT locally using HS256 with shared secret
3. **API Base URL Extraction**: The JWT's `iss` (issuer) claim contains the Runbeam Cloud API base URL
4. **Token Exchange**: Harmony calls Runbeam Cloud API to exchange user JWT for machine-scoped token
5. **Token Storage**: Machine token (30-day expiry) is stored locally at `./tmp/runbeam/auth.json`

### Security Configuration
- **JWT Secret**: Set via `RUNBEAM_JWT_SECRET` environment variable
  - Must match the secret Runbeam Cloud uses to sign JWTs (HS256)
  - Falls back to development default if not set (logs warning)
  - Never hardcode secrets in configuration files
- **Token Storage**: Machine tokens stored in JSON format with file permissions
  - Default path: `./tmp/runbeam/auth.json` (configurable via storage backend)
  - Contains: machine_token, expires_at, gateway_id, gateway_code, abilities
  - Consider restricting file permissions to owner-only (chmod 600)

### Machine Token Lifecycle
- **Expiry**: Machine tokens expire after 30 days (configured server-side)
- **Renewal**: Must re-run `runbeam harmony:authorize` before expiry
- **Revocation**: Tokens can be revoked via Runbeam Cloud API
- **Validation**: Check `is_valid()` method before using stored tokens

### Security Best Practices
- Run Harmony with least-privilege user account
- Restrict network access to Management API (bind to 127.0.0.1)
- Use firewall rules to limit access to management port
- Rotate machine tokens regularly (before 30-day expiry)
- Monitor token usage via Runbeam Cloud dashboard
- Never log actual token values (only metadata)

### Integration Security
- **API Communication**: All Runbeam Cloud API calls use HTTPS (in production)
- **Token Transmission**: JWT tokens only sent in Authorization headers
- **Error Handling**: Detailed errors logged server-side, generic errors returned to client
- **Rate Limiting**: Runbeam Cloud enforces rate limits on authorization endpoint

## Environment Variables

Harmony uses environment variables for security-sensitive configuration and runtime settings. These variables should be set in your deployment environment and never committed to version control.

### RUNBEAM_ENCRYPTION_KEY

**Purpose**: Provides encryption key for secure token storage when OS keyring is unavailable (typical in containers and headless environments).

**When Used**: 
- Automatically used when OS keyring (macOS Keychain, Linux Secret Service, Windows Credential Manager) is unavailable
- Recommended for all production container deployments to ensure consistent encryption across restarts
- Required for headless/CI environments where keyring access is not available

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
1. **Keyring Available**: Tokens stored in OS keyring (most secure, automatic)
2. **Keyring Unavailable + RUNBEAM_ENCRYPTION_KEY Set**: Encrypted filesystem storage with provided key
3. **Keyring Unavailable + No Environment Variable**: Encrypted filesystem storage with auto-generated key

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

### RUNBEAM_DISABLE_KEYRING

**Purpose**: Forces use of encrypted filesystem storage instead of OS keyring.

**When Used**:
- Testing encrypted filesystem storage behavior
- Debugging keyring-related issues
- Environments where keyring is available but not desired

**Format**: Any value (presence enables it)

```bash
export RUNBEAM_DISABLE_KEYRING=1
```

**Note**: This is primarily for testing. In production containers, keyring is typically unavailable anyway.

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

## Production Deployment Examples

### Docker Compose with Environment File

Create `.env` file (DO NOT commit):
```bash
# .env
RUNBEAM_ENCRYPTION_KEY=AGE-SECRET-KEY-1...<base64-encoded>
RUNBEAM_JWT_SECRET=your-jwt-secret-here
RUST_LOG=harmony=info
```

Reference in `docker-compose.yml`:
```yaml
services:
  harmony:
    image: harmony:latest
    env_file:
      - .env
    volumes:
      - ./config:/etc/harmony:ro
      - harmony-data:/data/harmony
```

### Docker with Secrets File

```bash
# Generate and save secrets (outside version control)
mkdir -p .secrets
age-keygen | base64 | tr -d '\n' > .secrets/encryption.key
openssl rand -base64 32 > .secrets/jwt-secret.txt

# Run container with secrets
docker run -d \
  -e RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/encryption.key) \
  -e RUNBEAM_JWT_SECRET=$(cat .secrets/jwt-secret.txt) \
  -e RUST_LOG=harmony=info \
  -v $(pwd)/config:/etc/harmony:ro \
  -p 8080:8080 -p 9090:9090 \
  harmony:latest
```

### Kubernetes with Secrets

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: harmony-secrets
  namespace: production
type: Opaque
data:
  # Base64-encode your already-base64-encoded age key
  encryption-key: QUFH... 
  # Base64-encode your JWT secret
  jwt-secret: eW91ci1zZWNyZXQ=
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: harmony
  namespace: production
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: harmony
        image: harmony:latest
        env:
        - name: RUNBEAM_ENCRYPTION_KEY
          valueFrom:
            secretKeyRef:
              name: harmony-secrets
              key: encryption-key
        - name: RUNBEAM_JWT_SECRET
          valueFrom:
            secretKeyRef:
              name: harmony-secrets
              key: jwt-secret
        - name: RUST_LOG
          value: "harmony=info"
        volumeMounts:
        - name: config
          mountPath: /etc/harmony
          readOnly: true
        - name: data
          mountPath: /data/harmony
      volumes:
      - name: config
        configMap:
          name: harmony-config
      - name: data
        persistentVolumeClaim:
          claimName: harmony-data
```

**Generate Kubernetes secret**:
```bash
# Generate keys
ENCRYPTION_KEY=$(age-keygen | base64 | tr -d '\n')
JWT_SECRET=$(openssl rand -base64 32)

# Create secret (double base64 encoding for Kubernetes)
kubectl create secret generic harmony-secrets \
  --from-literal=encryption-key=$(echo -n "$ENCRYPTION_KEY" | base64) \
  --from-literal=jwt-secret=$(echo -n "$JWT_SECRET" | base64) \
  --namespace production
```

### AWS ECS with Secrets Manager

**Store secrets**:
```bash
# Store encryption key
aws secretsmanager create-secret \
  --name harmony/production/encryption-key \
  --description "Harmony token encryption key" \
  --secret-string "$(age-keygen | base64 -w 0)"

# Store JWT secret
aws secretsmanager create-secret \
  --name harmony/production/jwt-secret \
  --description "Harmony JWT validation secret" \
  --secret-string "$(openssl rand -base64 32)"
```

**ECS Task Definition**:
```json
{
  "family": "harmony-production",
  "taskRoleArn": "arn:aws:iam::ACCOUNT:role/harmony-task-role",
  "executionRoleArn": "arn:aws:iam::ACCOUNT:role/harmony-execution-role",
  "containerDefinitions": [
    {
      "name": "harmony",
      "image": "ACCOUNT.dkr.ecr.REGION.amazonaws.com/harmony:latest",
      "secrets": [
        {
          "name": "RUNBEAM_ENCRYPTION_KEY",
          "valueFrom": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:harmony/production/encryption-key::"
        },
        {
          "name": "RUNBEAM_JWT_SECRET",
          "valueFrom": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:harmony/production/jwt-secret::"
        }
      ],
      "environment": [
        {
          "name": "RUST_LOG",
          "value": "harmony=info"
        }
      ],
      "portMappings": [
        {
          "containerPort": 8080,
          "protocol": "tcp"
        },
        {
          "containerPort": 9090,
          "protocol": "tcp"
        }
      ]
    }
  ]
}
```

**IAM Policy for Secrets Access**:
```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "secretsmanager:GetSecretValue"
      ],
      "Resource": [
        "arn:aws:secretsmanager:REGION:ACCOUNT:secret:harmony/production/*"
      ]
    }
  ]
}
```

### Secret Rotation Procedures

**Zero-Downtime Encryption Key Rotation**:
1. Generate new key: `NEW_KEY=$(age-keygen | base64 | tr -d '\n')`
2. Update secret in secret manager
3. Rolling restart of containers (old tokens still work temporarily)
4. Re-authorize gateway: `runbeam harmony:authorize`
5. New tokens encrypted with new key
6. Old tokens remain accessible until expiry (30 days)

**JWT Secret Rotation** (requires coordination):
1. Coordinate maintenance window with Runbeam Cloud team
2. Update secret in Runbeam Cloud first
3. Update RUNBEAM_JWT_SECRET in all Harmony deployments
4. Restart Harmony instances
5. Test authorization flow: `curl -X POST http://localhost:9090/admin/authorize`
6. Monitor logs for JWT validation errors

Key management
- Store public keys on disk with appropriate permissions
- Rotate keys periodically and have a plan to reload configuration
- Avoid embedding secrets in config files; prefer environment variables or secret managers

Temporary files
- Use ./tmp in the working directory for temporary output (not /tmp)

Encryption
- The project uses AES-256-GCM with ephemeral public key, IV, and authentication tag encoded in base64 where encryption features are utilized

Operational tips
- Use least-privilege file permissions for config and logs
- Enable structured logging and tracing to aid incident response
---
title: Production Deployment Security
description: Example secure production deployments for Docker, Kubernetes, and AWS ECS, with secret rotation procedures.
status: active
updated: 2026-10-24
type: Security
related:
  - security/overview.md
---

# Production Deployment Security

Example secure production deployments for Docker, Kubernetes, and AWS ECS, with secret rotation procedures. This page is part of [Security](overview.md).

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

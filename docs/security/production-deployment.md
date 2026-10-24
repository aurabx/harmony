---
title: Production Deployment Security
description: Example secure production deployments for Docker, Kubernetes, and AWS ECS, with an encryption key rotation procedure.
status: active
updated: 2026-10-24
type: Security
related:
  - security/overview.md
---

# Production Deployment Security

Example secure production deployments for Docker, Kubernetes, and AWS ECS, with an encryption key rotation procedure. This page is part of [Security](overview.md).

## Production Deployment Examples

Harmony stores the machine token under `~/.runbeam`. Each example sets `RUNBEAM_ENCRYPTION_KEY` and mounts a persistent volume there. Both are needed for the token to survive a restart. The published image sets no `USER`, so the path is `/root/.runbeam`. The ECS example has no volume, so supply `RUNBEAM_MACHINE_TOKEN` or authorize again after each new task starts.

### Docker Compose with Environment File

Create `.env` file (DO NOT commit):
```bash
# .env
RUNBEAM_ENCRYPTION_KEY=AGE-SECRET-KEY-1...
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
      # Token and key storage. The image runs as root, so HOME is /root.
      - harmony-runbeam:/root/.runbeam

volumes:
  harmony-runbeam:
```

### Docker with Secrets File

```bash
# Generate and save secrets (outside version control)
mkdir -p .secrets
age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-' > .secrets/encryption.key

# Run container with secrets
docker run -d \
  -e RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/encryption.key) \
  -e RUST_LOG=harmony=info \
  -v $(pwd)/config:/etc/harmony:ro \
  -v harmony-runbeam:/root/.runbeam \
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
  # Base64 of the AGE-SECRET-KEY-1... line (Kubernetes encodes all data values)
  encryption-key: QUdFLVNFQ1JFVC1LRVktMS4uLg==
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: harmony
  namespace: production
spec:
  # One replica: the token volume below is a single ReadWriteOnce claim.
  replicas: 1
  selector:
    matchLabels:
      app: harmony
  template:
    metadata:
      labels:
        app: harmony
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
        - name: RUST_LOG
          value: "harmony=info"
        volumeMounts:
        - name: config
          mountPath: /etc/harmony
          readOnly: true
        # Token and key storage. The image runs as root, so HOME is /root.
        - name: runbeam-data
          mountPath: /root/.runbeam
      volumes:
      - name: config
        configMap:
          name: harmony-config
      - name: runbeam-data
        persistentVolumeClaim:
          claimName: harmony-runbeam
```

**Generate Kubernetes secret**:
```bash
# Generate the key
ENCRYPTION_KEY=$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')

# Create the secret (kubectl base64-encodes literal values itself)
kubectl create secret generic harmony-secrets \
  --from-literal=encryption-key="$ENCRYPTION_KEY" \
  --namespace production
```

### AWS ECS with Secrets Manager

**Store secrets**:
```bash
# Store encryption key
aws secretsmanager create-secret \
  --name harmony/production/encryption-key \
  --description "Harmony token encryption key" \
  --secret-string "$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')"
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

**Encryption Key Rotation**:

A new key cannot decrypt a token saved with the old key. Plan for a short loss of cloud connectivity on each instance until it is re-authorized.

1. Generate a new key: `NEW_KEY=$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')`
2. Update the secret in your secret manager.
3. Restart the containers so they read the new key.
4. Re-authorize each gateway: `runbeam harmony:authorize`
5. Check the logs for a successful token save and for cloud polling starting.

See [Managing Secrets](secrets.md) for key generation and secret manager commands.

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

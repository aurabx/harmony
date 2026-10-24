---
title: Managing Secrets
description: How to generate, store, rotate, and protect the secret values Harmony reads from the environment.
status: active
updated: 2026-10-24
type: Security
related:
  - security/overview.md
  - configuration/environment.md
  - security/encryption-key-strategies.md
---

# Managing Secrets

How to handle the secret values Harmony reads from the environment. This page is part of [Security](overview.md).

For the type, default, and precedence of each variable, see [Environment Variables](../configuration/environment.md). For the ways to supply an encryption key (CLI, environment, auto-generated, pre-provisioned token), see [Encryption Key Management Strategies](encryption-key-strategies.md).

Set secrets in the deployment environment. Never commit them to version control.

## RUNBEAM_ENCRYPTION_KEY

This key encrypts the stored machine token at `~/.runbeam/<proxy_id>/runbeam/auth.json`. The SDK uses age X25519 encryption. The SDK sets 0600 permissions on a generated `encryption.key`. It does not set permissions on the token file, so restrict access to `~/.runbeam` yourself.

### Generate a Key

The value must be the `AGE-SECRET-KEY-1...` line on its own, or base64 of that line. The full `age-keygen` output also holds comment lines. Do not encode the full output, because Harmony cannot parse it.

```bash
# Raw key (works on Linux and macOS)
export RUNBEAM_ENCRYPTION_KEY=$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')

# Save to a file outside version control
mkdir -p .secrets
age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-' > .secrets/encryption.key
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/encryption.key)
```

### Security Implications

- Without this variable, the SDK generates a key at `~/.runbeam/<proxy_id>/encryption.key`.
- Anyone who can read the key and `auth.json` can recover the machine token.
- If the key is lost or changes, Harmony cannot decrypt the stored token. You must authorize the gateway again.

### Rotate the Key

A new key cannot decrypt a token saved with the old key. Rotation therefore means re-authorization.

1. Generate a new key.
2. Update the key in your secret manager or environment.
3. Restart Harmony.
4. Authorize the gateway again, for example with `runbeam harmony:authorize`.
5. Check the logs for a successful token save and for cloud polling starting.

## RUNBEAM_MACHINE_TOKEN

This variable holds a full machine token as JSON. Anyone who has it can act as the gateway against Runbeam Cloud until the token expires or is revoked.

- Store it in a secret manager and inject it at runtime.
- Do not print it in shell history, CI logs, or container labels.
- Harmony saves it to encrypted token storage at startup. The variable is read again on every start, so the stored copy matters only if you later remove the variable.

See [Strategy 4: Pre-Provisioned Tokens](encryption-key-strategies.md#strategy-4-pre-provisioned-tokens-fully-automated-deployments) for the JSON format and a Kubernetes example.

## RUNBEAM_ACCEPT_INVALID_CERTS

Setting this to `true` turns off TLS certificate checks on calls to Runbeam Cloud. An attacker on the network path could then read or change the machine token and configuration in transit. Use it only against a local development server. Never set it in staging or production.

## Secret Managers

**HashiCorp Vault**:
```bash
# Store
vault kv put secret/harmony/production \
  encryption_key="$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')"

# Retrieve at runtime
export RUNBEAM_ENCRYPTION_KEY=$(vault kv get -field=encryption_key secret/harmony/production)
```

**AWS Secrets Manager**:
```bash
# Store
aws secretsmanager create-secret \
  --name harmony/production/encryption-key \
  --secret-string "$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')"

# Retrieve at runtime (or use the secrets block in an ECS task definition)
export RUNBEAM_ENCRYPTION_KEY=$(aws secretsmanager get-secret-value \
  --secret-id harmony/production/encryption-key \
  --query SecretString --output text)
```

**Azure Key Vault**:
```bash
# Store
az keyvault secret set \
  --vault-name harmony-vault \
  --name encryption-key \
  --value "$(age-keygen 2>/dev/null | grep '^AGE-SECRET-KEY-')"

# Retrieve at runtime
export RUNBEAM_ENCRYPTION_KEY=$(az keyvault secret show \
  --vault-name harmony-vault \
  --name encryption-key \
  --query value -o tsv)
```

See [Production Deployment Security](production-deployment.md) for Docker, Kubernetes, and ECS examples.

## Per-Environment Keys

Use a different encryption key in each environment. A leaked development key then cannot decrypt a production token.

```bash
# Development
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/dev-encryption.key)

# Staging
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/staging-encryption.key)

# Production
export RUNBEAM_ENCRYPTION_KEY=$(cat .secrets/prod-encryption.key)
```

## Persistent and Ephemeral Keys in Containers

The token is stored under `~/.runbeam` in the container. A recreated container loses that directory unless it sits on a persistent volume. To keep the token across restarts, you need both of these:

- **A stable key.** Set `RUNBEAM_ENCRYPTION_KEY`, or keep the generated `encryption.key` on the volume.
- **A persistent volume at `~/.runbeam`** for the user that runs Harmony. The published `Dockerfile` sets no `USER`, so Harmony runs as root and the path is `/root/.runbeam`.

Without a volume, the token is lost on every restart whatever the key. Either authorize again after each restart or supply the token through `RUNBEAM_MACHINE_TOKEN`. This is acceptable for development only.

## Logging

Avoid `trace` level logging in production. It can write request and response data, including sensitive values, to the logs. Use `RUST_LOG=harmony=info` or the `logging.log_level` setting.

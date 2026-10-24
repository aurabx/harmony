---
title: Network Configuration
description: How to configure networks, bind addresses, TLS, and WireGuard.
status: active
updated: 2026-10-24
type: Configuration
related:
  - configuration/configuration-file.md
---

# Network Configuration

How to configure networks, bind addresses, TLS, and WireGuard. This page is part of [Configuration](configuration-file.md).

## Network Configuration

### Plain HTTP

```toml
[network.default]
interface = "wg0"
enable_wireguard = false

[network.default.http]
bind_address = "0.0.0.0"
bind_port = 8080
```

### HTTPS with TLS

To enable HTTPS on the HTTP adapter, add both `cert_path` and `key_path` to the TCP configuration:

```toml
[network.secure]
interface = "wg0"
enable_wireguard = false

[network.secure.http]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
```

**Requirements**:
- Both `cert_path` and `key_path` must be provided to enable HTTPS
- Certificate and private key must be in PEM format
- Supported key formats: PKCS#8 (preferred) and RSA PKCS#1 (legacy)
- The adapter automatically uses TLS 1.3 with HTTP/1.1 and HTTP/2 ALPN

### HTTP to HTTPS Redirect

To force all HTTP requests to redirect to HTTPS, use the `force_https` option:

```toml
[network.redirect]
interface = "wg0"
enable_wireguard = false

[network.redirect.http]
bind_address = "0.0.0.0"
bind_port = 80
force_https = true
```

**Behavior**:
- Returns HTTP 301 (Moved Permanently) redirect to `https://` URL
- Only applies when TLS is NOT configured (no `cert_path`/`key_path`)
- Preserves the original path and query parameters
- Uses the `Host` header to construct the HTTPS URL

**Common Use Case**: Run two networks - one on port 80 with `force_https = true` to redirect, and one on port 443 with TLS certificates to handle HTTPS requests.

**Certificate Generation** (for testing):

```bash
# Generate self-signed certificate for testing
openssl req -x509 -newkey rsa:4096 -nodes \
  -keyout key.pem -out cert.pem -days 365 \
  -subj "/CN=localhost"
```

**Production Certificates**:
- Use Let's Encrypt (certbot) for free production certificates
- Or obtain certificates from your organization's certificate authority
- Ensure certificates are renewed before expiration

### Private Key Security

**Supported Key Formats:**
- PKCS#8 unencrypted (RSA and ECDSA) - preferred format
- RSA PKCS#1 unencrypted (legacy format)

**Supported Key Algorithms:**
- RSA (2048, 3072, 4096 bit)
- ECDSA (P-256, P-384, P-521 curves)

**Encrypted Keys Not Supported:**

Harmony does not support encrypted private keys (PKCS#8 with PBES2 or encrypted RSA PKCS#1). If you have encrypted keys, decrypt them first:

```bash
# Decrypt PKCS#8 encrypted key
openssl pkcs8 -in encrypted_key.pem -out key.pem

# Decrypt RSA encrypted key (legacy format)
openssl rsa -in encrypted_rsa_key.pem -out key.pem

# Secure the unencrypted key
chmod 600 key.pem
chown harmony:harmony key.pem  # If running as harmony user
```

**Why No Encrypted Key Support?**

Encrypted keys with passwords stored in configuration files provide no real security benefit:
1. Password stored alongside encrypted key = no protection
2. Industry best practice is unencrypted keys with filesystem permissions
3. Modern deployments use secret management systems instead

**Security Best Practices:**

1. **File System Permissions** (primary security mechanism):
   ```bash
   chmod 600 /etc/harmony/certs/privkey.pem
   chown harmony:harmony /etc/harmony/certs/privkey.pem
   ```

2. **Secret Management Systems** (recommended for production):
   - HashiCorp Vault
   - AWS Secrets Manager
   - Azure Key Vault
   - Google Cloud Secret Manager
   
3. **Container Deployments**:
   - Docker secrets: `docker secret create harmony_key key.pem`
   - Kubernetes secrets: mount as read-only volume
   - Environment variable injection (base64 encoded)

4. **Automated Certificate Management**:
   - Use certbot with Let's Encrypt for automatic renewal
   - Set up hooks to reload Harmony after certificate renewal
   - Monitor certificate expiration dates

5. **Hardware Security Modules (HSM)**:
   - For highly sensitive environments, use HSM-backed certificates
   - Consider cloud provider HSM services (AWS CloudHSM, Azure Dedicated HSM)

6. **Key Rotation**:
   - Rotate private keys periodically (e.g., annually)
   - Use hot reload to apply new certificates without downtime
   - Keep backup of old keys during rotation period

**Example Production Setup:**

```bash
# 1. Store key in secret manager
aws secretsmanager create-secret \
  --name harmony-tls-key \
  --secret-string file://privkey.pem

# 2. Retrieve at container startup
aws secretsmanager get-secret-value \
  --secret-id harmony-tls-key \
  --query SecretString \
  --output text > /run/secrets/privkey.pem

# 3. Set permissions
chmod 600 /run/secrets/privkey.pem

# 4. Start Harmony
./harmony --config /etc/harmony/config.toml
```

### HTTP/3 with QUIC

```toml
[network.http3_network]
interface = "wg0"
enable_wireguard = false

[network.http3_network.http3]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
```

HTTP/3 always uses TLS and requires certificates. It runs over UDP instead of TCP.

### Dual-Mode: HTTP and HTTPS on Same Network

You can run both plain HTTP and HTTPS by configuring multiple networks:

```toml
# Plain HTTP for internal traffic
[network.internal]
interface = "wg0"
enable_wireguard = false

[network.internal.http]
bind_address = "127.0.0.1"
bind_port = 8080

# HTTPS for external traffic
[network.external]
interface = "wg0"
enable_wireguard = false

[network.external.http]
bind_address = "0.0.0.0"
bind_port = 443
cert_path = "/etc/harmony/certs/fullchain.pem"
key_path = "/etc/harmony/certs/privkey.pem"
```

Then configure your pipelines to use the appropriate network:

```toml
[pipelines.internal_api]
networks = ["internal"]
endpoints = ["api"]
backends = ["backend"]
middleware = { incoming = [], outgoing = [] }

[pipelines.external_api]
networks = ["external"]
endpoints = ["api"]
backends = ["backend"]
middleware = { incoming = ["jwt_auth"], outgoing = [] }
```

---
title: Security
description: Gateway authentication, Runbeam Cloud authorisation, and secret handling.
status: active
updated: 2026-10-24
type: Security
---

# Security

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
1. **User Authentication**: The user runs `runbeam login` and receives a JWT.
2. **Token Validation**: Harmony validates the JWT with RS256 only. It reads the `kid` from the token header and the issuer from the `iss` claim. It fetches the signing key from the JWKS endpoint at the issuer's base URL and caches it for `proxy.jwks_cache_duration_hours`. It then checks the signature and expiry. Harmony passes no trusted issuer list, so the SDK accepts a token from any issuer whose JWKS it can fetch. It logs a security warning when it does this.
3. **Token Exchange**: Harmony calls the Runbeam Cloud API to exchange the user JWT for a machine-scoped token. It uses the `api` URL of the primary provider.
4. **Token Storage**: Harmony stores the machine token, encrypted, at `~/.runbeam/<proxy_id>/runbeam/auth.json`.

No shared secret is involved. Harmony needs network access to the issuer's JWKS endpoint.

### Security Configuration
- **Token Storage**: Machine tokens are stored encrypted in JSON format
  - Path: `~/.runbeam/<proxy_id>/runbeam/auth.json`
  - Contains: machine_token, expires_at, gateway_id, abilities, issued_at (encrypted with age X25519)
  - File permissions: the SDK does not set them. It sets 0600 only on a generated `encryption.key`. Restrict access to `~/.runbeam` yourself.
- **Encryption Key**: To keep the stored token across container restarts, set `RUNBEAM_ENCRYPTION_KEY` and put `~/.runbeam` on a persistent volume. See [Managing Secrets](secrets.md).
- Never hardcode secrets in configuration files

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
- **Error Handling**: `/authorize` returns the underlying error text to the client, for example `Invalid or expired token: <details>` and `Authorization failed: <details>`. Restrict access to the management network.
- **Rate Limiting**: Runbeam Cloud enforces rate limits on authorization endpoint

## Managing Secrets

How to generate, store, rotate, and protect secret values. See [Managing Secrets](secrets.md). For the full list of environment variables, see [Environment Variables](../configuration/environment.md).

## Production Deployment Security

Example secure production deployments for Docker, Kubernetes, and AWS ECS, with an encryption key rotation procedure. See [Production Deployment Security](production-deployment.md).

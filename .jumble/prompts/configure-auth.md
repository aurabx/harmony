# Configuring Authentication in Harmony

## Reference Documentation
Full details: `docs/middleware.md` (Authentication section)

## Overview

Harmony supports two authentication middleware types:
- **JWT Auth** - Bearer token validation (RS256 or HS256)
- **Basic Auth** - Username/password validation

Authentication middleware should be placed **early** in the pipeline to reject unauthenticated requests before expensive processing.

## JWT Authentication (Recommended)

### RS256 (Production)

Uses RSA public key to verify tokens signed by your IdP.

```toml
[middleware.jwt_auth]
type = "jwt_auth"
public_key_path = "/etc/harmony/jwt_public.pem"
issuer = "https://auth.example.com/"
audience = "harmony"
leeway_secs = 60  # Clock skew tolerance
```

**Required files**:
- RSA public key in PEM format at `public_key_path`

**Claims validated**:
- `exp` (expiration) - with leeway
- `nbf` (not before) - with leeway
- `iat` (issued at) - with leeway
- `iss` (issuer) - if configured
- `aud` (audience) - if configured

### HS256 (Development/Testing Only)

Uses shared secret. **Not recommended for production**.

```toml
[middleware.jwt_auth]
type = "jwt_auth"
use_hs256 = true
hs256_secret = "$JWT_SECRET"  # Use env var
issuer = "https://auth.example.com/"
audience = "harmony"
```

**Security note**: Must explicitly set `use_hs256 = true`. Without this AND without a `public_key_path`, Harmony will panic at startup to prevent insecure defaults.

## Basic Authentication

Simple username/password validation.

```toml
[middleware.basic_auth]
type = "basic_auth"
username = "$BASIC_AUTH_USER"
password = "$BASIC_AUTH_PASS"
```

Clients send: `Authorization: Basic <base64(username:password)>`

## Pipeline Configuration

Add auth middleware to your pipeline:

```toml
[pipelines.secure_api]
networks = ["default"]
endpoints = ["api_endpoint"]
backends = ["api_backend"]
middleware = ["jwt_auth", "transform"]  # Auth first!
```

## Error Responses

| Scenario | HTTP Status |
|----------|-------------|
| Missing Authorization header | 401 |
| Invalid/malformed token | 401 |
| Expired token | 401 |
| Wrong issuer/audience | 401 |
| Key parsing error | 500 |
| Config error | 500 |

## Testing Authentication

### Generate test JWT (RS256)
```bash
# Generate key pair
openssl genrsa -out private.pem 2048
openssl rsa -in private.pem -pubout -out public.pem

# Create JWT (use jwt-cli or similar)
jwt encode --secret @private.pem --alg RS256 \
  '{"sub":"user","iss":"https://auth.example.com/","aud":"harmony","exp":'$(($(date +%s)+3600))'}'
```

### Test request
```bash
curl -v http://localhost:8080/api/endpoint \
  -H "Authorization: Bearer <your_jwt>"
```

### Test with invalid token
```bash
curl -v http://localhost:8080/api/endpoint \
  -H "Authorization: Bearer invalid_token"
# Should return 401
```

## Common Issues

### "No public key configured"
- Check `public_key_path` exists and is readable
- Ensure PEM format (starts with `-----BEGIN PUBLIC KEY-----`)

### "Token expired"
- Check `exp` claim in token
- Increase `leeway_secs` if clock skew is an issue

### "Invalid issuer/audience"
- Verify token's `iss` matches config `issuer`
- Verify token's `aud` matches config `audience`

### Getting 500 instead of 401
- This indicates a config error, not an auth failure
- Check logs for key parsing or config issues

## Multiple Auth Methods

You can have different auth for different endpoints by using separate pipelines:

```toml
[middleware.jwt_auth]
type = "jwt_auth"
# ...

[middleware.basic_auth]
type = "basic_auth"
# ...

[pipelines.api_pipeline]
middleware = ["jwt_auth"]
endpoints = ["api"]

[pipelines.admin_pipeline]
middleware = ["basic_auth"]
endpoints = ["admin"]
```

## Security Best Practices

1. **Use RS256 in production** - Never use HS256 with secrets that could leak
2. **Use environment variables** - Never hardcode secrets in config
3. **Rotate keys** - Have a key rotation strategy
4. **Short token lifetimes** - Use short `exp` with refresh tokens
5. **Validate issuer/audience** - Always configure these in production
6. **Use HTTPS** - Tokens in headers are visible over HTTP

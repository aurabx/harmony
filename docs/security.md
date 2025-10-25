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
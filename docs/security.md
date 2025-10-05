# Security

JWT authentication
- Prefer RS256 with an RSA public key (PEM). Do not use HS256 in production
- Enforce algorithm strictly; never accept a token signed with a different algorithm
- Validate exp, nbf, iat with minimal leeway (e.g., 60 seconds)
- Validate iss and aud where applicable
- Map all verification failures to HTTP 401 Unauthorized
- Startup safety: the middleware panics if RS256 keys are missing and HS256 is not explicitly enabled

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
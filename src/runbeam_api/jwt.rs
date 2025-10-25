use crate::runbeam_api::types::{RunbeamError, TeamInfo, UserInfo};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

/// JWT claims structure for Runbeam Cloud tokens
///
/// These claims follow the standard JWT specification plus custom claims
/// for user and team information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Issuer - the Runbeam Cloud API base URL
    pub iss: String,
    /// Subject - User or Team ID
    pub sub: String,
    /// Audience - 'runbeam-cli' or 'runbeam-api' (optional)
    #[serde(default)]
    pub aud: Option<String>,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at time (Unix timestamp)
    pub iat: i64,
    /// User information
    #[serde(default)]
    pub user: Option<UserInfo>,
    /// Team information
    #[serde(default)]
    pub team: Option<TeamInfo>,
}

impl JwtClaims {
    /// Extract the Runbeam API base URL from the issuer claim
    ///
    /// The `iss` claim may contain a full URL (e.g., "http://example.com/api/cli/check-login/xxx")
    /// This method extracts just the base URL (e.g., "http://example.com")
    pub fn api_base_url(&self) -> String {
        // Try to parse as URL and extract origin
        if let Ok(url) = url::Url::parse(&self.iss) {
            // Get scheme + host + port
            let scheme = url.scheme();
            let host = url.host_str().unwrap_or("");
            let port = url.port().map(|p| format!(":{}", p)).unwrap_or_default();
            format!("{}://{}{}", scheme, host, port)
        } else {
            // If parsing fails, return as-is
            self.iss.clone()
        }
    }

    /// Check if the token has expired
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        self.exp < now
    }
}

/// Validate a JWT token and extract claims
///
/// This function validates the JWT signature using HS256 algorithm and checks
/// the token expiry. It returns the decoded claims if validation succeeds.
///
/// # Arguments
///
/// * `token` - The JWT token string to validate
/// * `secret` - The shared secret used to sign the JWT (HS256)
///
/// # Returns
///
/// Returns `Ok(JwtClaims)` if validation succeeds, or `Err(RunbeamError)` if
/// validation fails for any reason (invalid signature, expired, malformed, etc.)
///
/// # Example
///
/// ```no_run
/// use harmony::runbeam_api::jwt::validate_jwt_token;
///
/// let token = "eyJhbGci...";
/// let secret = b"your-secret-key";
///
/// match validate_jwt_token(token, secret) {
///     Ok(claims) => {
///         println!("Token valid, API base URL: {}", claims.api_base_url());
///     }
///     Err(e) => {
///         eprintln!("Token validation failed: {}", e);
///     }
/// }
/// ```
pub fn validate_jwt_token(token: &str, secret: &[u8]) -> Result<JwtClaims, RunbeamError> {
    tracing::debug!("Validating JWT token (length: {})", token.len());

    // Create decoding key from secret
    let decoding_key = DecodingKey::from_secret(secret);

    // Configure validation
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true; // Ensure token is not expired
    validation.validate_nbf = false; // Not before is optional

    // Decode and validate the token
    let token_data = decode::<JwtClaims>(token, &decoding_key, &validation).map_err(|e| {
        tracing::error!("JWT validation failed: {}", e);
        RunbeamError::JwtValidation(format!("Token validation failed: {}", e))
    })?;

    let claims = token_data.claims;

    tracing::debug!(
        "JWT validation successful: iss={}, sub={}, aud={:?}",
        claims.iss,
        claims.sub,
        claims.aud
    );

    // Additional validation: ensure required claims are present
    if claims.iss.is_empty() {
        return Err(RunbeamError::JwtValidation(
            "Missing or empty issuer (iss) claim".to_string(),
        ));
    }

    if claims.sub.is_empty() {
        return Err(RunbeamError::JwtValidation(
            "Missing or empty subject (sub) claim".to_string(),
        ));
    }

    Ok(claims)
}

/// Extract JWT token from Authorization header
///
/// Parses the "Bearer <token>" format and returns just the token string.
///
/// # Arguments
///
/// * `auth_header` - The Authorization header value
///
/// # Returns
///
/// Returns `Ok(token)` if the header is valid, or `Err` if malformed.
pub fn extract_bearer_token(auth_header: &str) -> Result<&str, RunbeamError> {
    if !auth_header.starts_with("Bearer ") {
        return Err(RunbeamError::JwtValidation(
            "Authorization header must start with 'Bearer '".to_string(),
        ));
    }

    let token = auth_header.trim_start_matches("Bearer ").trim();
    if token.is_empty() {
        return Err(RunbeamError::JwtValidation(
            "Missing token in Authorization header".to_string(),
        ));
    }

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bearer_token_valid() {
        let header = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
        let token = extract_bearer_token(header).unwrap();
        assert_eq!(token, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test");
    }

    #[test]
    fn test_extract_bearer_token_with_whitespace() {
        let header = "Bearer   eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test   ";
        let token = extract_bearer_token(header).unwrap();
        assert_eq!(token, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test");
    }

    #[test]
    fn test_extract_bearer_token_missing_bearer() {
        let header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
        let result = extract_bearer_token(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_bearer_token_empty_token() {
        let header = "Bearer ";
        let result = extract_bearer_token(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_jwt_claims_is_expired() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let expired_claims = JwtClaims {
            iss: "http://example.com".to_string(),
            sub: "user123".to_string(),
            aud: Some("runbeam-cli".to_string()),
            exp: now - 3600, // Expired 1 hour ago
            iat: now - 7200,
            user: None,
            team: None,
        };

        assert!(expired_claims.is_expired());

        let valid_claims = JwtClaims {
            iss: "http://example.com".to_string(),
            sub: "user123".to_string(),
            aud: Some("runbeam-cli".to_string()),
            exp: now + 3600, // Expires in 1 hour
            iat: now,
            user: None,
            team: None,
        };

        assert!(!valid_claims.is_expired());
    }
}

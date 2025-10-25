use crate::runbeam_api::types::{RunbeamError, TeamInfo, UserInfo};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// JWKS (JSON Web Key Set) response structure
#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<JwkKey>,
}

/// Individual JWK (JSON Web Key) from JWKS
#[derive(Debug, Clone, Deserialize)]
pub struct JwkKey {
    /// Key type (e.g., "RSA")
    pub kty: String,
    /// Public key use (e.g., "sig" for signature)
    #[serde(rename = "use")]
    pub key_use: Option<String>,
    /// Key ID for key rotation
    pub kid: String,
    /// Algorithm (e.g., "RS256")
    pub alg: Option<String>,
    /// RSA modulus (base64url encoded)
    pub n: String,
    /// RSA exponent (base64url encoded)
    pub e: String,
}

impl JwkKey {
    /// Convert JWK to jsonwebtoken DecodingKey
    pub fn to_decoding_key(&self) -> Result<DecodingKey, RunbeamError> {
        if self.kty != "RSA" {
            return Err(RunbeamError::JwtValidation(format!(
                "Unsupported key type: {}. Only RSA is supported.",
                self.kty
            )));
        }

        // Use jsonwebtoken's built-in from_rsa_components method
        // which handles the conversion from base64url-encoded n and e
        DecodingKey::from_rsa_components(&self.n, &self.e)
            .map_err(|e| RunbeamError::JwtValidation(format!(
                "Failed to create RSA decoding key from JWK components: {}",
                e
            )))
    }
}

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

/// JWKS cache entry containing decoded keys and metadata
struct JwksCache {
    /// Map of kid -> DecodingKey
    keys: HashMap<String, DecodingKey>,
    /// When the cache was last fetched
    last_fetched: Instant,
}

impl JwksCache {
    /// Check if the cache is expired based on the configured duration
    fn is_expired(&self, cache_duration: Duration) -> bool {
        self.last_fetched.elapsed() > cache_duration
    }
}

/// Global JWKS cache manager (one per issuer)
static JWKS_CACHE: Lazy<Arc<RwLock<HashMap<String, JwksCache>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Get a decoding key from cache or fetch from JWKS endpoint
///
/// This function implements the cache logic:
/// 1. Check if we have a cached key for this issuer+kid
/// 2. If cache is expired or missing, fetch new JWKS
/// 3. Update cache with all keys from JWKS
/// 4. Return the requested key
async fn get_decoding_key(
    issuer: &str,
    kid: &str,
    cache_duration: Duration,
) -> Result<DecodingKey, RunbeamError> {
    // Try to get from cache first (read lock)
    {
        let cache = JWKS_CACHE.read().map_err(|e| {
            RunbeamError::JwtValidation(format!("Cache lock poisoned: {}", e))
        })?;

        if let Some(cache_entry) = cache.get(issuer) {
            if !cache_entry.is_expired(cache_duration) {
                if let Some(key) = cache_entry.keys.get(kid) {
                    tracing::debug!("JWKS cache hit for issuer={}, kid={}", issuer, kid);
                    return Ok(key.clone());
                } else {
                    tracing::debug!("JWKS cache miss: kid '{}' not found in cached keys", kid);
                }
            } else {
                tracing::debug!("JWKS cache expired for issuer={}", issuer);
            }
        } else {
            tracing::debug!("JWKS cache miss for issuer={}", issuer);
        }
    }

    // Double-check with write lock: another thread might have updated the cache
    {
        let cache = JWKS_CACHE.write().map_err(|e| {
            RunbeamError::JwtValidation(format!("Cache lock poisoned: {}", e))
        })?;

        if let Some(cache_entry) = cache.get(issuer) {
            if !cache_entry.is_expired(cache_duration) {
                if let Some(key) = cache_entry.keys.get(kid) {
                    tracing::debug!("JWKS cache hit after lock acquisition for issuer={}, kid={}", issuer, kid);
                    return Ok(key.clone());
                }
            }
        }
        // Lock is released here before await
    }

    // Fetch fresh JWKS (no lock held)
    tracing::info!("Fetching fresh JWKS for issuer={}", issuer);
    let jwks = fetch_jwks(issuer).await?;

    // Convert all keys to DecodingKeys
    let mut keys_map = HashMap::new();
    for jwk in &jwks.keys {
        match jwk.to_decoding_key() {
            Ok(key) => {
                keys_map.insert(jwk.kid.clone(), key);
            }
            Err(e) => {
                tracing::warn!("Failed to convert JWK kid='{}' to decoding key: {}", jwk.kid, e);
                // Continue processing other keys
            }
        }
    }

    // Find the requested key
    let decoding_key = keys_map.get(kid)
        .ok_or_else(|| RunbeamError::JwtValidation(format!(
            "Key ID '{}' not found in JWKS from issuer {}",
            kid, issuer
        )))?
        .clone();

    // Update cache (acquire write lock again)
    {
        let mut cache = JWKS_CACHE.write().map_err(|e| {
            RunbeamError::JwtValidation(format!("Cache lock poisoned: {}", e))
        })?;

        cache.insert(issuer.to_string(), JwksCache {
            keys: keys_map,
            last_fetched: Instant::now(),
        });
    }

    tracing::debug!("JWKS cache updated for issuer={}", issuer);
    Ok(decoding_key)
}

/// Clear JWKS cache for a specific issuer
///
/// Used when token validation fails to force a cache refresh
fn clear_jwks_cache(issuer: &str) -> Result<(), RunbeamError> {
    let mut cache = JWKS_CACHE.write().map_err(|e| {
        RunbeamError::JwtValidation(format!("Cache lock poisoned: {}", e))
    })?;

    if cache.remove(issuer).is_some() {
        tracing::debug!("Cleared JWKS cache for issuer={}", issuer);
    }
    Ok(())
}

/// Fetch JWKS from the issuer's well-known endpoint
///
/// Constructs the JWKS URL from the issuer and fetches the key set.
/// Returns error if the endpoint is unreachable or returns invalid data.
async fn fetch_jwks(issuer: &str) -> Result<Jwks, RunbeamError> {
    // JWKS endpoint is at /api/.well-known/jwks.json
    let jwks_url = format!("{}/api/.well-known/jwks.json", issuer.trim_end_matches('/'));

    tracing::debug!("Fetching JWKS from: {}", jwks_url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| RunbeamError::JwtValidation(format!("Failed to create HTTP client: {}", e)))?;

    let response = client
        .get(&jwks_url)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch JWKS from {}: {}", jwks_url, e);
            if e.is_timeout() {
                RunbeamError::JwtValidation(format!("JWKS endpoint timeout: {}", jwks_url))
            } else if e.is_connect() {
                RunbeamError::JwtValidation(format!("Failed to connect to JWKS endpoint: {}", jwks_url))
            } else {
                RunbeamError::JwtValidation(format!("Network error fetching JWKS: {}", e))
            }
        })?;

    let status = response.status();
    if !status.is_success() {
        tracing::error!("JWKS endpoint returned HTTP {}: {}", status.as_u16(), jwks_url);
        return Err(RunbeamError::JwtValidation(format!(
            "JWKS endpoint returned HTTP {}",
            status.as_u16()
        )));
    }

    let jwks = response
        .json::<Jwks>()
        .await
        .map_err(|e| {
            tracing::error!("Failed to parse JWKS response from {}: {}", jwks_url, e);
            RunbeamError::JwtValidation(format!("Invalid JWKS response: {}", e))
        })?;

    tracing::info!("Successfully fetched JWKS with {} keys from {}", jwks.keys.len(), jwks_url);
    Ok(jwks)
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

/// Validate a JWT token and extract claims using RS256
///
/// This function validates JWT signature using RS256 algorithm with public keys
/// fetched from the issuer's JWKS endpoint. It implements caching and automatic
/// refresh on validation failures.
///
/// # Arguments
///
/// * `token` - The JWT token string to validate
/// * `jwks_cache_duration_hours` - Duration in hours to cache JWKS keys
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
/// # tokio_test::block_on(async {
/// let token = "eyJhbGci...";
/// let cache_duration = 24; // 24 hours
///
/// match validate_jwt_token(token, cache_duration).await {
///     Ok(claims) => {
///         println!("Token valid, API base URL: {}", claims.api_base_url());
///     }
///     Err(e) => {
///         eprintln!("Token validation failed: {}", e);
///     }
/// }
/// # });
/// ```
pub async fn validate_jwt_token(
    token: &str,
    jwks_cache_duration_hours: u64,
) -> Result<JwtClaims, RunbeamError> {
    tracing::debug!("Validating JWT token (length: {})", token.len());

    // Step 1: Decode header to extract kid
    let header = decode_header(token)
        .map_err(|e| RunbeamError::JwtValidation(format!("Invalid JWT header: {}", e)))?;

    let kid = header.kid
        .ok_or_else(|| RunbeamError::JwtValidation(
            "Missing 'kid' (key ID) in JWT header".to_string()
        ))?;

    // Verify algorithm is RS256
    if header.alg != Algorithm::RS256 {
        return Err(RunbeamError::JwtValidation(format!(
            "Unsupported algorithm: {:?}. Only RS256 is supported.",
            header.alg
        )));
    }

    tracing::debug!("JWT header decoded: alg={:?}, kid={}", header.alg, kid);

    // Step 2: Decode without validation to extract issuer
    // We need to extract iss claim, so do a partial decode with no validation
    let mut validation_no_sig = Validation::new(Algorithm::RS256);
    validation_no_sig.insecure_disable_signature_validation();
    validation_no_sig.validate_exp = false;

    // Use a dummy decoding key (signature not validated)
    let dummy_key = DecodingKey::from_secret(b"dummy");
    let insecure_token_data = decode::<JwtClaims>(token, &dummy_key, &validation_no_sig)
        .map_err(|e| RunbeamError::JwtValidation(format!("Failed to decode JWT: {}", e)))?;

    let issuer = &insecure_token_data.claims.iss;
    if issuer.is_empty() {
        return Err(RunbeamError::JwtValidation(
            "Missing or empty issuer (iss) claim".to_string(),
        ));
    }

    tracing::debug!("JWT issuer extracted: {}", issuer);

    // Extract base URL (scheme + host + port) from issuer for JWKS lookup
    // The issuer might be a full URL like "http://example.com/api/cli/check-login/xxx"
    // but we only need "http://example.com" to construct the JWKS endpoint
    let base_url = insecure_token_data.claims.api_base_url();
    tracing::debug!("JWT issuer base URL: {}", base_url);

    // Step 3: Get decoding key from cache or JWKS
    let cache_duration = Duration::from_secs(jwks_cache_duration_hours * 3600);
    let decoding_key = match get_decoding_key(&base_url, &kid, cache_duration).await {
        Ok(key) => key,
        Err(e) => {
            tracing::warn!("Initial JWKS fetch/cache lookup failed: {}", e);
            return Err(e);
        }
    };

    // Step 4: Configure validation
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = true; // Ensure token is not expired
    validation.validate_nbf = false; // Not before is optional

    // Step 5: Validate token
    let validation_result = decode::<JwtClaims>(token, &decoding_key, &validation);

    let claims = match validation_result {
        Ok(token_data) => token_data.claims,
        Err(e) => {
            // Retry logic: if validation fails, clear cache and retry once
            tracing::warn!("JWT validation failed, attempting cache refresh: {}", e);

            // Clear cache for this issuer (use base URL)
            if let Err(clear_err) = clear_jwks_cache(&base_url) {
                tracing::error!("Failed to clear JWKS cache: {}", clear_err);
            }

            // Fetch fresh JWKS and retry
            let fresh_key = get_decoding_key(&base_url, &kid, cache_duration).await
                .map_err(|refresh_err| {
                    tracing::error!("Failed to refresh JWKS: {}", refresh_err);
                    RunbeamError::JwtValidation(format!(
                        "Token validation failed and refresh failed: {}. Original error: {}",
                        refresh_err, e
                    ))
                })?;

            // Retry validation with fresh key
            decode::<JwtClaims>(token, &fresh_key, &validation)
                .map_err(|retry_err| {
                    tracing::error!("JWT validation failed after refresh: {}", retry_err);
                    RunbeamError::JwtValidation(format!("Token validation failed: {}", retry_err))
                })?
                .claims
        }
    };

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

use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::AuthFailure;
use crate::utils::Error;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::slice::from_ref;
use std::sync::Arc;

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct JwtAuthConfig {
    pub public_key_path: String, // Path to the public key file (RS256) or unused for HS256
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub leeway_secs: Option<u64>,
    #[serde(default)]
    pub use_hs256: bool, // When true, use HS256 with hs256_secret
    #[serde(default)]
    pub hs256_secret: Option<String>, // Required if use_hs256 = true
}

pub struct JwtAuthMiddleware {
    pub config: JwtAuthConfig,          // Configuration for the middleware
    pub decoding_key: Arc<DecodingKey>, // Decoding key for JWT
    pub algorithm: Algorithm,           // Expected algorithm
}

#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: Option<String>,
    #[allow(dead_code)]
    exp: Option<i64>,
    #[allow(dead_code)]
    nbf: Option<i64>,
    #[allow(dead_code)]
    iat: Option<i64>,
    #[allow(dead_code)]
    iss: Option<String>,
    #[allow(dead_code)]
    aud: Option<JsonValue>, // string or array
}

pub fn parse_config(
    options: &std::collections::HashMap<String, JsonValue>,
) -> Result<JwtAuthConfig, String> {
    let public_key_path = options
        .get("public_key_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let issuer = options
        .get("issuer")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let audience = options
        .get("audience")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let leeway_secs = options.get("leeway_secs").and_then(|v| v.as_u64());
    let use_hs256 = options
        .get("use_hs256")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let hs256_secret = options
        .get("hs256_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(JwtAuthConfig {
        public_key_path,
        issuer,
        audience,
        leeway_secs,
        use_hs256,
        hs256_secret,
    })
}

impl JwtAuthMiddleware {
    pub fn new(config: JwtAuthConfig) -> Self {
        // Try to load an RSA public key; if unavailable, fall back to a local secret for tests/dev.
        // Choose algorithm based on config
        let (decoding_key, algorithm) = if config.use_hs256 {
            let secret = config
                .hs256_secret
                .as_ref()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| b"test-fallback-secret".to_vec());
            (DecodingKey::from_secret(&secret), Algorithm::HS256)
        } else {
            // Strict RS256 path: must load a valid RSA public key
            match std::fs::read_to_string(&config.public_key_path) {
                Ok(pem) => match DecodingKey::from_rsa_pem(pem.as_bytes()) {
                    Ok(k) => (k, Algorithm::RS256),
                    Err(e) => {
                        panic!(
                                "JWT: failed to parse RSA public key at '{}': {}. Set use_hs256=true with a secret for HS256.",
                                config.public_key_path,
                                e
                            );
                    }
                },
                Err(e) => {
                    panic!(
                        "JWT: failed to read RSA public key at '{}': {}. Set use_hs256=true with a secret for HS256.",
                        config.public_key_path,
                        e
                    );
                }
            }
        };

        Self {
            config,
            decoding_key: Arc::new(decoding_key), // Use Arc to avoid reloading
            algorithm,
        }
    }

    /// Real token validation: verify signature and claims
    async fn validate_token(&self, token: &str) -> Result<bool, Error> {
        // Enforce expected algorithm from header
        let header = decode_header(token).map_err(|_| AuthFailure("invalid JWT header"))?;
        if header.alg != self.algorithm {
            return Err(AuthFailure("unexpected JWT alg").into());
        }

        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.leeway = self.config.leeway_secs.unwrap_or(60);

        if let Some(ref iss) = self.config.issuer {
            validation.set_issuer(from_ref(iss));
        }
        if let Some(ref aud) = self.config.audience {
            validation.set_audience(from_ref(aud));
        }

        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|_| AuthFailure("jwt verify failed"))?;

        // Additional audience check if aud is array or string
        if let (Some(expected), Some(aud_val)) = (&self.config.audience, &token_data.claims.aud) {
            match aud_val {
                JsonValue::String(s) if s == expected => {}
                JsonValue::Array(arr) if arr.iter().any(|v| v == expected) => {}
                _ => return Err(AuthFailure("jwt verify failed").into()),
            }
        }

        Ok(true)
    }

    /// Extract JWT token from Authorization header in the envelope
    fn extract_token_from_envelope(
        &self,
        envelope: &RequestEnvelope<serde_json::Value>,
    ) -> Result<String, Error> {
        if let Some(auth_header) = envelope.request_details.headers.get("authorization") {
            if auth_header.starts_with("Bearer ") {
                Ok(auth_header.trim_start_matches("Bearer ").to_string())
            } else {
                Err(AuthFailure("Authorization header must start with 'Bearer '").into())
            }
        } else {
            Err(AuthFailure("Missing Authorization header").into())
        }
    }
}

#[async_trait::async_trait]
impl Middleware for JwtAuthMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Step 1: Extract the JWT token from the envelope's headers
        let token = match self.extract_token_from_envelope(&envelope) {
            Ok(token) => token,
            Err(err) => {
                tracing::error!("JWT Auth failed: {}", err);
                return Err(err);
            }
        };

        // Step 2: Validate the token
        if !self.validate_token(&token).await? {
            let error_message = "Invalid or expired JWT token";
            tracing::error!("{}", error_message);
            return Err(AuthFailure("jwt verify failed").into());
        }

        tracing::info!("JWT token validated successfully");

        // Step 3: Pass through the envelope (token is valid)
        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // For JWT auth, typically no processing is needed on the right side
        // Just pass through the response envelope
        tracing::debug!("JWT Auth middleware processing response (right) - passthrough");
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use rand::thread_rng;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use serde::Serialize;
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[derive(Serialize)]
    struct TestClaims {
        sub: Option<String>,
        iss: Option<String>,
        aud: Option<String>,
        exp: i64,
        iat: i64,
        nbf: Option<i64>,
    }

    fn create_test_envelope_with_auth(auth_header: Option<&str>) -> RequestEnvelope<JsonValue> {
        let mut headers = HashMap::new();
        if let Some(auth) = auth_header {
            headers.insert("authorization".to_string(), auth.to_string());
        }

        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/test".to_string(),
            headers,
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
        };
        let backend_request_details = request_details.clone();

        RequestEnvelope {
            request_details,
            backend_request_details,
            original_data: serde_json::json!({}),
            normalized_data: Some(serde_json::json!({"test": "data"})),
            normalized_snapshot: None,
        }
    }

    fn get_current_timestamp() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn test_parse_config_hs256() {
        let mut options = HashMap::new();
        options.insert("use_hs256".to_string(), JsonValue::Bool(true));
        options.insert(
            "hs256_secret".to_string(),
            JsonValue::String("my-secret".to_string()),
        );
        options.insert(
            "issuer".to_string(),
            JsonValue::String("test-issuer".to_string()),
        );
        options.insert(
            "audience".to_string(),
            JsonValue::String("test-audience".to_string()),
        );
        options.insert("leeway_secs".to_string(), JsonValue::Number(30.into()));

        let config = parse_config(&options).unwrap();
        assert!(config.use_hs256);
        assert_eq!(config.hs256_secret, Some("my-secret".to_string()));
        assert_eq!(config.issuer, Some("test-issuer".to_string()));
        assert_eq!(config.audience, Some("test-audience".to_string()));
        assert_eq!(config.leeway_secs, Some(30));
    }

    #[test]
    fn test_parse_config_rs256() {
        let mut options = HashMap::new();
        options.insert(
            "public_key_path".to_string(),
            JsonValue::String("/path/to/key.pem".to_string()),
        );
        options.insert("use_hs256".to_string(), JsonValue::Bool(false));
        options.insert(
            "issuer".to_string(),
            JsonValue::String("test-issuer".to_string()),
        );

        let config = parse_config(&options).unwrap();
        assert!(!config.use_hs256);
        assert_eq!(config.public_key_path, "/path/to/key.pem");
        assert_eq!(config.issuer, Some("test-issuer".to_string()));
        assert_eq!(config.hs256_secret, None);
    }

    #[test]
    fn test_parse_config_defaults() {
        let options = HashMap::new();
        let config = parse_config(&options).unwrap();

        assert!(!config.use_hs256);
        assert_eq!(config.public_key_path, "");
        assert_eq!(config.issuer, None);
        assert_eq!(config.audience, None);
        assert_eq!(config.leeway_secs, None);
        assert_eq!(config.hs256_secret, None);
    }

    #[test]
    fn test_jwt_auth_middleware_new_hs256() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: Some("test-issuer".to_string()),
            audience: Some("test-audience".to_string()),
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };

        let middleware = JwtAuthMiddleware::new(config);
        assert_eq!(middleware.algorithm, Algorithm::HS256);
        assert_eq!(middleware.config.issuer, Some("test-issuer".to_string()));
        assert_eq!(
            middleware.config.audience,
            Some("test-audience".to_string())
        );
    }

    #[test]
    fn test_jwt_auth_middleware_new_hs256_fallback_secret() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: true,
            hs256_secret: None, // Should use fallback secret
        };

        let middleware = JwtAuthMiddleware::new(config);
        assert_eq!(middleware.algorithm, Algorithm::HS256);
    }

    #[test]
    fn test_extract_token_from_envelope_success() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let envelope = create_test_envelope_with_auth(Some("Bearer my-jwt-token"));
        let token = middleware.extract_token_from_envelope(&envelope).unwrap();
        assert_eq!(token, "my-jwt-token");
    }

    #[test]
    fn test_extract_token_from_envelope_missing_header() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let envelope = create_test_envelope_with_auth(None);
        let result = middleware.extract_token_from_envelope(&envelope);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing Authorization header"));
    }

    #[test]
    fn test_extract_token_from_envelope_invalid_format() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let envelope = create_test_envelope_with_auth(Some("Basic dXNlcjpwYXNz"));
        let result = middleware.extract_token_from_envelope(&envelope);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Authorization header must start with 'Bearer '"));
    }

    #[tokio::test]
    async fn test_validate_token_success_hs256() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: Some("test-issuer".to_string()),
            audience: Some("test-audience".to_string()),
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: Some("test-issuer".to_string()),
            aud: Some("test-audience".to_string()),
            exp: now + 3600, // 1 hour from now
            iat: now,
            nbf: Some(now),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = middleware.validate_token(&token).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_validate_token_expired() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: None,
            aud: None,
            exp: now - 3600, // 1 hour ago (expired)
            iat: now - 7200, // 2 hours ago
            nbf: None,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = middleware.validate_token(&token).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("jwt verify failed"));
    }

    #[tokio::test]
    async fn test_validate_token_wrong_issuer() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: Some("expected-issuer".to_string()),
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: Some("wrong-issuer".to_string()), // Wrong issuer
            aud: None,
            exp: now + 3600,
            iat: now,
            nbf: None,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = middleware.validate_token(&token).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("jwt verify failed"));
    }

    #[tokio::test]
    async fn test_validate_token_wrong_audience_string() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: Some("expected-audience".to_string()),
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: None,
            aud: Some("wrong-audience".to_string()), // Wrong audience
            exp: now + 3600,
            iat: now,
            nbf: None,
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = middleware.validate_token(&token).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("jwt verify failed"));
    }

    #[tokio::test]
    async fn test_validate_token_wrong_algorithm() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true, // Expecting HS256
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: None,
            aud: None,
            exp: now + 3600,
            iat: now,
            nbf: None,
        };

        // Create token with HS512 algorithm instead of HS256
        let token = encode(
            &Header::new(Algorithm::HS512),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let result = middleware.validate_token(&token).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unexpected JWT alg"));
    }

    #[tokio::test]
    async fn test_validate_token_invalid_format() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let result = middleware.validate_token("invalid-jwt-token").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid JWT header"));
    }

    #[tokio::test]
    async fn test_left_middleware_success() {
        let secret = "test-secret-key";
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: Some("test-issuer".to_string()),
            audience: Some("test-audience".to_string()),
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some(secret.to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let now = get_current_timestamp();
        let claims = TestClaims {
            sub: Some("test-user".to_string()),
            iss: Some("test-issuer".to_string()),
            aud: Some("test-audience".to_string()),
            exp: now + 3600,
            iat: now,
            nbf: Some(now),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let envelope = create_test_envelope_with_auth(Some(&format!("Bearer {}", token)));
        let result = middleware.left(envelope).await;

        assert!(result.is_ok());
        let returned_envelope = result.unwrap();
        assert_eq!(returned_envelope.request_details.method, "GET");
        assert_eq!(returned_envelope.request_details.uri, "/test");
    }

    #[tokio::test]
    async fn test_left_middleware_missing_token() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let envelope = create_test_envelope_with_auth(None);
        let result = middleware.left(envelope).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing Authorization header"));
    }

    #[tokio::test]
    async fn test_left_middleware_invalid_token() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let envelope = create_test_envelope_with_auth(Some("Bearer invalid-token"));
        let result = middleware.left(envelope).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid JWT header"));
    }

    #[tokio::test]
    async fn test_right_middleware_passthrough() {
        let config = JwtAuthConfig {
            public_key_path: "".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: Some(60),
            use_hs256: true,
            hs256_secret: Some("test-secret".to_string()),
        };
        let middleware = JwtAuthMiddleware::new(config);

        let request_env = create_test_envelope_with_auth(None);
        let original_method = request_env.request_details.method.clone();
        let original_uri = request_env.request_details.uri.clone();

        // Convert to ResponseEnvelope for right() method
        let envelope = crate::models::envelope::envelope::ResponseEnvelope {
            request_details: request_env.request_details,
            response_details: crate::models::envelope::envelope::ResponseDetails {
                status: 200,
                headers: HashMap::new(),
                metadata: HashMap::new(),
            },
            original_data: request_env.original_data,
            normalized_data: request_env.normalized_data,
            normalized_snapshot: request_env.normalized_snapshot,
        };

        let result = middleware.right(envelope).await;

        assert!(result.is_ok());
        let returned_envelope = result.unwrap();
        assert_eq!(returned_envelope.request_details.method, original_method);
        assert_eq!(returned_envelope.request_details.uri, original_uri);
    }

    #[test]
    fn test_jwt_auth_middleware_new_rs256_with_valid_key() {
        // Generate a temporary RSA key pair for testing
        let mut rng = thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let public_pem = public_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();

        // Write public key to a temporary file
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(public_pem.as_bytes()).unwrap();
        let temp_path = temp_file.path().to_string_lossy().to_string();

        let config = JwtAuthConfig {
            public_key_path: temp_path,
            issuer: Some("test-issuer".to_string()),
            audience: Some("test-audience".to_string()),
            leeway_secs: Some(60),
            use_hs256: false, // RS256 mode
            hs256_secret: None,
        };

        let middleware = JwtAuthMiddleware::new(config);
        assert_eq!(middleware.algorithm, Algorithm::RS256);
    }

    #[test]
    #[should_panic(expected = "JWT: failed to read RSA public key")]
    fn test_jwt_auth_middleware_new_rs256_missing_key() {
        let config = JwtAuthConfig {
            public_key_path: "/nonexistent/path/key.pem".to_string(),
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: false,
            hs256_secret: None,
        };

        JwtAuthMiddleware::new(config);
    }

    #[test]
    #[should_panic(expected = "JWT: failed to parse RSA public key")]
    fn test_jwt_auth_middleware_new_rs256_invalid_key() {
        // Write invalid PEM content to temporary file
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"invalid pem content").unwrap();
        let temp_path = temp_file.path().to_string_lossy().to_string();

        let config = JwtAuthConfig {
            public_key_path: temp_path,
            issuer: None,
            audience: None,
            leeway_secs: None,
            use_hs256: false,
            hs256_secret: None,
        };

        JwtAuthMiddleware::new(config);
    }
}

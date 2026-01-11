use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{engine::general_purpose, Engine as _};
use harmony::config::config::{Config, ConfigError};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use std::sync::Arc;
use tower::ServiceExt; // for Router::oneshot

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn basic_header(user: &str, pass: &str) -> String {
    let creds = format!("{}:{}", user, pass);
    format!(
        "Basic {}",
        general_purpose::STANDARD.encode(creds.as_bytes())
    )
}

#[tokio::test]
async fn jwt_auth_with_authentication_reference() {
    let toml = r#"
        [proxy]
        id = "jwt-auth-ref-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Global authentication definition
        [authentications.test-jwt]
        id = "test-jwt"
        method = "jwt"
        [authentications.test-jwt.options]
        use_hs256 = true
        hs256_secret = "test-secret-key"
        issuer = "https://auth.example.com/"
        audience = "test-api"
        leeway_secs = 60
        public_key_path = ""

        [pipelines.core]
        description = "HTTP->Echo with JWT auth via reference"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["jwt_middleware"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/api"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Middleware references authentication
        [middleware.jwt_middleware]
        type = "jwt_auth"
        authentication = "test-jwt"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Generate valid JWT
    #[derive(Serialize)]
    struct TestClaims {
        iss: String,
        aud: String,
        exp: i64,
        iat: i64,
    }
    let now = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()) as i64;
    let claims = TestClaims {
        iss: "https://auth.example.com/".to_string(),
        aud: "test-api".to_string(),
        exp: now + 600,
        iat: now - 10,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(b"test-secret-key"),
    )
    .expect("encode jwt");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/test")
                .method("GET")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn basic_auth_with_authentication_reference() {
    let toml = r#"
        [proxy]
        id = "basic-auth-ref-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Global authentication definition
        [authentications.test-basic]
        id = "test-basic"
        method = "basic"
        [authentications.test-basic.options]
        username = "testuser"
        password = "testpass"
        token_path = ""

        [pipelines.core]
        description = "HTTP->Echo with Basic auth via reference"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["basic_middleware"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/secure"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Middleware references authentication
        [middleware.basic_middleware]
        type = "basic_auth"
        authentication = "test-basic"
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Valid credentials
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/secure/data")
                .method("GET")
                .header("Authorization", basic_header("testuser", "testpass"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    // Invalid credentials
    let response = app
        .oneshot(
            Request::builder()
                .uri("/secure/data")
                .method("GET")
                .header("Authorization", basic_header("testuser", "wrongpass"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn missing_authentication_reference_fails_validation() {
    let toml = r#"
        [proxy]
        id = "missing-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["jwt_middleware"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/test"

        [backends.echo_backend]
        service = "echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Middleware references non-existent authentication
        [middleware.jwt_middleware]
        type = "jwt_auth"
        authentication = "nonexistent"
    "#;

    let config: Config = toml::from_str(toml).expect("TOML parse should succeed");
    
    // Config validation should succeed (it's a reference check, not a parse error)
    // The error happens during middleware building
    if let Err(e) = config.validate() {
        panic!("Validation failed (should pass for missing auth ref): {:?}", e);
    }
    
    // Try to build middleware - this should fail
    let result = harmony::models::middleware::middleware::build_middleware_instances_for_pipeline(
        &["jwt_middleware".to_string()],
        &config,
    );
    
    match result {
        Ok(_) => panic!("Expected error for missing authentication reference"),
        Err(err) => {
            assert!(err.contains("unknown authentication"), "Error message should mention 'unknown authentication': {}", err);
            assert!(err.contains("nonexistent"), "Error message should mention the missing auth ID: {}", err);
        }
    }
}

#[test]
fn multiple_middlewares_can_share_authentication() {
    let toml = r#"
        [proxy]
        id = "shared-auth-test"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        # Shared authentication definition
        [authentications.shared-jwt]
        id = "shared-jwt"
        method = "jwt"
        [authentications.shared-jwt.options]
        use_hs256 = true
        hs256_secret = "shared-secret"
        issuer = "https://auth.example.com/"
        audience = "api"
        public_key_path = ""

        [pipelines.api]
        networks = ["default"]
        endpoints = ["api_v1"]
        backends = ["backend1"]
        middleware = ["auth1"]

        [pipelines.admin]
        networks = ["default"]
        endpoints = ["api_v2"]
        backends = ["backend2"]
        middleware = ["auth2"]

        [endpoints.api_v1]
        service = "http"
        [endpoints.api_v1.options]
        path_prefix = "/v1"

        [endpoints.api_v2]
        service = "http"
        [endpoints.api_v2.options]
        path_prefix = "/v2"

        [backends.backend1]
        service = "echo"

        [backends.backend2]
        service = "echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Both middlewares reference the same authentication
        [middleware.auth1]
        type = "jwt_auth"
        authentication = "shared-jwt"

        [middleware.auth2]
        type = "jwt_auth"
        authentication = "shared-jwt"
    "#;

    let config: Config = toml::from_str(toml).expect("TOML parse should succeed");
    assert!(config.validate().is_ok());

    // Both middleware instances should build successfully using the same authentication
    let result1 = harmony::models::middleware::middleware::build_middleware_instances_for_pipeline(
        &["auth1".to_string()],
        &config,
    );
    if let Err(e) = &result1 {
        panic!("Failed to build auth1 middleware: {}", e);
    }

    let result2 = harmony::models::middleware::middleware::build_middleware_instances_for_pipeline(
        &["auth2".to_string()],
        &config,
    );
    if let Err(e) = &result2 {
        panic!("Failed to build auth2 middleware: {}", e);
    }
}

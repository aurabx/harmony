use axum::body::Body;
use axum::http::{Request, StatusCode};
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

#[tokio::test]
async fn jwt_auth_allows_valid_bearer() {
    let toml = r#"
        [proxy]
        id = "jwt-auth-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with jwt auth"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["middleware.jwt_auth"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/jwt"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        # Built-in registry entry so resolver accepts jwtauth
        [middleware_types.jwtauth]
        module = ""

        # Middleware options
        [middleware.jwt_auth]
        use_hs256 = true
        hs256_secret = "test-fallback-secret"
        issuer = "https://test-issuer/"
        audience = "harmony"
        leeway_secs = 60
        public_key_path = "" # unused in HS256 mode
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Generate HS256 JWT compatible with middleware fallback secret
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
        iss: "https://test-issuer/".to_string(),
        aud: "harmony".to_string(),
        exp: now + 600,
        iat: now - 10,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(b"test-fallback-secret"),
    )
    .expect("encode jwt");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/jwt/get-route")
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
async fn jwt_auth_rejects_invalid_bearer() {
    let toml = r#"
        [proxy]
        id = "jwt-auth-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with jwt auth"
        networks = ["default"]
        endpoints = ["http_in"]
        backends = ["echo_backend"]
        middleware = ["middleware.jwt_auth"]

        [endpoints.http_in]
        service = "http"
        [endpoints.http_in.options]
        path_prefix = "/jwt"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.jwtauth]
        module = ""

        [middleware.jwt_auth]
        use_hs256 = true
        hs256_secret = "test-fallback-secret"
        issuer = "https://test-issuer/"
        audience = "harmony"
        leeway_secs = 60
        public_key_path = ""
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Invalid token
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/jwt/get-route")
                .method("GET")
                .header("Authorization", "Bearer nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Missing header
    let response = app
        .oneshot(
            Request::builder()
                .uri("/jwt/get-route")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

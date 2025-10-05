use harmony::config::config::{Config, ConfigError};
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt; // for Router::oneshot
use std::sync::Arc;
use std::fs;
use std::path::Path;
use jsonwebtoken::{encode, Header, EncodingKey, Algorithm};
use serde::Serialize;
use rand::thread_rng;
use rsa::{RsaPrivateKey, RsaPublicKey};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn ensure_tmp() {
    let _ = fs::create_dir_all("tmp");
}

fn write_tmp_file(name: &str, contents: &str) {
    ensure_tmp();
    fs::write(Path::new("tmp").join(name), contents).expect("write tmp file");
}

fn generate_rs256_keypair() -> (String, String) {
    let mut rng = thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate rsa key");
    let pub_key = RsaPublicKey::from(&priv_key);
    let priv_pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).expect("pkcs8 pem").to_string();
    let pub_pem = pub_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF).expect("pub pem");
    (priv_pem, pub_pem)
}

#[tokio::test]
async fn jwt_rs256_allows_valid_signature() {
    // Write the public key to tmp so middleware can read it
    let (priv_pem, pub_pem) = generate_rs256_keypair();
    write_tmp_file("rs256_pub.pem", &pub_pem);

    let toml = r#"
        [proxy]
        id = "jwt-rs256-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with RS256 jwt auth"
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
        public_key_path = "tmp/rs256_pub.pem"
        issuer = "https://test-issuer/"
        audience = "harmony"
        leeway_secs = 60
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Sign a token with the matching private key using RS256
    #[derive(Serialize)]
    struct Claims { iss: String, aud: String, exp: i64, iat: i64 }
    let now = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()) as i64;
    let claims = Claims { iss: "https://test-issuer/".to_string(), aud: "harmony".to_string(), exp: now + 600, iat: now - 10 };
    let token = encode(
        &Header::new(Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("load priv pem"),
    ).expect("encode jwt");

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
async fn jwt_rs256_rejects_wrong_alg() {
    // Use same RS256 configuration (public key present)
    let (_priv_pem, pub_pem) = generate_rs256_keypair();
    write_tmp_file("rs256_pub.pem", &pub_pem);

    let toml = r#"
        [proxy]
        id = "jwt-rs256-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.core]
        description = "HTTP->Echo with RS256 jwt auth"
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
        public_key_path = "tmp/rs256_pub.pem"
        issuer = "https://test-issuer/"
        audience = "harmony"
        leeway_secs = 60
    "#;

    let cfg = load_config_from_str(toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Create an HS256 token (wrong alg for RS256 mode)
    #[derive(Serialize)]
    struct Claims { iss: String, aud: String, exp: i64, iat: i64 }
    let now = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()) as i64;
    let claims = Claims { iss: "https://test-issuer/".to_string(), aud: "harmony".to_string(), exp: now + 600, iat: now - 10 };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(b"wrong-secret"),
    ).expect("encode jwt");

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

    // Expect 401 because header alg != expected RS256
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
//! HTTP endpoints whose paths overlap on one network.
//!
//! Every HTTP endpoint also serves the paths below its `path_prefix`. A REST
//! API such as UCP checkout has a collection endpoint and item endpoints
//! below it, so the prefixes overlap. The router used to hand every route to
//! axum, whose path matcher rejects a `{param}` and a catch-all at the same
//! position, and Harmony panicked at startup. Overlapping prefixes now route
//! to the endpoint whose path matches most specifically. The method never
//! moves a request to a less specific endpoint: if the chosen endpoint does
//! not allow it, the answer is 405. Two endpoints with the same path shape
//! fail config load instead.
//! Linear: AURA-2443.

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, Uri};
use axum::routing::any;
use axum::Router;
use harmony::config::config::{Config, ConfigError};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tower::ServiceExt;

struct Upstream {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, String)>>>,
    _handle: tokio::task::JoinHandle<()>,
}

impl Upstream {
    /// Method and path of the last request the upstream received.
    fn last(&self) -> Option<(String, String)> {
        self.seen.lock().unwrap().last().cloned()
    }
}

async fn start_recording_upstream() -> Upstream {
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handler = move |method: Method, uri: Uri, _body: Bytes| {
        let sink = sink.clone();
        async move {
            sink.lock()
                .unwrap()
                .push((method.to_string(), uri.path().to_string()));
            ([("content-type", "application/json")], "{\"ok\":true}")
        }
    };
    let app = Router::new()
        .route("/", any(handler.clone()))
        .route("/{*path}", any(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Upstream {
        base_url,
        seen,
        _handle: handle,
    }
}

/// One pipeline per endpoint, as a user splits them across pipeline files.
/// Each backend adds its own path prefix, so the upstream path shows which
/// endpoint handled the request.
fn pipeline_toml(name: &str, path_prefix: &str, base_url: &str) -> String {
    format!(
        r#"
        [pipelines.{name}]
        networks = ["default"]
        endpoints = ["{name}_in"]
        backends = ["{name}_out"]
        middleware = []

        [endpoints.{name}_in]
        service = "http"
        [endpoints.{name}_in.options]
        path_prefix = "{path_prefix}"

        [backends.{name}_out]
        service = "http"
        [backends.{name}_out.options]
        base_url = "{base_url}"
        path_prefix = "/{name}"
        "#
    )
}

const MAIN_TOML: &str = r#"
    [proxy]
    id = "http-overlapping-routes-test"
    pipelines_path = "pipelines"

    [network.default]
    enable_wireguard = false
    interface = "wg0"

    [network.default.http]
    bind_address = "127.0.0.1"
    bind_port = 8080

    [management]
    enabled = false

    [services.http]
    module = ""
"#;

async fn build_app(toml: &str) -> Router {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate().expect("valid config");
    harmony::router::build_network_router(Arc::new(config), "default").await
}

async fn send(app: &Router, method: &str, uri: &str) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = if method == "POST" || method == "PUT" || method == "PATCH" {
        builder = builder.header("content-type", "application/json");
        Body::from("{\"x\":1}")
    } else {
        Body::empty()
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .expect("router response")
}

async fn status_of(app: &Router, method: &str, uri: &str) -> StatusCode {
    send(app, method, uri).await.status()
}

/// Send one request and return the backend prefix that handled it.
async fn routed_to(app: &Router, upstream: &Upstream, method: &str, uri: &str) -> String {
    let status = status_of(app, method, uri).await;
    assert_eq!(status, StatusCode::OK, "{} {}", method, uri);
    let (seen_method, seen_path) = upstream.last().expect("upstream saw the request");
    assert_eq!(seen_method, method);
    seen_path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn ucp_checkout_routes_each_request_to_the_most_specific_endpoint() {
    let upstream = start_recording_upstream().await;
    let toml = [
        MAIN_TOML.to_string(),
        pipeline_toml("collection", "/v1/checkout/sessions", &upstream.base_url),
        pipeline_toml("item", "/v1/checkout/sessions/{id}", &upstream.base_url),
        pipeline_toml(
            "complete",
            "/v1/checkout/sessions/{id}/complete",
            &upstream.base_url,
        ),
    ]
    .join("\n");
    let app = build_app(&toml).await;

    // Collection: only the collection endpoint matches the exact path.
    assert_eq!(
        routed_to(&app, &upstream, "POST", "/v1/checkout/sessions").await,
        "collection"
    );
    // Item: `{id}` beats the collection catch-all.
    assert_eq!(
        routed_to(&app, &upstream, "GET", "/v1/checkout/sessions/cs_123").await,
        "item"
    );
    assert_eq!(
        routed_to(&app, &upstream, "PUT", "/v1/checkout/sessions/cs_123").await,
        "item"
    );
    // Item action: the literal `complete` beats the item catch-all.
    assert_eq!(
        routed_to(
            &app,
            &upstream,
            "POST",
            "/v1/checkout/sessions/cs_123/complete"
        )
        .await,
        "complete"
    );
    // Other paths below an item stay with the item endpoint.
    assert_eq!(
        routed_to(&app, &upstream, "GET", "/v1/checkout/sessions/cs_123/lines").await,
        "item"
    );
    // Unrelated paths are still 404.
    assert_eq!(
        status_of(&app, "GET", "/v1/other").await,
        StatusCode::NOT_FOUND
    );
}

/// The pair from the AURA-2443 report: a collection endpoint and an action
/// endpoint below it, with no item endpoint.
#[tokio::test]
async fn collection_and_action_endpoints_route_without_an_item_endpoint() {
    let upstream = start_recording_upstream().await;
    let toml = [
        MAIN_TOML.to_string(),
        pipeline_toml("collection", "/v1/checkout/sessions", &upstream.base_url),
        pipeline_toml(
            "complete",
            "/v1/checkout/sessions/{id}/complete",
            &upstream.base_url,
        ),
    ]
    .join("\n");
    let app = build_app(&toml).await;

    assert_eq!(
        routed_to(&app, &upstream, "GET", "/v1/checkout/sessions/cs_123").await,
        "collection"
    );
    assert_eq!(
        routed_to(
            &app,
            &upstream,
            "POST",
            "/v1/checkout/sessions/cs_123/complete"
        )
        .await,
        "complete"
    );
    assert_eq!(
        routed_to(
            &app,
            &upstream,
            "POST",
            "/v1/checkout/sessions/cs_123/complete/extra"
        )
        .await,
        "complete"
    );
}

#[tokio::test]
async fn nested_literal_prefixes_route_to_the_longest_prefix() {
    let upstream = start_recording_upstream().await;
    let toml = [
        MAIN_TOML.to_string(),
        pipeline_toml("root", "/", &upstream.base_url),
        pipeline_toml("api", "/api", &upstream.base_url),
        pipeline_toml("admin", "/api/admin", &upstream.base_url),
    ]
    .join("\n");
    let app = build_app(&toml).await;

    assert_eq!(routed_to(&app, &upstream, "GET", "/").await, "root");
    assert_eq!(routed_to(&app, &upstream, "GET", "/other").await, "root");
    assert_eq!(routed_to(&app, &upstream, "GET", "/api").await, "api");
    assert_eq!(routed_to(&app, &upstream, "GET", "/api/users").await, "api");
    assert_eq!(
        routed_to(&app, &upstream, "GET", "/api/admin").await,
        "admin"
    );
    assert_eq!(
        routed_to(&app, &upstream, "DELETE", "/api/admin/users/1").await,
        "admin"
    );
    // A literal that only shares a text prefix is not a path prefix.
    assert_eq!(
        routed_to(&app, &upstream, "GET", "/api/administrators").await,
        "api"
    );
}

/// A pipeline with a `jmix` endpoint whose routes start at `/api/jmix`.
/// JMIX serves GET on `/api/jmix/{id}` and GET and POST on `/api/jmix`,
/// plus the HEAD and OPTIONS it adds to GET routes.
fn jmix_pipeline_toml(base_url: &str) -> String {
    format!(
        r#"
        [pipelines.jmix]
        networks = ["default"]
        endpoints = ["jmix_in"]
        backends = ["jmix_out"]
        middleware = []

        [endpoints.jmix_in]
        service = "jmix"
        [endpoints.jmix_in.options]
        path_prefix = "/"

        [backends.jmix_out]
        service = "http"
        [backends.jmix_out.options]
        base_url = "{base_url}"
        path_prefix = "/jmix"
        "#
    )
}

/// The endpoint is chosen by path alone. An `http` endpoint at `/api` must
/// not receive a DELETE that the JMIX endpoint at `/api/jmix` refuses. The
/// `/api` pipeline may have different auth, policies and backends.
#[tokio::test]
async fn method_the_most_specific_endpoint_lacks_gets_405_not_a_less_specific_endpoint() {
    let upstream = start_recording_upstream().await;
    let toml = [
        MAIN_TOML.to_string(),
        pipeline_toml("api", "/api", &upstream.base_url),
        jmix_pipeline_toml(&upstream.base_url),
    ]
    .join("\n");
    let app = build_app(&toml).await;

    for (method, uri, allow) in [
        ("DELETE", "/api/jmix/1", "GET, HEAD, OPTIONS"),
        ("PUT", "/api/jmix/1", "GET, HEAD, OPTIONS"),
        ("PUT", "/api/jmix", "GET, HEAD, OPTIONS, POST"),
    ] {
        let response = send(&app, method, uri).await;
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{} {}",
            method,
            uri
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some(allow),
            "{} {}",
            method,
            uri
        );
    }
    assert_eq!(
        upstream.last(),
        None,
        "the /api backend must not receive requests the JMIX endpoint refuses"
    );

    // Paths the JMIX endpoint does not match still go to /api.
    assert_eq!(
        routed_to(&app, &upstream, "DELETE", "/api/users/1").await,
        "api"
    );
    assert_eq!(
        routed_to(&app, &upstream, "DELETE", "/api/jmix/1/extra").await,
        "api"
    );
}

// ---------------------------------------------------------------------------
// Config load
// ---------------------------------------------------------------------------

/// Write `config.toml` plus one file per `(file name, contents)` under
/// `pipelines/`, and load it the way the binary does.
fn load_from_files(dir: &Path, pipelines: &[(&str, String)]) -> Result<Config, ConfigError> {
    fs::create_dir_all(dir.join("pipelines")).unwrap();
    fs::write(dir.join("config.toml"), MAIN_TOML).unwrap();
    for (file, contents) in pipelines {
        fs::write(dir.join("pipelines").join(file), contents).unwrap();
    }
    Config::load(dir.join("config.toml").to_str().unwrap())
}

fn load_error_text(result: Result<Config, ConfigError>) -> String {
    match result {
        Ok(_) => panic!("config loaded, expected a route conflict error"),
        Err(ConfigError::InvalidNetwork { name, reason }) => {
            assert_eq!(name, "default");
            reason
        }
        Err(other) => panic!("expected InvalidNetwork, got {:?}", other),
    }
}

#[test]
fn ucp_checkout_config_loads() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    let result = load_from_files(
        dir.path(),
        &[
            (
                "collection.toml",
                pipeline_toml("collection", "/v1/checkout/sessions", url),
            ),
            (
                "item.toml",
                pipeline_toml("item", "/v1/checkout/sessions/{id}", url),
            ),
            (
                "complete.toml",
                pipeline_toml("complete", "/v1/checkout/sessions/{id}/complete", url),
            ),
        ],
    );
    if let Err(e) = result {
        panic!("ucp checkout config should load: {:?}", e);
    }
}

#[test]
fn identical_prefixes_fail_config_load_naming_both_endpoints_and_files() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    let reason = load_error_text(load_from_files(
        dir.path(),
        &[
            (
                "orders-a.toml",
                pipeline_toml("orders_a", "/v1/orders", url),
            ),
            (
                "orders-b.toml",
                pipeline_toml("orders_b", "/v1/orders", url),
            ),
        ],
    ));

    for needle in [
        "orders_a_in",
        "orders_b_in",
        "orders-a.toml",
        "orders-b.toml",
        "/v1/orders",
    ] {
        assert!(
            reason.contains(needle),
            "'{}' missing from: {}",
            needle,
            reason
        );
    }
}

#[test]
fn same_shape_with_different_parameter_names_fails_config_load() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    let reason = load_error_text(load_from_files(
        dir.path(),
        &[
            ("by-id.toml", pipeline_toml("by_id", "/v1/items/{id}", url)),
            (
                "by-sku.toml",
                pipeline_toml("by_sku", "/v1/items/{sku}", url),
            ),
        ],
    ));

    assert!(reason.contains("by_id_in"), "{}", reason);
    assert!(reason.contains("by_sku_in"), "{}", reason);
}

/// Test-only endpoint service that registers one route with the path and
/// methods given in its options. No built-in endpoint lets a config choose
/// its methods, so this is the only way to put two endpoints with the same
/// path shape and disjoint methods on one network.
mod method_scoped {
    use async_trait::async_trait;
    use axum::http::Method;
    use axum::response::Response;
    use harmony::config::config::ConfigError;
    use harmony::router::route_config::RouteConfig;
    use harmony::{RequestEnvelope, ResponseEnvelope, ServiceFactory, ServiceHandler, ServiceType};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Once;

    pub const NAME: &str = "method_scoped_test";

    type Error = Box<dyn std::error::Error + Send + Sync>;

    struct Factory;

    impl ServiceFactory for Factory {
        fn service_name(&self) -> &str {
            NAME
        }

        fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
            Box::new(Service)
        }
    }

    struct Service;

    #[async_trait]
    impl ServiceType for Service {
        fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
            Ok(())
        }

        fn build_router(&self, options: &HashMap<String, Value>) -> Vec<RouteConfig> {
            let path = options
                .get("path")
                .and_then(Value::as_str)
                .expect("method_scoped_test endpoint needs a 'path' option");
            let methods = options
                .get("methods")
                .and_then(Value::as_array)
                .expect("method_scoped_test endpoint needs a 'methods' option")
                .iter()
                .map(|m| {
                    let name = m.as_str().expect("method names are strings");
                    Method::from_bytes(name.as_bytes()).expect("valid method name")
                })
                .collect();
            vec![RouteConfig {
                path: path.to_string(),
                methods,
                description: None,
            }]
        }
    }

    #[async_trait]
    impl ServiceHandler<Value> for Service {
        type ReqBody = Value;

        async fn endpoint_incoming_request(
            &self,
            envelope: RequestEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
            Ok(envelope)
        }

        async fn endpoint_outgoing_response(
            &self,
            _envelope: ResponseEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<Response, Error> {
            Err("method_scoped_test is only used for config load tests".into())
        }

        async fn backend_outgoing_request(
            &self,
            _envelope: RequestEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
            Err("method_scoped_test is not a backend".into())
        }
    }

    pub fn register() {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| {
            harmony::extensions::registry::initialize_builtin_factories_sync();
            harmony::get_registry()
                .write()
                .expect("registry lock")
                .register_service_factory(Box::new(Factory));
        });
    }

    pub fn pipeline_toml(name: &str, path: &str, methods: &[&str]) -> String {
        let methods = methods
            .iter()
            .map(|m| format!("\"{}\"", m))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"
            [pipelines.{name}]
            networks = ["default"]
            endpoints = ["{name}_in"]
            backends = []
            middleware = []

            [endpoints.{name}_in]
            service = "{NAME}"
            [endpoints.{name}_in.options]
            path = "{path}"
            methods = [{methods}]
            "#
        )
    }
}

/// Endpoints are chosen by path alone, so a second endpoint with the same
/// path shape could never receive a request. Splitting methods across two
/// endpoints does not change that.
#[test]
fn same_shape_with_disjoint_methods_fails_config_load() {
    method_scoped::register();
    let dir = TempDir::new().unwrap();
    let reason = load_error_text(load_from_files(
        dir.path(),
        &[
            (
                "orders-read.toml",
                method_scoped::pipeline_toml("orders_read", "/v1/orders/{id}", &["GET"]),
            ),
            (
                "orders-write.toml",
                method_scoped::pipeline_toml("orders_write", "/v1/orders/{order_id}", &["DELETE"]),
            ),
        ],
    ));

    for needle in [
        "orders_read_in",
        "orders_write_in",
        "orders-read.toml",
        "orders-write.toml",
        "/v1/orders/{id}",
        "/v1/orders/{order_id}",
    ] {
        assert!(
            reason.contains(needle),
            "'{}' missing from: {}",
            needle,
            reason
        );
    }
}

#[test]
fn invalid_path_prefix_fails_config_load() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    let reason = load_error_text(load_from_files(
        dir.path(),
        &[("bad.toml", pipeline_toml("bad", "/v1/{id}.json", url))],
    ));

    assert!(reason.contains("bad_in"), "{}", reason);
    assert!(reason.contains("/v1/{id}.json"), "{}", reason);
}

#[test]
fn path_prefix_without_leading_slash_fails_config_load() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    let reason = load_error_text(load_from_files(
        dir.path(),
        &[("bad.toml", pipeline_toml("bad", "v1/orders", url))],
    ));

    assert!(reason.contains("bad_in"), "{}", reason);
    assert!(reason.contains("must start with '/'"), "{}", reason);
}

/// `--validate-config` and a normal start both reject the config with exit
/// code 1 and a message on stderr, like every other load error.
#[test]
fn binary_rejects_conflicting_routes_with_exit_code_1() {
    let dir = TempDir::new().unwrap();
    let url = "http://127.0.0.1:1";
    fs::create_dir_all(dir.path().join("pipelines")).unwrap();
    fs::write(dir.path().join("config.toml"), MAIN_TOML).unwrap();
    fs::write(
        dir.path().join("pipelines/orders-a.toml"),
        pipeline_toml("orders_a", "/v1/orders", url),
    )
    .unwrap();
    fs::write(
        dir.path().join("pipelines/orders-b.toml"),
        pipeline_toml("orders_b", "/v1/orders", url),
    )
    .unwrap();
    let config_path = dir.path().join("config.toml");

    for extra_args in [vec!["--validate-config"], vec![]] {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_harmony"))
            .arg("--config")
            .arg(&config_path)
            .args(&extra_args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run harmony binary");

        // A config that is wrongly accepted would serve forever.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while child.try_wait().expect("poll harmony").is_none() {
            if std::time::Instant::now() > deadline {
                child.kill().ok();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = child.wait_with_output().expect("harmony output");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(
            output.status.code(),
            Some(1),
            "args {:?}, stderr: {}",
            extra_args,
            stderr
        );
        assert!(
            stderr.contains("Invalid network 'default'"),
            "stderr: {}",
            stderr
        );
        assert!(stderr.contains("orders_a_in"), "stderr: {}", stderr);
        assert!(stderr.contains("orders_b_in"), "stderr: {}", stderr);
        assert!(!stderr.contains("panicked"), "stderr: {}", stderr);
    }
}

/// A config that never went through validation must not panic the router.
/// The conflicting and invalid routes are left out and the rest still work.
#[tokio::test]
async fn router_does_not_panic_on_an_unvalidated_config() {
    let upstream = start_recording_upstream().await;
    let toml = [
        MAIN_TOML.to_string(),
        pipeline_toml("good", "/good", &upstream.base_url),
        pipeline_toml("bad_pattern", "/v1/{id}.json", &upstream.base_url),
        pipeline_toml("no_slash", "v1/orders", &upstream.base_url),
        pipeline_toml("dup_a", "/dup/{a}", &upstream.base_url),
        pipeline_toml("dup_b", "/dup/{b}", &upstream.base_url),
    ]
    .join("\n");
    let config: Config = toml::from_str(&toml).expect("TOML parse error");
    let app = harmony::router::build_network_router(Arc::new(config), "default").await;

    assert_eq!(routed_to(&app, &upstream, "GET", "/good/x").await, "good");
    // Pipelines register in name order, so the first duplicate keeps the path.
    assert_eq!(routed_to(&app, &upstream, "GET", "/dup/1").await, "dup_a");
}

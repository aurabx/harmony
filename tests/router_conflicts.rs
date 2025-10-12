use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for .oneshot

#[tokio::test]
async fn router_builds_and_skips_conflicting_pipeline() {
    // Build a minimal Config with two pipelines that conflict on the same path
    let mut cfg: harmony::config::config::Config = Default::default();

    // Network named "default" so the router includes these pipelines
    cfg.network
        .insert("default".to_string(), Default::default());

    // Two HTTP endpoints with the same path_prefix -> will produce identical routes
    let mut ep_opts: HashMap<String, serde_json::Value> = HashMap::new();
    ep_opts.insert(
        "path_prefix".into(),
        serde_json::Value::String("/conflict".into()),
    );

    cfg.endpoints.insert(
        "ep1".into(),
        harmony::models::endpoints::endpoint::Endpoint {
            service: "http".into(),
            options: Some(ep_opts.clone()),
        },
    );
    cfg.endpoints.insert(
        "ep2".into(),
        harmony::models::endpoints::endpoint::Endpoint {
            service: "http".into(),
            options: Some(ep_opts.clone()),
        },
    );

    // Pipelines referencing the endpoints above with the same network
    let mut p1: harmony::models::pipelines::config::Pipeline = Default::default();
    p1.description = "p1".into();
    p1.networks = vec!["default".into()];
    p1.endpoints = vec!["ep1".into()];

    let mut p2: harmony::models::pipelines::config::Pipeline = Default::default();
    p2.description = "p2".into();
    p2.networks = vec!["default".into()];
    p2.endpoints = vec!["ep2".into()];

    cfg.pipelines.insert("p1".into(), p1);
    cfg.pipelines.insert("p2".into(), p2);

    // Build the router for the "default" network
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Hit the conflicting route. The router should have registered exactly one of the pipelines,
    // and, critically, it must NOT panic. Expect 200 OK from the surviving route.
    let req = Request::builder()
        .method("GET")
        .uri("/conflict/abc")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.expect("handler");
    assert_eq!(resp.status(), StatusCode::OK);

    // Sanity: a non-existent route should still return 404
    let req2 = Request::builder()
        .method("GET")
        .uri("/nope")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp2 = app.oneshot(req2).await.expect("handler");
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
}

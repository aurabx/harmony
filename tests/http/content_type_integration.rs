use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tower::ServiceExt; // for Router::oneshot

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

fn get_test_config() -> &'static str {
    r#"
        [proxy]
        id = "content-type-test"
        store_dir = "./tmp"

        [proxy.content_limits]
        max_body_size = 10485760
        max_csv_rows = 10000
        max_xml_depth = 100
        max_multipart_files = 10
        max_form_fields = 1000

        [logging]
        log_level = "info"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.content_test]
        description = "Content-type testing pipeline"
        networks = ["default"]
        endpoints = ["content_endpoint"]
        backends = ["echo_backend"]
        middleware = ["passthru"]

        [middleware.passthru]
        type = "passthru"

        [endpoints.content_endpoint]
        service = "http"
        [endpoints.content_endpoint.options]
        path_prefix = "/content"

        [backends.echo_backend]
        service = "echo"
        [backends.echo_backend.options]
        path_prefix = "/echo-back"

        [services.http]
        module = ""

        [services.echo]
        module = ""

        [middleware_types.passthru]
        module = ""
    "#
}

async fn build_test_router() -> axum::Router<()> {
    // Ensure ./tmp directory exists for store_dir
    let _ = std::fs::create_dir_all("./tmp");

    let cfg = load_config_from_str(get_test_config()).expect("valid config");
    harmony::router::build_network_router(Arc::new(cfg), "default").await
}

#[tokio::test]
async fn test_json_content_type() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"name": "Alice", "age": 30});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert_eq!(json["name"], "Alice");
    assert_eq!(json["age"], 30);

    // Verify metadata fields from echo backend
    assert_eq!(json["path"], "test");
    assert_eq!(json["full_path"], "/content/test");
}

#[tokio::test]
async fn test_xml_content_type() {
    let app = build_test_router().await;

    let xml_payload = r#"<person><name>Bob</name><age>25</age><city>NYC</city></person>"#;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/xml")
                .body(Body::from(xml_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert!(json["person"].is_object());

    let person = &json["person"];
    assert_eq!(person["name"], "Bob");
    assert_eq!(person["age"], "25");
    assert_eq!(person["city"], "NYC");
}

#[tokio::test]
async fn test_xml_with_attributes() {
    let app = build_test_router().await;

    let xml_payload = r#"<person id="123" type="customer"><name>Charlie</name></person>"#;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "text/xml")
                .body(Body::from(xml_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    let person = &json["person"];
    assert_eq!(person["@id"], "123");
    assert_eq!(person["@type"], "customer");
    assert_eq!(person["name"], "Charlie");
}

#[tokio::test]
async fn test_csv_content_type() {
    let app = build_test_router().await;

    let csv_payload = "name,age,city\nAlice,30,NYC\nBob,25,LA\nCharlie,35,SF";

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "text/csv")
                .body(Body::from(csv_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert!(json["rows"].is_array());
    let rows = json["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);

    assert_eq!(rows[0]["name"], "Alice");
    assert_eq!(rows[0]["age"], "30");
    assert_eq!(rows[0]["city"], "NYC");

    assert_eq!(rows[1]["name"], "Bob");
    assert_eq!(rows[2]["name"], "Charlie");
}

#[tokio::test]
async fn test_csv_with_formula_injection_prevention() {
    let app = build_test_router().await;

    // CSV with potentially dangerous formulas
    let csv_payload = "name,formula\nAlice,=SUM(A1:A10)\nBob,+1234\nCharlie,-5678\nDave,@import";

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "text/csv")
                .body(Body::from(csv_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    let rows = json["rows"].as_array().unwrap();

    // Verify formulas are escaped with leading single quote
    assert_eq!(rows[0]["formula"], "'=SUM(A1:A10)");
    assert_eq!(rows[1]["formula"], "'+1234");
    assert_eq!(rows[2]["formula"], "'-5678");
    assert_eq!(rows[3]["formula"], "'@import");
}

#[tokio::test]
async fn test_form_urlencoded_content_type() {
    let app = build_test_router().await;

    let form_payload = "name=Alice&age=30&city=NYC&interests=coding&interests=music";

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert_eq!(json["name"], "Alice");
    assert_eq!(json["age"], "30");
    assert_eq!(json["city"], "NYC");

    // Note: Multiple values with same name get overwritten in form-urlencoded
    // This is expected behavior - use multipart for multiple files
    assert_eq!(json["interests"], "music");
}

#[tokio::test]
async fn test_form_urlencoded_with_array_notation() {
    let app = build_test_router().await;

    let form_payload = "name=Alice&interests[]=coding&interests[]=music&interests[]=gaming";

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert_eq!(json["name"], "Alice");
    assert!(json["interests"].is_array());

    let interests = json["interests"].as_array().unwrap();
    assert_eq!(interests.len(), 3);
    assert_eq!(interests[0], "coding");
    assert_eq!(interests[1], "music");
    assert_eq!(interests[2], "gaming");
}

#[tokio::test]
async fn test_multipart_form_data() {
    let app = build_test_router().await;

    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let multipart_payload = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"name\"\r\n\
         \r\n\
         Alice\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"age\"\r\n\
         \r\n\
         30\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"file1\"; filename=\"test.txt\"\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         This is test file content.\r\n\
         --{boundary}--\r\n",
        boundary = boundary
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(multipart_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert!(json["fields"].is_object());
    assert_eq!(json["fields"]["name"], "Alice");
    assert_eq!(json["fields"]["age"], "30");

    // Verify file metadata
    assert!(json["files"].is_array());
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);

    assert_eq!(files[0]["name"], "file1"); // Field name in multipart
    assert_eq!(files[0]["filename"], "test.txt");
    assert_eq!(files[0]["content_type"], "text/plain");
    assert!(files[0]["size"].as_u64().unwrap() > 0);
    assert!(files[0]["checksum"].is_string());
}

#[tokio::test]
async fn test_binary_content_type() {
    let app = build_test_router().await;

    // Simulate binary image data
    let binary_payload: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46]; // JPEG header

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "image/jpeg")
                .body(Body::from(binary_payload.clone()))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Echo backend returns normalized_data flattened at top level
    assert_eq!(json["format"], "binary");
    assert_eq!(json["content_type"], "image/jpeg");
    assert_eq!(json["size"], binary_payload.len());
    assert!(json["checksum"].is_string());
}

#[tokio::test]
async fn test_unsupported_content_type_falls_back_to_json() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"data": "test"});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/vnd.custom+json") // Unsupported type
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Should fall back to JSON parsing
    assert_eq!(json["data"], "test");
}

#[tokio::test]
async fn test_missing_content_type_defaults_to_json() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"fallback": "json"});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                // No content-type header
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Should default to JSON parsing
    assert_eq!(json["fallback"], "json");
}

#[tokio::test]
async fn test_malformed_xml_partial_parse() {
    let app = build_test_router().await;

    let malformed_xml = "<person><name>Alice</name>"; // Missing closing tag

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/xml")
                .body(Body::from(malformed_xml))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    // XML parser does partial parsing on malformed input, so it succeeds
    // This validates that the system handles gracefully what it can parse
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Malformed XML parsing fails, so it falls back to binary representation
    // Verify we got a response with the original_data field (binary fallback)
    assert!(json.is_object(), "Should have response object");
    assert!(json["original_data"].is_array(), "Should have binary fallback for failed parse");
}

#[tokio::test]
async fn test_content_type_with_charset() {
    let app = build_test_router().await;

    let test_payload = serde_json::json!({"charset": "test"});
    let payload_str = serde_json::to_string(&test_payload).expect("serialize payload");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/json; charset=utf-8")
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Should parse JSON correctly despite charset parameter
    assert_eq!(json["charset"], "test");
}

#[tokio::test]
async fn test_fhir_json_content_type() {
    let app = build_test_router().await;

    let fhir_payload = serde_json::json!({
        "resourceType": "Patient",
        "id": "example",
        "name": [{"family": "Doe", "given": ["John"]}]
    });
    let payload_str = serde_json::to_string(&fhir_payload).expect("serialize payload");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/fhir+json")
                .body(Body::from(payload_str))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Verify FHIR JSON is parsed correctly
    assert_eq!(json["resourceType"], "Patient");
    assert_eq!(json["id"], "example");
}

#[tokio::test]
async fn test_xml_nested_elements() {
    let app = build_test_router().await;

    let xml_payload = r#"
        <patient>
            <name>
                <first>John</first>
                <last>Doe</last>
            </name>
            <contact>
                <phone>555-1234</phone>
                <email>john@example.com</email>
            </contact>
        </patient>
    "#;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/content/test")
                .method("POST")
                .header("content-type", "application/xml")
                .body(Body::from(xml_payload))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body.to_vec()).expect("utf8");

    let json: serde_json::Value = serde_json::from_str(&body_str).expect("json");

    // Verify nested XML structure
    let patient = &json["patient"];
    assert!(patient["name"].is_object());
    assert_eq!(patient["name"]["first"], "John");
    assert_eq!(patient["name"]["last"], "Doe");

    assert!(patient["contact"].is_object());
    assert_eq!(patient["contact"]["phone"], "555-1234");
    assert_eq!(patient["contact"]["email"], "john@example.com");
}

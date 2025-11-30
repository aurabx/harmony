use harmony::models::envelope::envelope::{RequestEnvelopeBuilder, ResponseEnvelope};
use harmony::models::middleware::middleware::Middleware;
use harmony::models::middleware::types::dicom_to_dicomweb::DicomToDicomwebMiddleware;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_left_c_find_to_qido() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Construct a C-FIND request envelope
    // normalized_data contains the "identifier" used in DIMSE C-FIND
    let identifier = json!({
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "Smith^John"}] },
        "00100020": { "vr": "LO", "Value": ["12345"] }
    });
    
    let normalized_data = json!({
        "identifier": identifier
    });

    let mut metadata = HashMap::new();
    metadata.insert("operation".to_string(), "C-FIND".to_string());

    let envelope = RequestEnvelopeBuilder::new()
        .method("POST") // DIMSE usually comes in via some mechanism, but here we simulate internal pipeline flow
        .uri("dimse://scp/find")
        .metadata(metadata)
        .original_data(Value::Null)
        .normalized_data(Some(normalized_data))
        .build()
        .expect("Failed to build envelope");

    // Execute LEFT side
    let result = middleware.left(envelope).await.expect("Middleware failed");

    // Verify QIDO-RS transformation
    assert_eq!(result.request_details.method, "GET");
    assert_eq!(result.request_details.uri, "/studies");
    
    let qp = result.request_details.query_params;
    assert!(qp.contains_key("00100010"));
    assert!(qp.contains_key("00100020"));
    
    assert_eq!(qp.get("00100010").unwrap()[0], "Smith^John");
    assert_eq!(qp.get("00100020").unwrap()[0], "12345");
}

#[tokio::test]
async fn test_left_c_store_to_stow() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Create a dummy DICOM file
    let mut tmp_file = NamedTempFile::new().expect("create temp file");
    tmp_file.write_all(b"FAKE_DICOM_DATA").expect("write temp file");
    let path = tmp_file.path().to_string_lossy().to_string();

    let normalized_data = json!({
        "file": path
    });

    let mut metadata = HashMap::new();
    metadata.insert("operation".to_string(), "C-STORE".to_string());

    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("dimse://scp/store")
        .metadata(metadata)
        .original_data(Value::Null)
        .normalized_data(Some(normalized_data))
        .build()
        .expect("Failed to build envelope");

    // Execute LEFT side
    let result = middleware.left(envelope).await.expect("Middleware failed");

    // Verify STOW-RS transformation
    assert_eq!(result.request_details.method, "POST");
    assert_eq!(result.request_details.uri, "/studies");
    
    let headers = result.request_details.headers;
    let content_type = headers.get("content-type").expect("Missing content-type");
    assert!(content_type.contains("multipart/related"));
    assert!(content_type.contains("application/dicom"));
    
    // Verify body_b64 is set in normalized_data
    let nd = result.normalized_data.expect("Missing normalized_data");
    let b64 = nd.get("body_b64").expect("Missing body_b64").as_str().expect("body_b64 not string");
    
    use base64::Engine;
    let body_bytes = base64::engine::general_purpose::STANDARD.decode(b64).expect("Failed to decode body_b64");
    let body_str = String::from_utf8_lossy(&body_bytes);
    
    assert!(body_str.contains("FAKE_DICOM_DATA"));
    assert!(body_str.contains("--boundary_"));
}

#[tokio::test]
async fn test_left_c_get_to_wado() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Construct C-GET identifier with UIDs
    let identifier = json!({
        "0020000D": { "vr": "UI", "Value": ["1.2.840.113619.2.55.3.42710457.305"] }, // Study
        "0020000E": { "vr": "UI", "Value": ["1.2.840.113619.2.55.3.42710457.305.1"] }, // Series
        "00080018": { "vr": "UI", "Value": ["1.2.840.113619.2.55.3.42710457.305.1.1"] } // Instance
    });
    
    let normalized_data = json!({
        "identifier": identifier
    });

    let mut metadata = HashMap::new();
    metadata.insert("operation".to_string(), "C-GET".to_string());

    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("dimse://scp/get")
        .metadata(metadata)
        .original_data(Value::Null)
        .normalized_data(Some(normalized_data))
        .build()
        .expect("Failed to build envelope");

    // Execute LEFT side
    let result = middleware.left(envelope).await.expect("Middleware failed");

    // Verify WADO-RS transformation
    assert_eq!(result.request_details.method, "GET");
    assert_eq!(result.request_details.uri, "/studies/1.2.840.113619.2.55.3.42710457.305/series/1.2.840.113619.2.55.3.42710457.305.1/instances/1.2.840.113619.2.55.3.42710457.305.1.1");
    
    let headers = result.request_details.headers;
    let accept = headers.get("accept").expect("Missing accept header");
    assert!(accept.contains("multipart/related"));
    assert!(accept.contains("application/dicom"));
}

#[tokio::test]
async fn test_left_c_move_to_wado() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Construct C-MOVE identifier (Study Level)
    let identifier = json!({
        "0020000D": { "vr": "UI", "Value": ["1.2.3.4"] }
    });
    
    let normalized_data = json!({
        "identifier": identifier
    });

    let mut metadata = HashMap::new();
    metadata.insert("operation".to_string(), "C-MOVE".to_string());

    let envelope = RequestEnvelopeBuilder::new()
        .method("POST")
        .uri("dimse://scp/move")
        .metadata(metadata)
        .original_data(Value::Null)
        .normalized_data(Some(normalized_data))
        .build()
        .expect("Failed to build envelope");

    // Execute LEFT side
    let result = middleware.left(envelope).await.expect("Middleware failed");

    // Verify WADO-RS transformation
    assert_eq!(result.request_details.method, "GET");
    assert_eq!(result.request_details.uri, "/studies/1.2.3.4");
}

#[tokio::test]
async fn test_right_passthrough() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Construct response envelope
    let mut metadata = HashMap::new();
    metadata.insert("operation".to_string(), "C-FIND".to_string());

    let response_envelope = ResponseEnvelope::<Value>::from_backend(
        harmony::models::envelope::envelope::RequestDetails::default(),
        200,
        HashMap::new(),
        Value::Null,
        Some(metadata)
    );

    // Execute RIGHT side
    let result = middleware.right(response_envelope).await.expect("Middleware failed");

    // Currently right side is passthrough, just verify status
    assert_eq!(result.response_details.status, 200);
}

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

// Helper to create a response envelope for testing
fn create_response_envelope(
    operation: &str,
    status: u16,
    headers: HashMap<String, String>,
    original_data: Value,
    normalized_data: Option<Value>,
) -> ResponseEnvelope<Value> {
    let mut request_metadata = HashMap::new();
    request_metadata.insert("operation".to_string(), operation.to_string());

    let mut request_details = harmony::models::envelope::envelope::RequestDetails::default();
    request_details.metadata = request_metadata;

    let mut envelope = ResponseEnvelope::<Value>::from_backend(
        request_details,
        status,
        headers,
        original_data,
        None,
    );
    envelope.normalized_data = normalized_data;
    envelope
}

// ==================== C-FIND Response Tests ====================

#[tokio::test]
async fn test_right_cfind_response_array() {
    let middleware = DicomToDicomwebMiddleware::new();

    // QIDO-RS returns an array of study results
    let qido_results = json!([
        { "00100020": { "vr": "LO", "Value": ["PATIENT1"] } },
        { "00100020": { "vr": "LO", "Value": ["PATIENT2"] } }
    ]);

    let envelope = create_response_envelope(
        "C-FIND",
        200,
        HashMap::new(),
        Value::Null,
        Some(qido_results),
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );

    // Verify result_count is 2
    assert_eq!(
        result.response_details.metadata.get("result_count"),
        Some(&"2".to_string())
    );
}

#[tokio::test]
async fn test_right_cfind_response_single_object() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Some QIDO-RS servers return a single object for single match
    let qido_result = json!({
        "00100020": { "vr": "LO", "Value": ["PATIENT1"] }
    });

    let envelope = create_response_envelope(
        "C-FIND",
        200,
        HashMap::new(),
        qido_result.clone(),
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );

    // Verify result_count is 1
    assert_eq!(
        result.response_details.metadata.get("result_count"),
        Some(&"1".to_string())
    );

    // Verify normalized_data is now an array
    let nd = result.normalized_data.expect("normalized_data should exist");
    assert!(nd.is_array());
    assert_eq!(nd.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_right_cfind_response_empty_null() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Empty response (no matches)
    let envelope = create_response_envelope(
        "C-FIND",
        200,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS (no matches is still success)
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );

    // Verify result_count is 0
    assert_eq!(
        result.response_details.metadata.get("result_count"),
        Some(&"0".to_string())
    );

    // Verify normalized_data is empty array
    let nd = result.normalized_data.expect("normalized_data should exist");
    assert!(nd.is_array());
    assert_eq!(nd.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_right_cfind_response_404_no_matches() {
    let middleware = DicomToDicomwebMiddleware::new();

    // 404 for C-FIND means no matches (should map to SUCCESS)
    let envelope = create_response_envelope(
        "C-FIND",
        404,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS (404 = no matches for QIDO)
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );
}

#[tokio::test]
async fn test_right_cfind_response_500_failure() {
    let middleware = DicomToDicomwebMiddleware::new();

    // 500 should map to FAILURE_UNABLE_TO_PROCESS
    let envelope = create_response_envelope(
        "C-FIND",
        500,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is FAILURE_UNABLE_TO_PROCESS (0xC000)
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0xC000".to_string())
    );
}

// ==================== C-STORE Response Tests ====================

#[tokio::test]
async fn test_right_cstore_response_success() {
    let middleware = DicomToDicomwebMiddleware::new();

    // STOW-RS success response with ReferencedSOPSequence
    let stow_response = json!({
        "00081199": {
            "vr": "SQ",
            "Value": [{
                "00081150": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.2"] },
                "00081155": { "vr": "UI", "Value": ["1.2.3.4.5.6.7.8.9"] }
            }]
        }
    });

    let envelope = create_response_envelope(
        "C-STORE",
        200,
        HashMap::new(),
        Value::Null,
        Some(stow_response),
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );
}

#[tokio::test]
async fn test_right_cstore_response_failure() {
    let middleware = DicomToDicomwebMiddleware::new();

    // STOW-RS failure (500)
    let envelope = create_response_envelope(
        "C-STORE",
        500,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is FAILURE_UNABLE_TO_PROCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0xC000".to_string())
    );
}

#[tokio::test]
async fn test_right_cstore_response_404_failure() {
    let middleware = DicomToDicomwebMiddleware::new();

    // 404 for C-STORE is a failure (unlike C-FIND)
    let envelope = create_response_envelope(
        "C-STORE",
        404,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is FAILURE_UNABLE_TO_PROCESS (404 is failure for C-STORE)
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0xC000".to_string())
    );
}

// ==================== C-GET/C-MOVE Response Tests ====================

#[tokio::test]
async fn test_right_cget_response_single_dicom() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Single DICOM instance response
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/dicom".to_string());

    let envelope = create_response_envelope(
        "C-GET",
        200,
        headers,
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );

    // Verify dataset_count is 1
    assert_eq!(
        result.response_details.metadata.get("dataset_count"),
        Some(&"1".to_string())
    );
}

#[tokio::test]
async fn test_right_cget_response_multipart_with_body() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Simulate multipart/related response with body_b64 (as would be set by to_json())
    let boundary = "boundary123";
    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        format!("multipart/related; type=\"application/dicom\"; boundary={}", boundary),
    );

    // Create fake multipart body
    let multipart_body = format!(
        "--{}\r\nContent-Type: application/dicom\r\n\r\nFAKE_DICOM_DATA_1\r\n--{}\r\nContent-Type: application/dicom\r\n\r\nFAKE_DICOM_DATA_2\r\n--{}--\r\n",
        boundary, boundary, boundary
    );

    use base64::Engine;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode(multipart_body.as_bytes());

    let normalized_data = json!({
        "body_b64": body_b64,
        "content_length": multipart_body.len()
    });

    let envelope = create_response_envelope(
        "C-GET",
        200,
        headers,
        Value::Null,
        Some(normalized_data),
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );

    // Verify boundary was extracted
    assert_eq!(
        result.response_details.metadata.get("multipart_boundary"),
        Some(&boundary.to_string())
    );

    // Verify dataset_count is 2
    assert_eq!(
        result.response_details.metadata.get("dataset_count"),
        Some(&"2".to_string())
    );

    // Verify normalized_data contains parsed datasets
    let nd = result.normalized_data.expect("normalized_data should exist");
    let datasets = nd.get("datasets").expect("datasets array should exist");
    assert!(datasets.is_array());
    assert_eq!(datasets.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_right_cmove_response_multipart_with_body() {
    let middleware = DicomToDicomwebMiddleware::new();

    // Same as C-GET multipart but operation=C-MOVE
    let boundary = "b-move-001";
    let mut headers = HashMap::new();
    headers.insert(
        "Content-Type".to_string(), // mixed-case header key
        format!("multipart/related; type=\"application/dicom\"; boundary=\"{}\"", boundary), // quoted boundary
    );

    let multipart_body = format!(
        "--{}\r\nContent-Type: application/dicom\r\n\r\nMOVE_DATA_1\r\n--{}--\r\n",
        boundary, boundary
    );

    use base64::Engine;
    let body_b64 = base64::engine::general_purpose::STANDARD.encode(multipart_body.as_bytes());

    let normalized_data = json!({
        "body_b64": body_b64,
        "content_length": multipart_body.len()
    });

    let envelope = create_response_envelope(
        "C-MOVE",
        200,
        headers,
        Value::Null,
        Some(normalized_data),
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // SUCCESS and dataset_count==1
    assert_eq!(result.response_details.metadata.get("dicom_status"), Some(&"0x0000".to_string()));
    assert_eq!(result.response_details.metadata.get("dataset_count"), Some(&"1".to_string()));

    // Boundary extracted without quotes
    assert_eq!(result.response_details.metadata.get("multipart_boundary"), Some(&boundary.to_string()));
}

#[tokio::test]
async fn test_right_cstore_response_409_warning() {
    let middleware = DicomToDicomwebMiddleware::new();

    let envelope = create_response_envelope(
        "C-STORE",
        409,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Warning status 0xB000
    assert_eq!(result.response_details.metadata.get("dicom_status"), Some(&"0xB000".to_string()));
}

#[tokio::test]
async fn test_right_cget_response_404_failure() {
    let middleware = DicomToDicomwebMiddleware::new();

    // 404 for C-GET is a failure (resource not found)
    let envelope = create_response_envelope(
        "C-GET",
        404,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is FAILURE_UNABLE_TO_PROCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0xC000".to_string())
    );
}

#[tokio::test]
async fn test_right_cmove_response_404_failure() {
    let middleware = DicomToDicomwebMiddleware::new();

    // 404 for C-MOVE is also a failure
    let envelope = create_response_envelope(
        "C-MOVE",
        404,
        HashMap::new(),
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is FAILURE_UNABLE_TO_PROCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0xC000".to_string())
    );
}

#[tokio::test]
async fn test_right_cget_response_dicom_json() {
    let middleware = DicomToDicomwebMiddleware::new();

    // WADO-RS metadata endpoint returns JSON
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/dicom+json".to_string());

    let envelope = create_response_envelope(
        "C-GET",
        200,
        headers,
        Value::Null,
        None,
    );

    let result = middleware.right(envelope).await.expect("Middleware failed");

    // Verify DICOM status is SUCCESS
    assert_eq!(
        result.response_details.metadata.get("dicom_status"),
        Some(&"0x0000".to_string())
    );
}

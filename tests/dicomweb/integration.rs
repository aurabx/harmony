use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

mod common;
use common::{assert_uid_in_response, get_test_context};

// =============================================================================
// QIDO-RS Tests: Query Studies
// =============================================================================

#[tokio::test]
async fn test_qido_query_all_studies() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies")
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("studies query");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    if !body.is_empty() {
        let studies: Vec<serde_json::Value> =
            serde_json::from_slice(&body).expect("parse studies");
        assert!(!studies.is_empty(), "Expected at least one study");
    }
}

#[tokio::test]
async fn test_qido_query_studies_with_patient_id() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/dicomweb/studies?PatientID={}", ctx.uids.patient_id))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("studies with PatientID");
    // Should return 200 with matching studies
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for PatientID query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    if !body.is_empty() {
        let studies: Vec<serde_json::Value> =
            serde_json::from_slice(&body).expect("parse studies");
        assert!(
            !studies.is_empty(),
            "Expected at least one study for patient ID: {}",
            ctx.uids.patient_id
        );
    }
}

#[tokio::test]
async fn test_qido_query_studies_with_includefield() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies?includefield=0020000D")
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("studies with includefield");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for study query with includefield"
    );
}

#[tokio::test]
async fn test_qido_query_specific_study() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/dicomweb/studies/{}", ctx.uids.study_uid))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("specific study");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for specific study query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let study_data = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse specific study");
    // Verify response contains and matches the requested Study Instance UID
    assert_uid_in_response(
        &study_data,
        "0020000D",
        &ctx.uids.study_uid,
        "specific study query",
    );
}

// =============================================================================
// QIDO-RS Tests: Query Series
// =============================================================================

#[tokio::test]
async fn test_qido_query_all_series_in_study() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/dicomweb/studies/{}/series", ctx.uids.study_uid))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("series query");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for series query with valid study"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let series_list: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("parse series");
    assert!(!series_list.is_empty(), "Expected at least one series");
}

#[tokio::test]
async fn test_qido_query_series_with_modality_filter() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series?Modality=CT",
                    ctx.uids.study_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("series with Modality");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for series query with modality filter"
    );
}

#[tokio::test]
async fn test_qido_query_series_with_includefield() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series?includefield=0020000E",
                    ctx.uids.study_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("series with includefield");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for series query with includefield"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let series_list: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("parse series with includefield");
    assert!(!series_list.is_empty(), "Expected at least one series");
    // Verify that Series Instance UID (0020000E) is present in response
    let first_series = &series_list[0];
    assert!(
        first_series.get("0020000E").is_some(),
        "Series Instance UID (0020000E) should be in includefield response"
    );
}

#[tokio::test]
async fn test_qido_query_specific_series() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}",
                    ctx.uids.study_uid, ctx.uids.series_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("specific series");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for specific series query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let series_data = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse specific series");
    // Verify response contains and matches the requested Series Instance UID
    assert_uid_in_response(
        &series_data,
        "0020000E",
        &ctx.uids.series_1_uid,
        "specific series query",
    );
}

// =============================================================================
// QIDO-RS Tests: Query Instances
// =============================================================================

#[tokio::test]
async fn test_qido_query_all_instances_in_series() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances",
                    ctx.uids.study_uid, ctx.uids.series_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("instances query");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for instances query with valid series"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let instances: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("parse instances");
    assert!(!instances.is_empty(), "Expected at least one instance");
}

#[tokio::test]
async fn test_qido_query_instances_with_instance_number_filter() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances?InstanceNumber=1",
                    ctx.uids.study_uid, ctx.uids.series_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("instances with InstanceNumber");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for instances query with filter"
    );
}

#[tokio::test]
async fn test_qido_query_instances_with_includefield() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances?includefield=00080018",
                    ctx.uids.study_uid, ctx.uids.series_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("instances with includefield");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for instances query with includefield"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let instances: Vec<serde_json::Value> =
        serde_json::from_slice(&body).expect("parse instances with includefield");
    assert!(!instances.is_empty(), "Expected at least one instance");
    // Verify that SOP Instance UID (00080018) is present in response
    let first_instance = &instances[0];
    assert!(
        first_instance.get("00080018").is_some(),
        "SOP Instance UID (00080018) should be in includefield response"
    );
}

#[tokio::test]
async fn test_qido_query_specific_instance() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances/{}",
                    ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("specific instance");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for specific instance query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let instance_data = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse specific instance");
    // Verify response contains and matches the requested SOP Instance UID
    assert_uid_in_response(
        &instance_data,
        "00080018",
        &ctx.uids.instance_1_1_uid,
        "specific instance query",
    );
}

// =============================================================================
// WADO-RS Tests: Metadata Retrieval
// =============================================================================

#[tokio::test]
async fn test_wado_study_metadata() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/dicomweb/studies/{}/metadata", ctx.uids.study_uid))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("study metadata");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for study metadata query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let metadata = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse study metadata");
    // Metadata should be either array or object with DICOM tags
    assert!(
        metadata.is_array() || metadata.is_object(),
        "Study metadata should be array or object"
    );
}

#[tokio::test]
async fn test_wado_series_metadata() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/metadata",
                    ctx.uids.study_uid, ctx.uids.series_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("series metadata");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for series metadata query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let metadata = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse series metadata");
    assert!(
        metadata.is_array() || metadata.is_object(),
        "Series metadata should be array or object"
    );
}

#[tokio::test]
async fn test_wado_instance_metadata() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances/{}/metadata",
                    ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("instance metadata");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for instance metadata query"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let metadata = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse instance metadata");
    assert!(
        metadata.is_array() || metadata.is_object(),
        "Instance metadata should be array or object"
    );
}

#[tokio::test]
async fn test_wado_metadata_with_includefield() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/metadata?includefield=0020000D,00100010",
                    ctx.uids.study_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("study metadata with includefield");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for study metadata with includefield"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let metadata = serde_json::from_slice::<serde_json::Value>(&body)
        .expect("parse study metadata with includefield");
    // With includefield, response should be limited to requested tags or be array with such tags
    assert!(
        metadata.is_array() || metadata.is_object(),
        "Includefield metadata should be array or object"
    );
    // Verify that requested fields are present
    if metadata.is_array() {
        let arr = metadata.as_array().unwrap();
        assert!(!arr.is_empty(), "Metadata array should not be empty");
        let first = &arr[0];
        assert!(
            first.get("0020000D").is_some() || first.get("00100010").is_some(),
            "At least one of the requested includefield tags should be present"
        );
    } else {
        assert!(
            metadata.get("0020000D").is_some() || metadata.get("00100010").is_some(),
            "At least one of the requested includefield tags should be present"
        );
    }
}

// =============================================================================
// WADO-RS Tests: Instance Retrieval
// =============================================================================

#[tokio::test]
async fn test_wado_retrieve_dicom_instance() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances/{}",
                    ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("retrieve instance");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(!body.is_empty(), "Expected non-empty DICOM object");
    assert!(body.len() > 100, "Expected substantial DICOM file");
}

#[tokio::test]
async fn test_wado_retrieve_multipart_instance() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances/{}",
                    ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
                ))
                .method("GET")
                .header("Accept", "multipart/related; type=application/dicom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("retrieve instance multipart");
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("multipart/related"),
        "Expected multipart/related, got {}",
        content_type
    );
}

// =============================================================================
// WADO-RS Tests: Frame Retrieval
// =============================================================================

// #[tokio::test]
// async fn test_wado_retrieve_frame_as_jpeg() {
//     let ctx = get_test_context().await;
//
//     let resp = ctx
//         .app
//         .clone()
//         .oneshot(
//             Request::builder()
//                 .uri(&format!(
//                     "/dicomweb/studies/{}/series/{}/instances/{}/frames/1",
//                     ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
//                 ))
//                 .method("GET")
//                 .header("Accept", "image/jpeg")
//                 .body(Body::empty())
//                 .unwrap(),
//         )
//         .await
//         .expect("frame jpeg");
//     // May be 200, 406, or 500 depending on transfer syntax and backend support
//     assert!(
//         resp.status() == StatusCode::OK
//             || resp.status() == StatusCode::NOT_ACCEPTABLE
//             || resp.status().is_server_error(),
//         "Expected 200, 406, or 5xx, got {}",
//         resp.status()
//     );
//     if resp.status() == StatusCode::OK {
//         let headers = resp.headers().clone();
//         let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
//             .await
//             .unwrap();
//         assert!(!body.is_empty(), "Expected non-empty JPEG frame");
//         let content_type = headers
//             .get("content-type")
//             .and_then(|v| v.to_str().ok())
//             .unwrap_or("");
//         assert!(
//             content_type.contains("image/jpeg"),
//             "Expected image/jpeg content-type, got {}",
//             content_type
//         );
//     }
// }
//
// #[tokio::test]
// async fn test_wado_retrieve_frame_as_png() {
//     let ctx = get_test_context().await;
//
//     let resp = ctx
//         .app
//         .clone()
//         .oneshot(
//             Request::builder()
//                 .uri(&format!(
//                     "/dicomweb/studies/{}/series/{}/instances/{}/frames/1",
//                     ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
//                 ))
//                 .method("GET")
//                 .header("Accept", "image/png")
//                 .body(Body::empty())
//                 .unwrap(),
//         )
//         .await
//         .expect("frame png");
//     assert!(
//         resp.status() == StatusCode::OK
//             || resp.status() == StatusCode::NOT_ACCEPTABLE
//             || resp.status().is_server_error(),
//         "Expected 200, 406, or 5xx, got {}",
//         resp.status()
//     );
//     if resp.status() == StatusCode::OK {
//         let headers = resp.headers().clone();
//         let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
//             .await
//             .unwrap();
//         assert!(!body.is_empty(), "Expected non-empty PNG frame");
//         let content_type = headers
//             .get("content-type")
//             .and_then(|v| v.to_str().ok())
//             .unwrap_or("");
//         assert!(
//             content_type.contains("image/png"),
//             "Expected image/png content-type, got {}",
//             content_type
//         );
//     }
// }
//
// #[tokio::test]
// async fn test_wado_retrieve_multiple_frames() {
//     let ctx = get_test_context().await;
//
//     let resp = ctx
//         .app
//         .clone()
//         .oneshot(
//             Request::builder()
//                 .uri(&format!(
//                     "/dicomweb/studies/{}/series/{}/instances/{}/frames/1,2",
//                     ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
//                 ))
//                 .method("GET")
//                 .header("Accept", "multipart/related; type=image/jpeg")
//                 .body(Body::empty())
//                 .unwrap(),
//         )
//         .await
//         .expect("multiframes");
//     assert!(
//         resp.status() == StatusCode::OK
//             || resp.status() == StatusCode::NOT_ACCEPTABLE
//             || resp.status().is_server_error(),
//         "Expected 200, 406, or 5xx, got {}",
//         resp.status()
//     );
// }

// =============================================================================
// Complex Query Tests
// =============================================================================

#[tokio::test]
async fn test_complex_study_search_with_multiple_criteria() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies?Modality=CT&includefield=0020000D,00100010")
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("complex study search");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for complex study search"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let results: serde_json::Value = serde_json::from_slice(&body)
        .expect("parse complex study search");
    // Should be array of results
    assert!(results.is_array(), "Complex search result should be array");
}

#[tokio::test]
async fn test_series_search_with_detailed_filtering() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series?Modality=CT&includefield=0020000E,0008103E",
                    ctx.uids.study_uid
                ))
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("series detailed search");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Expected 200 for series detailed search"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let results: serde_json::Value =
        serde_json::from_slice(&body).expect("parse series detailed search");
    assert!(results.is_array(), "Series search result should be array");
}

// =============================================================================
// CORS and Error Tests
// =============================================================================

#[tokio::test]
async fn test_cors_preflight_options_request() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies")
                .method("OPTIONS")
                .header("Origin", "http://localhost:3000")
                .header("Access-Control-Request-Method", "GET")
                .header("Access-Control-Request-Headers", "accept, content-type")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("cors options");
    // OPTIONS may return 204 or 200
    assert!(
        resp.status() == StatusCode::NO_CONTENT || resp.status() == StatusCode::OK,
        "Expected 204 or 200, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_invalid_query_parameter() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies?InvalidParam=value123")
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invalid query param");
    // May return 200 (if ignored), 204 (no match), or 400 (if validated)
    assert!(
        resp.status() == StatusCode::OK
            || resp.status() == StatusCode::NO_CONTENT
            || resp.status() == StatusCode::BAD_REQUEST,
        "Expected 200, 204, or 400, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_nonexistent_study_query() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dicomweb/studies/1.2.3.4.5.6.7.8.9.0/series")
                .method("GET")
                .header("Accept", "application/dicom+json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("nonexistent study");
    assert!(
        resp.status() == StatusCode::NO_CONTENT || resp.status() == StatusCode::OK,
        "Expected 204 or 200, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_frame_out_of_range() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!(
                    "/dicomweb/studies/{}/series/{}/instances/{}/frames/9999",
                    ctx.uids.study_uid, ctx.uids.series_1_uid, ctx.uids.instance_1_1_uid
                ))
                .method("GET")
                .header("Accept", "image/jpeg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("frame out of range");
    assert!(
        resp.status().is_server_error()
            || resp.status().is_client_error()
            || resp.status() == StatusCode::NOT_ACCEPTABLE,
        "Expected error status, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_unsupported_accept_header() {
    let ctx = get_test_context().await;

    let resp = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/dicomweb/studies/{}", ctx.uids.study_uid))
                .method("GET")
                .header("Accept", "application/xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unsupported accept");
    // 406 or 200 depending on implementation
    assert!(
        resp.status() == StatusCode::NOT_ACCEPTABLE || resp.status() == StatusCode::OK,
        "Expected 406 or 200, got {}",
        resp.status()
    );
}

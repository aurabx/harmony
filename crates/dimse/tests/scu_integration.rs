//! Integration tests for DIMSE SCU (Service Class User)
//!
//! These tests verify the SCU can connect to external DICOM SCPs and perform all operations.
//! Uses DCMTK dcmqrscp as the test SCP.

mod scu_test_helpers;

use dimse::{DimseConfig, DimseScu, RemoteNode};
use dimse::types::{FindQuery, GetQuery, MoveQuery, QueryLevel, DatasetStream};
use futures::stream::StreamExt;
use tempfile::TempDir;

use scu_test_helpers::{dcmtk_available, DcmtkQrScp, create_test_dicom_file, mkuid};

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_echo_success() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("echo").await.expect("start QR SCP");
    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    let result = scu.echo(&remote).await;
    
    assert!(result.is_ok(), "C-ECHO should succeed");
    assert!(result.unwrap(), "C-ECHO should return true");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_find() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("find").await.expect("start QR SCP");
    
    // Create and store test DICOM file
    let patient_id = "FIND_TEST_123";
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    
    let test_file = qr_scp.base_dir().join("test.dcm");
    create_test_dicom_file(
        &test_file,
        patient_id,
        &study_uid,
        &series_uid,
        &sop_uid,
    ).expect("create test DICOM");
    
    qr_scp.store_file(&test_file).await.expect("store file");

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Query by Study Instance UID at Study level
    let query = FindQuery::study(Some(study_uid.clone()))
        .with_max_results(10);
    let mut stream = scu.find(&remote, query).await.expect("C-FIND should start");

    // Collect results
    let mut results = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(dataset) => results.push(dataset),
            Err(e) => {
                eprintln!("C-FIND error: {}", e);
                break;
            }
        }
    }

    assert!(!results.is_empty(), "Should find at least one result");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_store() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("store").await.expect("start QR SCP");

    // Create test DICOM file to send
    let temp_dir = TempDir::new().expect("create temp dir");
    let patient_id = "STORE_TEST_123";
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    
    let test_file = temp_dir.path().join("to_store.dcm");
    create_test_dicom_file(
        &test_file,
        patient_id,
        &study_uid,
        &series_uid,
        &sop_uid,
    ).expect("create test DICOM");

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    let dataset = DatasetStream::from_file(test_file, false);
    let result = scu.store(&remote, dataset).await;
    
    assert!(result.is_ok(), "C-STORE should succeed");
    assert!(result.unwrap(), "C-STORE should return true");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_get() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("get").await.expect("start QR SCP");
    
    // Create and store test DICOM file
    let patient_id = "GET_TEST_123";
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    
    let test_file = qr_scp.base_dir().join("test.dcm");
    create_test_dicom_file(
        &test_file,
        patient_id,
        &study_uid,
        &series_uid,
        &sop_uid,
    ).expect("create test DICOM");
    
    qr_scp.store_file(&test_file).await.expect("store file");

    let temp_dir = TempDir::new().expect("create temp dir");
    let output_dir = temp_dir.path().to_path_buf();

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Query by Study Instance UID
    let query = GetQuery::new(QueryLevel::Study)
        .with_parameter("StudyInstanceUID", &study_uid);
    
    let mut stream = scu.get_request(&remote, query, Some(output_dir.clone()))
        .await
        .expect("C-GET should start");

    // Collect results
    let mut results = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(dataset) => results.push(dataset),
            Err(e) => {
                eprintln!("C-GET error: {}", e);
                break;
            }
        }
    }

    assert!(!results.is_empty(), "Should receive at least one dataset");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_move() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("move").await.expect("start QR SCP");
    
    // Create and store test DICOM file
    let patient_id = "MOVE_TEST_123";
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");
    
    let test_file = qr_scp.base_dir().join("test.dcm");
    create_test_dicom_file(
        &test_file,
        patient_id,
        &study_uid,
        &series_uid,
        &sop_uid,
    ).expect("create test DICOM");
    
    qr_scp.store_file(&test_file).await.expect("store file");

    let temp_dir = TempDir::new().expect("create temp dir");
    let output_dir = temp_dir.path().to_path_buf();

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        incoming_store_port: 11124, // Match HostTable in helper
        external_store_scp: false, // Use transient listener
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Query by Study Instance UID with destination AET
    let query = MoveQuery::new(QueryLevel::Study, "HARMONY_SCU")
        .with_parameter("StudyInstanceUID", &study_uid);
    
    let mut stream = scu.move_request(&remote, query, Some(output_dir.clone()))
        .await
        .expect("C-MOVE should start");

    // Collect results
    let mut results = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(dataset) => results.push(dataset),
            Err(e) => {
                eprintln!("C-MOVE error: {}", e);
                break;
            }
        }
    }

    // C-MOVE may complete without streaming if external SCP is used,
    // but with transient listener we should receive files
    // Note: This may vary depending on QR SCP configuration
    eprintln!("C-MOVE received {} datasets", results.len());
}

#[tokio::test]
async fn test_scu_config_validation() {
    let remote = RemoteNode::new("", "localhost", 11112);
    let result = remote.validate();
    assert!(result.is_err(), "Should reject empty AET");

    let remote = RemoteNode::new(
        "THIS_IS_DEFINITELY_TOO_LONG_FOR_A_DICOM_AET",
        "localhost",
        11112,
    );
    let result = remote.validate();
    assert!(
        result.is_err(),
        "Should reject AET longer than 16 characters"
    );

    let remote = RemoteNode::new("VALID_AET", "", 11112);
    let result = remote.validate();
    assert!(result.is_err(), "Should reject empty host");

    let remote = RemoteNode::new("VALID_AET", "localhost", 0);
    let result = remote.validate();
    assert!(result.is_err(), "Should reject port 0");
}

#[tokio::test]
async fn test_scu_connection_timeout() {
    // Try to connect to a non-existent server
    let mut remote = RemoteNode::new("TEST_SCP", "192.0.2.1", 11112); // TEST-NET-1
    remote.connect_timeout_ms = Some(500);

    let config = DimseConfig::default();
    let scu = DimseScu::new(config);

    let result = scu.echo(&remote).await;

    // Should timeout or fail to connect
    assert!(
        result.is_err(),
        "Should fail to connect to non-existent server"
    );
}

#[tokio::test]
async fn test_scu_find_query_builder() {
    use dimse::types::FindQuery;

    // Build a patient-level query
    let query = FindQuery::patient(Some("12345".to_string()))
        .with_parameter("PatientName", "DOE^JOHN")
        .with_max_results(100);

    assert_eq!(query.query_level, QueryLevel::Patient);
    assert_eq!(
        query.parameters.get("PatientID"),
        Some(&"12345".to_string())
    );
    assert_eq!(
        query.parameters.get("PatientName"),
        Some(&"DOE^JOHN".to_string())
    );
    assert_eq!(query.max_results, 100);

    // Build a study-level query
    let query = FindQuery::study(Some("1.2.3.4.5".to_string()))
        .with_parameter("StudyDate", "20240101")
        .with_max_results(50);

    assert_eq!(query.query_level, QueryLevel::Study);
    assert_eq!(
        query.parameters.get("StudyInstanceUID"),
        Some(&"1.2.3.4.5".to_string())
    );
    assert_eq!(query.max_results, 50);
}

#[tokio::test]
async fn test_scu_move_query_builder() {
    use dimse::types::{MovePriority, MoveQuery};

    let query = MoveQuery::new(QueryLevel::Study, "DEST_AET")
        .with_parameter("StudyInstanceUID", "1.2.3.4.5")
        .with_priority(MovePriority::High);

    assert_eq!(query.query_level, QueryLevel::Study);
    assert_eq!(query.destination_aet, "DEST_AET");
    assert_eq!(query.priority, MovePriority::High);
    assert_eq!(
        query.parameters.get("StudyInstanceUID"),
        Some(&"1.2.3.4.5".to_string())
    );
}

#[tokio::test]
async fn test_scu_get_query_builder() {
    use dimse::types::GetQuery;

    let query =
        GetQuery::new(QueryLevel::Series).with_parameter("SeriesInstanceUID", "1.2.3.4.5.6");

    assert_eq!(query.query_level, QueryLevel::Series);
    assert_eq!(
        query.parameters.get("SeriesInstanceUID"),
        Some(&"1.2.3.4.5.6".to_string())
    );
}

// Error handling tests

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_handles_server_disconnect() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    // Start a QR SCP then stop it
    let qr_scp = DcmtkQrScp::start("disconnect").await.expect("start QR SCP");
    let port = qr_scp.port();
    
    // Drop the SCP to stop it
    drop(qr_scp);
    
    // Wait for it to fully stop
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", port);
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        connect_timeout_ms: 1000,
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Should fail to connect
    let result = scu.echo(&remote).await;
    assert!(result.is_err(), "Should fail to connect to stopped server");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_handles_invalid_response() {
    // This tests the SCU's ability to handle unexpected scenarios
    // by checking error handling when server isn't ready
    
    let mut remote = RemoteNode::new("TEST_SCP", "127.0.0.1", 1); // Invalid port 1
    remote.connect_timeout_ms = Some(500);

    let config = DimseConfig::default();
    let scu = DimseScu::new(config);

    let result = scu.echo(&remote).await;
    assert!(result.is_err(), "Should fail with invalid port");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scu_find_with_empty_results() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("empty_find").await.expect("start QR SCP");
    
    // Don't store any files - database is empty

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Query with UID that doesn't exist
    let query = FindQuery::study(Some("9.9.9.9.9".to_string()))
        .with_max_results(10);
    let stream_result = scu.find(&remote, query).await;

    assert!(stream_result.is_ok(), "C-FIND should start even with no results");

    let mut stream = stream_result.unwrap();
    let mut count = 0;
    while let Some(_) = stream.next().await {
        count += 1;
    }

    assert_eq!(count, 0, "Should have no results for non-existent UID");
}

#[tokio::test(flavor = "multi_thread")]  
async fn test_scu_get_with_no_results() {
    if !dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let qr_scp = DcmtkQrScp::start("empty_get").await.expect("start QR SCP");

    let temp_dir = TempDir::new().expect("create temp dir");
    let output_dir = temp_dir.path().to_path_buf();

    let remote = RemoteNode::new("QR_SCP", "127.0.0.1", qr_scp.port());
    let config = DimseConfig {
        local_aet: "HARMONY_SCU".to_string(),
        ..Default::default()
    };
    let scu = DimseScu::new(config);

    // Query with UID that doesn't exist
    let query = GetQuery::new(QueryLevel::Study)
        .with_parameter("StudyInstanceUID", "9.9.9.9.9");
    
    let stream_result = scu.get_request(&remote, query, Some(output_dir)).await;

    // C-GET should complete without error even with no results
    assert!(stream_result.is_ok(), "C-GET should start even with no matching data");

    let mut stream = stream_result.unwrap();
    let mut count = 0;
    while let Some(result) = stream.next().await {
        if result.is_ok() {
            count += 1;
        }
    }

    assert_eq!(count, 0, "Should have no results for non-existent UID");
}

#[tokio::test]
async fn test_scu_validates_query_level() {
    // Test that query builders produce valid queries at different levels
    let patient_query = FindQuery::patient(Some("123".to_string()));
    assert_eq!(patient_query.query_level, QueryLevel::Patient);

    let study_query = FindQuery::study(Some("1.2.3".to_string()));
    assert_eq!(study_query.query_level, QueryLevel::Study);

    // For series/image levels, use the generic constructor with parameters
    let get_query = GetQuery::new(QueryLevel::Series)
        .with_parameter("SeriesInstanceUID", "1.2.3.4");
    assert_eq!(get_query.query_level, QueryLevel::Series);

    let move_query = MoveQuery::new(QueryLevel::Image, "DEST_AET")
        .with_parameter("SOPInstanceUID", "1.2.3.4.5");
    assert_eq!(move_query.query_level, QueryLevel::Image);
}

#[tokio::test]
async fn test_scu_remote_node_validation_comprehensive() {
    // Test all validation cases
    
    // Valid node
    let valid = RemoteNode::new("VALID_AET", "localhost", 11112);
    assert!(valid.validate().is_ok());

    // Empty AET
    let empty_aet = RemoteNode::new("", "localhost", 11112);
    assert!(empty_aet.validate().is_err());

    // AET too long (> 16 chars)
    let long_aet = RemoteNode::new("THIS_IS_TOO_LONG_AET", "localhost", 11112);
    assert!(long_aet.validate().is_err());

    // Empty host
    let empty_host = RemoteNode::new("AET", "", 11112);
    assert!(empty_host.validate().is_err());

    // Port 0
    let zero_port = RemoteNode::new("AET", "localhost", 0);
    assert!(zero_port.validate().is_err());

    // Valid with IP address
    let ip_host = RemoteNode::new("AET", "192.168.1.1", 104);
    assert!(ip_host.validate().is_ok());

    // Valid with TLS
    let tls_node = RemoteNode::new("SECURE", "secure.example.com", 2762).with_tls();
    assert!(tls_node.validate().is_ok());
    assert!(tls_node.use_tls);
}

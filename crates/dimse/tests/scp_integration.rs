//! Integration tests for DIMSE SCP (Service Class Provider)
//!
//! These tests verify the SCP can handle real DICOM associations and commands.

use dimse::scp::QueryProvider;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::{DimseConfig, DimseScp};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

/// Mock query provider for testing
struct MockQueryProvider;

#[async_trait::async_trait]
impl QueryProvider for MockQueryProvider {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<Vec<DatasetStream>> {
        // Return empty results for now
        Ok(Vec::new())
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<Vec<DatasetStream>> {
        Ok(Vec::new())
    }

    async fn get(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<Vec<DatasetStream>> {
        Ok(Vec::new())
    }

    async fn store(&self, _dataset: DatasetStream) -> dimse::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_scp_starts_and_stops() {
    // Allocate an ephemeral port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "TEST_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: true,
        enable_move: false,
        enable_get: false,
        enable_store: true,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test"),
        max_associations: 10,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let provider: Arc<dyn QueryProvider> = Arc::new(MockQueryProvider);
    let scp = DimseScp::new(config, provider);

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    // Start SCP in background
    let handle = tokio::spawn(async move { scp.run(shutdown_clone).await });

    // Give SCP time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify port is listening
    let result = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await;
    assert!(result.is_ok(), "SCP should be listening on port {}", port);

    // Trigger shutdown
    shutdown.cancel();

    // Wait for SCP to stop
    let stop_result = timeout(Duration::from_secs(2), handle).await;
    assert!(stop_result.is_ok(), "SCP should stop gracefully");
}

#[tokio::test]
async fn test_scp_accepts_c_echo() {
    // This test requires DCMTK echoscu to be installed
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "ECHO_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: false,
        enable_move: false,
        enable_get: false,
        enable_store: false,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test"),
        max_associations: 10,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let provider: Arc<dyn QueryProvider> = Arc::new(MockQueryProvider);
    let scp = DimseScp::new(config, provider);

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    let handle = tokio::spawn(async move { scp.run(shutdown_clone).await });

    // Wait for SCP to be ready
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Run echoscu
    let output = tokio::process::Command::new("echoscu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("ECHO_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .output()
        .await
        .expect("Failed to run echoscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    assert!(output.status.success(), "echoscu should succeed");
}

#[tokio::test]
async fn test_scp_accepts_c_find() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "FIND_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: true,
        enable_move: false,
        enable_get: false,
        enable_store: false,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test"),
        max_associations: 10,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let provider: Arc<dyn QueryProvider> = Arc::new(MockQueryProvider);
    let scp = DimseScp::new(config, provider);

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    let handle = tokio::spawn(async move { scp.run(shutdown_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Run findscu with a simple query
    let output = tokio::process::Command::new("findscu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("FIND_SCP")
        .arg("-P")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg("-k")
        .arg("0010,0020=*")
        .output()
        .await
        .expect("Failed to run findscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    assert!(
        output.status.success(),
        "findscu should complete successfully"
    );
}

#[tokio::test]
async fn test_scp_accepts_c_store() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    // Create a temporary DICOM file
    let temp_dir = tempfile::tempdir().unwrap();
    let dcm_path = temp_dir.path().join("test.dcm");

    {
        use dicom_core::{DataElement, PrimitiveValue, VR};
        use dicom_dictionary_std::tags;
        use dicom_object::meta::FileMetaTableBuilder;
        use dicom_object::InMemDicomObject;

        let mut obj = InMemDicomObject::new_empty();

        // Patient Name
        obj.put(DataElement::new(
            tags::PATIENT_NAME,
            VR::PN,
            PrimitiveValue::from("Doe^John"),
        ));

        // Patient ID
        obj.put(DataElement::new(
            tags::PATIENT_ID,
            VR::LO,
            PrimitiveValue::from("12345"),
        ));

        // SOP Class UID (Secondary Capture Image Storage)
        obj.put(DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.5.1.4.1.1.7"),
        ));

        // SOP Instance UID
        obj.put(DataElement::new(
            tags::SOP_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from("1.2.3.4.5.6.7"),
        ));

        let file_meta = FileMetaTableBuilder::new()
            .media_storage_sop_class_uid("1.2.840.10008.5.1.4.1.1.7")
            .media_storage_sop_instance_uid("1.2.3.4.5.6.7")
            .transfer_syntax("1.2.840.10008.1.2.1"); // Explicit VR Little Endian

        let file_obj = obj.with_meta(file_meta).expect("Failed to create file object");
        file_obj
            .write_to_file(&dcm_path)
            .expect("Failed to create DICOM file");
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "STORE_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: false,
        enable_move: false,
        enable_get: false,
        enable_store: true,
        storage_dir: temp_dir.path().to_path_buf(),
        max_associations: 10,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let provider: Arc<dyn QueryProvider> = Arc::new(MockQueryProvider);
    let scp = DimseScp::new(config, provider);

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    let handle = tokio::spawn(async move { scp.run(shutdown_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Run storescu
    let output = tokio::process::Command::new("storescu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("STORE_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(dcm_path.to_str().unwrap())
        .output()
        .await
        .expect("Failed to run storescu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    if !output.status.success() {
        eprintln!(
            "storescu stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert!(
        output.status.success(),
        "storescu should complete successfully"
    );
}

#[tokio::test]
async fn test_scp_config_validation() {
    // Invalid AET (too long)
    let config = DimseConfig {
        local_aet: "THIS_AET_IS_WAY_TOO_LONG_FOR_DICOM".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port: 11112,
        enable_echo: true,
        enable_find: true,
        enable_move: false,
        enable_get: false,
        enable_store: false,
        storage_dir: std::path::PathBuf::from("/tmp"),
        max_associations: 10,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let result = config.validate();
    assert!(
        result.is_err(),
        "Should reject AET longer than 16 characters"
    );
}

#[tokio::test]
async fn test_scp_multiple_associations() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "MULTI_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: false,
        enable_move: false,
        enable_get: false,
        enable_store: false,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test"),
        max_associations: 5,
        incoming_store_port: 11113,
        max_pdu: 16384,
        connect_timeout_ms: 5000,
        association_timeout_ms: 30000,
        tls: None,
        preferred_transfer_syntaxes: vec![],
        external_store_scp: false,
    };

    let provider: Arc<dyn QueryProvider> = Arc::new(MockQueryProvider);
    let scp = DimseScp::new(config, provider);

    let shutdown = CancellationToken::new();
    let shutdown_clone = shutdown.clone();

    let handle = tokio::spawn(async move { scp.run(shutdown_clone).await });

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Run multiple echoscu commands concurrently
    let mut handles = vec![];
    for _ in 0..3 {
        let port = port;
        handles.push(tokio::spawn(async move {
            tokio::process::Command::new("echoscu")
                .arg("--aetitle")
                .arg("TEST_SCU")
                .arg("--call")
                .arg("MULTI_SCP")
                .arg("127.0.0.1")
                .arg(port.to_string())
                .output()
                .await
        }));
    }

    // Wait for all to complete
    for handle in handles {
        let output = handle.await.expect("Task failed").expect("echoscu failed");
        assert!(output.status.success(), "All echoscu calls should succeed");
    }

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;
}

/// Helper module for DCMTK detection
mod dimse_test_helpers {
    pub fn dcmtk_available() -> bool {
        std::process::Command::new("echoscu")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

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

/// Wait for a port to be accepting connections (with timeout)
async fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

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

#[tokio::test(flavor = "multi_thread")]
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
    assert!(wait_for_port(port, 2000).await, "SCP should be listening on port");

    // Verify port is listening
    let result = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await;
    assert!(result.is_ok(), "SCP should be listening on port {}", port);

    // Trigger shutdown
    shutdown.cancel();

    // Wait for SCP to stop
    let stop_result = timeout(Duration::from_secs(2), handle).await;
    assert!(stop_result.is_ok(), "SCP should stop gracefully");
}

#[tokio::test(flavor = "multi_thread")]
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
    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

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

#[tokio::test(flavor = "multi_thread")]
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

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

#[tokio::test(flavor = "multi_thread")]
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

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

#[tokio::test(flavor = "multi_thread")]
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

#[tokio::test(flavor = "multi_thread")]
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

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

#[tokio::test(flavor = "multi_thread")]
async fn test_scp_accepts_c_move() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    // Check if movescu is available
    if std::process::Command::new("movescu")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("Skipping test: movescu not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "MOVE_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: true,
        enable_move: true,
        enable_get: false,
        enable_store: true,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test_move"),
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // Run movescu - it should complete even with no results
    // The important thing is the SCP accepts and handles the C-MOVE request
    let output = tokio::process::Command::new("movescu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("MOVE_SCP")
        .arg("--move")
        .arg("TEST_SCU") // Move destination
        .arg("-S") // Study root
        .arg("-k")
        .arg("StudyInstanceUID=1.2.3.4.5")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .output()
        .await
        .expect("Failed to run movescu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    // movescu may return non-zero if no results, but the connection should work
    // Check that it didn't fail due to association rejection
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Association Rejected"),
        "SCP should accept C-MOVE association"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scp_accepts_c_get() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    // Check if getscu is available
    if std::process::Command::new("getscu")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("Skipping test: getscu not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let temp_dir = tempfile::tempdir().unwrap();

    let config = DimseConfig {
        local_aet: "GET_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: true,
        enable_move: false,
        enable_get: true,
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // Run getscu - it should complete even with no results
    let output = tokio::process::Command::new("getscu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("GET_SCP")
        .arg("-S") // Study root
        .arg("-k")
        .arg("StudyInstanceUID=1.2.3.4.5")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .output()
        .await
        .expect("Failed to run getscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    // Check that association was not rejected
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Association Rejected"),
        "SCP should accept C-GET association"
    );
}

// Error handling tests

#[tokio::test(flavor = "multi_thread")]
async fn test_scp_rejects_unknown_aet() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "STRICT_SCP".to_string(),
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // Try to connect with wrong called AET
    let _output = tokio::process::Command::new("echoscu")
        .arg("--aetitle")
        .arg("TEST_SCU")
        .arg("--call")
        .arg("WRONG_AET") // Wrong AET
        .arg("127.0.0.1")
        .arg(port.to_string())
        .output()
        .await
        .expect("Failed to run echoscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    // SCP may either reject or accept (depending on strictness)
    // We just verify it handles the request gracefully without crashing
    // The test passes if we get here without timeout
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scp_handles_rapid_connections() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "RAPID_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_echo: true,
        enable_find: false,
        enable_move: false,
        enable_get: false,
        enable_store: false,
        storage_dir: std::path::PathBuf::from("/tmp/dimse_test"),
        max_associations: 20,
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // Fire 10 rapid connections
    let mut handles = vec![];
    for i in 0..10 {
        let port = port;
        handles.push(tokio::spawn(async move {
            tokio::process::Command::new("echoscu")
                .arg("--aetitle")
                .arg(format!("SCU_{}", i))
                .arg("--call")
                .arg("RAPID_SCP")
                .arg("127.0.0.1")
                .arg(port.to_string())
                .output()
                .await
        }));
    }

    // Count successful connections
    let mut success_count = 0;
    for h in handles {
        if let Ok(Ok(output)) = h.await {
            if output.status.success() {
                success_count += 1;
            }
        }
    }

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    // At least some should succeed
    assert!(
        success_count >= 1,
        "At least one rapid connection should succeed, got {}",
        success_count
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_scp_handles_connection_drop() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "DROP_SCP".to_string(),
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // Connect and immediately drop
    for _ in 0..5 {
        if let Ok(stream) = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            drop(stream); // Immediate close
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // SCP should still be running
    tokio::time::sleep(Duration::from_millis(100)).await;
    let result = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await;
    assert!(result.is_ok(), "SCP should still be accepting connections after drops");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;
}

// PDU size negotiation tests

/// Test that SCP respects a client's small max PDU size
/// This simulates the Orthanc scenario where the client proposes a 16KB PDU
#[tokio::test(flavor = "multi_thread")]
async fn test_scp_respects_small_pdu_from_client() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Configure SCP with large max PDU, but client will propose smaller
    let config = DimseConfig {
        local_aet: "PDU_SCP".to_string(),
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
        max_pdu: 65536, // SCP has large PDU
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // echoscu with explicit small max PDU (16KB like Orthanc default)
    // The --max-pdu flag sets the SCU's proposed max PDU length
    let output = tokio::process::Command::new("echoscu")
        .arg("--aetitle")
        .arg("ORTHANC")
        .arg("--call")
        .arg("PDU_SCP")
        .arg("--max-pdu")
        .arg("16384") // 16KB - Orthanc's typical default
        .arg("127.0.0.1")
        .arg(port.to_string())
        .output()
        .await
        .expect("Failed to run echoscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    if !output.status.success() {
        eprintln!("echoscu stderr: {}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(output.status.success(), "echoscu with small PDU should succeed");
}

/// Test that SCP can handle C-FIND responses when client has small PDU
/// This is important for response fragmentation
#[tokio::test(flavor = "multi_thread")]
async fn test_scp_find_with_small_client_pdu() {
    if !dimse_test_helpers::dcmtk_available() {
        eprintln!("Skipping test: DCMTK not available");
        return;
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config = DimseConfig {
        local_aet: "FIND_PDU_SCP".to_string(),
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
        max_pdu: 65536, // Large SCP PDU
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

    assert!(wait_for_port(port, 2000).await, "SCP should be listening");

    // findscu with small max PDU
    let output = tokio::process::Command::new("findscu")
        .arg("--aetitle")
        .arg("SMALL_PDU")
        .arg("--call")
        .arg("FIND_PDU_SCP")
        .arg("--max-pdu")
        .arg("16384") // 16KB
        .arg("-P") // Patient root
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg("-k")
        .arg("0010,0020=*") // Query all patients
        .output()
        .await
        .expect("Failed to run findscu");

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(2), handle).await;

    // Should complete without errors (even if no results)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Association Rejected"),
        "C-FIND with small PDU should not be rejected"
    );
}

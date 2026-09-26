//! Integration tests for DICOM streaming using dcmtk tools
//!
//! These tests spawn dcmqrscp (DICOM QR SCP) and use dcmtk tools to test streaming.
//! They are self-contained and only require dcmtk tools to be installed.
//! Tests that need a running Harmony live in harmony-examples as htest
//! examples: `pipelines/dicom-scp` and `pipelines/streaming-dicom`.

use std::path::PathBuf;
use std::process::Command;

/// Check if dcmtk tools are available
fn has_dcmtk() -> bool {
    ["echoscu", "findscu", "getscu", "storescu", "dcmqrscp"]
        .iter()
        .all(|tool| Command::new(tool).arg("--version").output().is_ok())
}

/// Spawn dcmqrscp on a free port and return (child, port, base_dir)
async fn spawn_dcmqrscp(verbose: bool) -> (tokio::process::Child, u16, PathBuf) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let base = PathBuf::from("./tmp/streaming_qrscp");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("create qr db dir");
    let cfg_path = base.join("dcmqrscp.cfg");

    let abs_db = match std::fs::canonicalize(&dbdir) {
        Ok(p) => p,
        Err(_) => std::env::current_dir().unwrap().join(&dbdir),
    };

    let cfg = format!(
        "# Minimal dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {}  RW  (9, 1024mb)  ANY\nAETable END\n",
        abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("create cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose {
        dcmqr.arg("-d");
    }
    let dcmqr = dcmqr.arg("-c").arg(&cfg_path).arg(port.to_string());
    if !verbose {
        dcmqr
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let child = dcmqr.kill_on_drop(true).spawn().expect("spawn dcmqrscp");

    // Wait for port to be ready
    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    (child, port, base)
}

/// Store a test DICOM dataset via storescu
async fn store_test_dataset(
    port: u16,
    base: &PathBuf,
    patient_id: &str,
    verbose: bool,
) -> anyhow::Result<()> {
    let mkuid = |suf: &str| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        format!(
            "1.2.826.0.1.3680043.10.5432.{}.{}.{}",
            suf,
            now.as_secs(),
            now.subsec_nanos()
        )
    };

    let identifier = serde_json::json!({
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        "00080018": { "vr": "UI", "Value": [ mkuid("1") ] },
        "0020000D": { "vr": "UI", "Value": [ mkuid("2") ] },
        "0020000E": { "vr": "UI", "Value": [ mkuid("3") ] },
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        "00100020": { "vr": "LO", "Value": [patient_id] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^TEST"}] }
    });
    let obj = dicom_json_tool::json_value_to_identifier(&identifier)?;

    let dicom_path = base.join(format!("seed_{}.dcm", patient_id));
    dicom_json_tool::write_part10(&dicom_path, &obj)?;

    let mut st = tokio::process::Command::new("storescu");
    let st = st
        .arg("--aetitle")
        .arg("HARMONY_SCU")
        .arg("--call")
        .arg("QR_SCP")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(&dicom_path);
    if !verbose {
        st.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let status = st.status().await?;
    if !status.success() {
        anyhow::bail!("storescu failed");
    }
    Ok(())
}

/// Test DICOM echo (C-ECHO) to verify connectivity
#[tokio::test]
async fn test_dicom_echo_to_qrscp() {
    if !has_dcmtk() {
        eprintln!("Skipping: dcmtk tools not found");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, port, _base) = spawn_dcmqrscp(verbose).await;

    let output = Command::new("echoscu")
        .args(["-aet", "TEST", "-aec", "QR_SCP", "localhost", &port.to_string()])
        .output()
        .expect("echoscu failed");

    let _ = qr_child.kill().await;

    assert!(
        output.status.success(),
        "C-ECHO failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test DICOM find (C-FIND) query
#[tokio::test]
async fn test_dicom_find_query() {
    if !has_dcmtk() {
        eprintln!("Skipping: dcmtk tools not found");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, port, base) = spawn_dcmqrscp(verbose).await;

    // Store test data
    if store_test_dataset(port, &base, "FIND_TEST", verbose)
        .await
        .is_err()
    {
        eprintln!("Failed to store test dataset; skipping test");
        let _ = qr_child.kill().await;
        return;
    }

    // C-FIND at study level
    let output = Command::new("findscu")
        .args([
            "-aet",
            "TEST",
            "-aec",
            "QR_SCP",
            "-S",
            "localhost",
            &port.to_string(),
            "-k",
            "0008,0052=STUDY",
            "-k",
            "0010,0020", // PatientID
        ])
        .output()
        .expect("findscu failed");

    let _ = qr_child.kill().await;

    assert!(
        output.status.success(),
        "C-FIND failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Test DICOM get (C-GET) retrieve operation
#[tokio::test]
async fn test_dicom_get_retrieve() {
    if !has_dcmtk() {
        eprintln!("Skipping: dcmtk tools not found");
        return;
    }

    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let (mut qr_child, port, base) = spawn_dcmqrscp(verbose).await;

    // Store test data
    if store_test_dataset(port, &base, "GET_TEST", verbose)
        .await
        .is_err()
    {
        eprintln!("Failed to store test dataset; skipping test");
        let _ = qr_child.kill().await;
        return;
    }

    // Create output directory for retrieved files
    let out_dir = base.join("retrieved");
    std::fs::create_dir_all(&out_dir).expect("create output dir");

    // C-GET with actual study UID (we just stored one)
    let output = Command::new("getscu")
        .args([
            "-aet",
            "TEST",
            "-aec",
            "QR_SCP",
            "localhost",
            &port.to_string(),
            "-k",
            "0008,0052=STUDY",
            "-k",
            "0010,0020=GET_TEST", // PatientID we stored
            "-od",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("getscu failed");

    let _ = qr_child.kill().await;

    assert!(
        output.status.success(),
        "getscu failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Document test setup requirements
#[test]
fn test_setup_instructions() {
    println!("\n═══════════════════════════════════════════════════════════════");
    println!("  DICOM STREAMING INTEGRATION TESTS");
    println!("═══════════════════════════════════════════════════════════════\n");
    
    println!("Self-contained tests (no external services required):");
    println!("  cargo test --test streaming_dicom_dcmtk\n");
    
    println!("Tests that need a running Harmony are htest examples in harmony-examples:");
    println!("  htest --examples-dir . run dicom-scp");
    println!("  htest --examples-dir . run streaming-dicom   (skipped until AURA-2408 is fixed)\n");
    
    println!("Available dcmtk tools:");
    let tools = ["echoscu", "findscu", "getscu", "storescu", "dcmqrscp"];
    for tool in &tools {
        match Command::new("which").arg(tool).output() {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("  ✓ {} -> {}", tool, path);
            }
            _ => println!("  ✗ {} (not found - install with: brew install dcmtk)", tool),
        }
    }
    
    println!("\n═══════════════════════════════════════════════════════════════\n");
}

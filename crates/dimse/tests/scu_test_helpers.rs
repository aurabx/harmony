//! Helper utilities for SCU integration tests

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;

// Global lock to serialize dcmqrscp execution across tests
static DCMTK_LOCK: Mutex<()> = Mutex::const_new(());

/// Check if DCMTK tools are available
pub fn dcmtk_available() -> bool {
    use std::process::Command as StdCommand;
    
    for tool in ["dcmqrscp", "storescu", "dcmqridx", "getscu"].iter() {
        if StdCommand::new(tool)
            .arg("--version")
            .output()
            .is_err()
        {
            return false;
        }
    }
    true
}

/// Generate a unique UID for test data
pub fn mkuid(suffix: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    format!(
        "1.2.826.0.1.3680043.10.5432.{}.{}.{}",
        suffix,
        now.as_secs(),
        now.subsec_nanos()
    )
}

/// DCMTK QR SCP test server
pub struct DcmtkQrScp {
    child: tokio::process::Child,
    port: u16,
    base_dir: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl DcmtkQrScp {
    /// Start a DCMTK QR SCP server on a random port
    pub async fn start(test_name: &str) -> anyhow::Result<Self> {
        // Acquire global lock to prevent parallel execution
        let guard = DCMTK_LOCK.lock().await;

        // Retry loop for starting dcmqrscp (handles port binding race conditions)
        for _ in 0..5 {
            // Pick a free port
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            drop(listener);

            // Prepare storage directory and config
            let base = PathBuf::from("../../tmp").join(format!("qrscp_{}_{}", test_name, Uuid::new_v4()));
            let dbdir = base.join("qrdb");
            std::fs::create_dir_all(&dbdir)?;
            let cfg_path = base.join("dcmqrscp.cfg");

            // Use absolute path for database directory
            let abs_db = std::fs::canonicalize(&dbdir)
                .unwrap_or_else(|_| std::env::current_dir().unwrap().join(&dbdir));

            // Create minimal config with HostTable entry for SCU
            let cfg = format!(
                "# Minimal dcmqrscp.cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHARMONY_SCU = (HARMONY_SCU, 127.0.0.1, 11124)\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
                db = abs_db.to_string_lossy()
            );
            std::fs::create_dir_all(&base)?;
            std::fs::write(&cfg_path, cfg)?;

            // Start dcmqrscp
            let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
            let mut dcmqr = Command::new("dcmqrscp");
            if verbose {
                dcmqr.arg("-d");
            }
            dcmqr
                .arg("-c")
                .arg(&cfg_path)
                .arg(port.to_string())
                .kill_on_drop(true);
            
            if !verbose {
                dcmqr.stdout(Stdio::null()).stderr(Stdio::null());
            }

            let mut child = dcmqr.spawn()?;

            // Wait for port to be ready
            let mut ready = false;
            for _ in 0..20 { // 2 seconds max
                // Check if child has exited early
                if let Ok(Some(_)) = child.try_wait() {
                    // Exited early, probably port conflict
                    break;
                }

                if tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .is_ok()
                {
                    ready = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            if ready {
                return Ok(Self {
                    child,
                    port,
                    base_dir: base,
                    _guard: guard,
                });
            } else {
                // Cleanup and retry
                let _ = child.kill().await;
            }
        }
        
        anyhow::bail!("Failed to start dcmqrscp after multiple attempts");
    }

    /// Get the port the server is listening on
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the base directory
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Store a DICOM file into the QR SCP using storescu
    pub async fn store_file(&self, dicom_file: &Path) -> anyhow::Result<()> {
        let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
        let mut cmd = Command::new("storescu");
        cmd.arg("--aetitle")
            .arg("HARMONY_SCU")
            .arg("--call")
            .arg("QR_SCP")
            .arg("127.0.0.1")
            .arg(self.port.to_string())
            .arg(dicom_file);
        
        if !verbose {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }

        let status = cmd.status().await?;
        if !status.success() {
            anyhow::bail!("storescu failed");
        }
        
        // Wait a bit for indexing and then run dcmqridx if available
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        // Try to index the database (dcmqridx might not always be needed, but helps)
        if std::process::Command::new("dcmqridx")
            .arg("--version")
            .output()
            .is_ok()
        {
            let dbdir = self.base_dir.join("qrdb");
            if dbdir.exists() {
                let mut idx_cmd = Command::new("dcmqridx");
                idx_cmd.arg(&dbdir);
                if !verbose {
                    idx_cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
                let _ = idx_cmd.status().await; // Ignore errors, indexing might not always be needed
            }
        }
        
        // Wait a bit more for indexing to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        Ok(())
    }
}

impl Drop for DcmtkQrScp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Cleanup directory if needed (optional, let OS handle it)
    }
}

/// Create a minimal test DICOM file
pub fn create_test_dicom_file(
    output_path: &Path,
    patient_id: &str,
    study_uid: &str,
    series_uid: &str,
    sop_instance_uid: &str,
) -> anyhow::Result<()> {
    use dicom_json_tool;

    let identifier = serde_json::json!({
        // SOP Class: Secondary Capture Image Storage
        "00080016": { "vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.7"] },
        // SOP Instance UID
        "00080018": { "vr": "UI", "Value": [ sop_instance_uid ] },
        // Study/Series Instance UIDs
        "0020000D": { "vr": "UI", "Value": [ study_uid ] },
        "0020000E": { "vr": "UI", "Value": [ series_uid ] },
        // Modality
        "00080060": { "vr": "CS", "Value": [ "OT" ] },
        // Patient ID / Name
        "00100020": { "vr": "LO", "Value": [patient_id] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": format!("TEST^{}", patient_id)}] }
    });

    let obj = dicom_json_tool::json_value_to_identifier(&identifier)?;
    dicom_json_tool::write_part10(output_path, &obj)?;
    Ok(())
}


use crate::integrations::dimse::pipeline_query_provider::PipelineQueryProvider;
use dimse::{DimseConfig, DEFAULT_DIMSE_PORT};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;

static STARTED_SCP: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub fn ensure_dimse_scp_started(
    endpoint_name: &str,
    pipeline_name: &str,
    options: &HashMap<String, serde_json::Value>,
) {
    let local_aet = options
        .get("local_aet")
        .and_then(|v| v.as_str())
        .unwrap_or("HARMONY_SCP")
        .to_string();

    let bind_addr = options
        .get("bind_addr")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<IpAddr>().ok())
        .unwrap_or_else(|| IpAddr::from(std::net::Ipv4Addr::new(0, 0, 0, 0)));

    let port = options
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .unwrap_or(DEFAULT_DIMSE_PORT);

    let key = format!("{}@{}:{}#{}", local_aet, bind_addr, port, endpoint_name);
    {
        let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
        if guard.contains(&key) {
            return;
        }
        guard.insert(key.clone());
    }

    let mut dimse_config = DimseConfig {
        local_aet: local_aet.clone(),
        bind_addr,
        port,
        ..Default::default()
    };

    // Determine storage_dir: prefer options, else storage adapter, else ./tmp/dimse
    if let Some(dir) = options.get("storage_dir").and_then(|v| v.as_str()) {
        dimse_config.storage_dir = std::path::PathBuf::from(dir);
    } else if let Some(storage) = crate::globals::get_storage() {
        let p = storage
            .ensure_dir_str("dimse")
            .unwrap_or_else(|_| std::path::PathBuf::from("./tmp/dimse"));
        dimse_config.storage_dir = p;
    } else {
        dimse_config.storage_dir = std::path::PathBuf::from("./tmp/dimse");
    }

    // Feature toggles
    if let Some(b) = options.get("enable_echo").and_then(|v| v.as_bool()) {
        dimse_config.enable_echo = b;
    }
    if let Some(b) = options.get("enable_find").and_then(|v| v.as_bool()) {
        dimse_config.enable_find = b;
    }
    if let Some(b) = options.get("enable_move").and_then(|v| v.as_bool()) {
        dimse_config.enable_move = b;
    }

    let pipeline = pipeline_name.to_string();
    let endpoint = endpoint_name.to_string();

    // If requested, use DCMTK storescp instead of the internal stub SCP.
    // Only default to DCMTK when this is a backend persistent Store SCP scenario.
    // For endpoint SCPs (SCP mode), prefer the internal stub (supports port=0 ephemeral bind).
    let is_persistent_backend = options
        .get("persistent_store_scp")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let use_dcmtk_store = options
        .get("use_dcmtk_store")
        .and_then(|v| v.as_bool())
        .unwrap_or(is_persistent_backend);

    if use_dcmtk_store {
        let storage_dir = dimse_config.storage_dir.clone();
        let pipeline2 = pipeline.clone();
        let endpoint2 = endpoint.clone();
        tokio::spawn(async move {
            use tokio::process::Command;
            let _ = tokio::fs::create_dir_all(&storage_dir).await;

            // Try to start DCMTK storescp
            let mut cmd = Command::new("storescp");
            cmd.arg("-v")
                .arg("-od")
                .arg(storage_dir.to_string_lossy().to_string())
                .arg("-aet")
                .arg(local_aet.clone())
                .arg(port.to_string());
            tracing::info!("Starting DCMTK storescp AET='{}' on :{} -> {}", local_aet, port, storage_dir.display());
            match cmd.spawn() {
                Ok(mut child) => {
                    if let Err(e) = child.wait().await {
                        tracing::error!("storescp exited with error: {}", e);
                    } else {
                        tracing::info!("storescp exited");
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to spawn storescp: {} — falling back to internal SCP", e);
                    // Fallback to internal stub SCP so at least a listener exists
                    let provider: Arc<dyn dimse::scp::QueryProvider> =
                        Arc::new(PipelineQueryProvider::new(pipeline2, endpoint2));
                    let scp = dimse::DimseScp::new(dimse_config.clone(), provider);
                    if let Err(e2) = scp.run().await {
                        tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e2);
                    } else {
                        tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
                    }
                }
            }
            let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
            guard.retain(|k| k != &key);
        });
        // Readiness loop (best-effort): try to connect to the listening port
        {
            let target = if bind_addr.is_unspecified() {
                format!("127.0.0.1:{}", port)
            } else {
                format!("{}:{}", bind_addr, port)
            };
            for _ in 0..40 {
                if std::net::TcpStream::connect(&target).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    } else {
        tokio::spawn(async move {
            let provider: Arc<dyn dimse::scp::QueryProvider> =
                Arc::new(PipelineQueryProvider::new(pipeline, endpoint));
            let scp = dimse::DimseScp::new(dimse_config.clone(), provider);
            if let Err(e) = scp.run().await {
                tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e);
                let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
                guard.retain(|k| k != &key);
            } else {
                tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
                let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
                guard.retain(|k| k != &key);
            }
        });
        // Readiness loop (internal SCP)
        {
            let target = if bind_addr.is_unspecified() {
                format!("127.0.0.1:{}", port)
            } else {
                format!("{}:{}", bind_addr, port)
            };
            for _ in 0..40 {
                if std::net::TcpStream::connect(&target).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}

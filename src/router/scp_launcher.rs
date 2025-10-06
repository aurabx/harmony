use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::net::IpAddr;
use dimse::{DimseConfig, DEFAULT_DIMSE_PORT};
use crate::integrations::dimse::pipeline_query_provider::PipelineQueryProvider;

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

    if let Some(dir) = options.get("storage_dir").and_then(|v| v.as_str()) {
        dimse_config.storage_dir = std::path::PathBuf::from(dir);
    }

    // Feature toggles
    if let Some(b) = options.get("enable_echo").and_then(|v| v.as_bool()) { dimse_config.enable_echo = b; }
    if let Some(b) = options.get("enable_find").and_then(|v| v.as_bool()) { dimse_config.enable_find = b; }
    if let Some(b) = options.get("enable_move").and_then(|v| v.as_bool()) { dimse_config.enable_move = b; }

    let pipeline = pipeline_name.to_string();

    tokio::spawn(async move {
        let provider: Arc<dyn dimse::scp::QueryProvider> = Arc::new(PipelineQueryProvider::new(pipeline));
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
}
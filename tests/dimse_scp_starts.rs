use harmony::config::config::{Config, ConfigError};
use std::sync::Arc;
use tokio::net::TcpStream;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn dimse_scp_starts_for_dicom_endpoint() {
    // Pick a free local port by binding to port 0 first
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephem port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Build a config where a pipeline references a DICOM endpoint (SCP)
    let toml = format!(
        r#"
        [proxy]
        id = "dimse-scp-test"
        log_level = "info"
        store_dir = "/tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080

        [pipelines.dicom_scp_demo]
        description = "SCP demo"
        networks = ["default"]
        endpoints = ["dicom_scp"]
        backends = []
        middleware = []

        [endpoints.dicom_scp]
        service = "dicom"
        
        [endpoints.dicom_scp.options]
        local_aet = "HARMONY_SCP"
        bind_addr = "127.0.0.1"
        port = {port}

        [services.dicom]
        module = ""
    "#
    );

    let cfg: Config = load_config_from_str(&toml).expect("valid config");

    // Build the network router; this should trigger SCP startup for dicom_scp_demo
    let _app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Retry-connect to the SCP listener to verify it's listening
    let addr = format!("127.0.0.1:{}", port);
    let mut last_err: Option<std::io::Error> = None;
    let mut connected = false;
    for _ in 0..30 {
        match TcpStream::connect(&addr).await {
            Ok(_s) => {
                connected = true;
                break;
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(
        connected,
        "Failed to connect to DIMSE SCP on {}: {:?}",
        addr, last_err
    );
}

//! A `dicom_scp` pipeline's IP rules decide who may open an association.
//!
//! The check runs before the association is accepted. A denied peer gets an
//! A-ASSOCIATE-RJ, so it cannot get a C-ECHO reply, which never reaches the
//! pipeline.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use dimse::{DimseConfig, DimseError, DimseScp, DimseScu, RemoteNode};
use harmony::adapters::dimse::query_provider::PipelineQueryProvider;
use harmony::config::config::Config;
use tokio_util::sync::CancellationToken;

/// `deny_local` denies 127.0.0.0/8. `allow_local` allows only 127.0.0.1.
/// `office_only` allows only 192.168.0.0/16, so a local peer is outside it.
fn admission_config() -> Config {
    let toml = r#"
[proxy]
id = "dimse-peer-admission"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 18080

[pipelines.deny_local]
networks = ["default"]
endpoints = ["scp"]
middleware = ["deny_local"]

[pipelines.allow_local]
networks = ["default"]
endpoints = ["scp"]
middleware = ["allow_local"]

[pipelines.office_only]
networks = ["default"]
endpoints = ["scp"]
middleware = ["office_only"]

[endpoints.scp]
service = "dicom_scp"
[endpoints.scp.options]
local_aet = "GATE_SCP"
enable_echo = true

[middleware.deny_local]
type = "policies"
[middleware.deny_local.options]
policies = ["no_loopback"]

[middleware.allow_local]
type = "policies"
[middleware.allow_local.options]
policies = ["loopback_only"]

[middleware.office_only]
type = "policies"
[middleware.office_only.options]
policies = ["office"]

[policies.no_loopback]
id = "no_loopback"
rules = ["loopback_deny", "anyone"]

[policies.loopback_only]
id = "loopback_only"
rules = ["loopback_allow"]

[policies.office]
id = "office"
rules = ["office_allow"]

[rules.loopback_deny]
id = "loopback_deny"
type = "ip_deny"
[rules.loopback_deny.options]
ip_addresses = ["127.0.0.0/8"]

[rules.anyone]
id = "anyone"
type = "allow_all"

[rules.loopback_allow]
id = "loopback_allow"
type = "ip_allow"
[rules.loopback_allow.options]
ip_addresses = ["127.0.0.1/32"]

[rules.office_allow]
id = "office_allow"
type = "ip_allow"
[rules.office_allow.options]
ip_addresses = ["192.168.0.0/16"]

[services.dicom_scp]
module = ""
"#;
    let mut config: Config = toml::from_str(toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("references should resolve");
    config
}

/// A local port that is free now.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("local address").port()
}

async fn wait_for_port(port: u16) {
    for _ in 0..250 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing started listening on port {}", port);
}

/// Start an internal SCP whose query provider runs `pipeline`.
async fn start_scp(pipeline: &str, shutdown: &CancellationToken) -> u16 {
    let port = free_port();
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "GATE_SCP".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            enable_echo: true,
            ..Default::default()
        },
        Arc::new(PipelineQueryProvider::new(pipeline, "scp")),
    );
    tokio::spawn(scp.run(shutdown.clone()));
    wait_for_port(port).await;
    port
}

async fn echo(port: u16) -> dimse::Result<bool> {
    let scu = DimseScu::new(DimseConfig {
        local_aet: "GATE_SCU".to_string(),
        ..Default::default()
    });
    scu.echo(&RemoteNode {
        ae_title: "GATE_SCP".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    })
    .await
}

fn assert_rejected(result: dimse::Result<bool>, pipeline: &str) {
    match result {
        Err(DimseError::DicomUl(message)) => assert!(
            message.to_lowercase().contains("reject"),
            "pipeline '{}': expected an A-ASSOCIATE-RJ, got: {}",
            pipeline,
            message
        ),
        other => panic!(
            "pipeline '{}': a denied peer should not get a C-ECHO reply, got: {:?}",
            pipeline, other
        ),
    }
}

/// One test, because the pipelines read the process-wide config.
#[tokio::test]
async fn pipeline_ip_rules_decide_who_gets_a_c_echo_reply() {
    harmony::globals::set_config(Arc::new(admission_config()));
    let shutdown = CancellationToken::new();

    let denied = start_scp("deny_local", &shutdown).await;
    assert_rejected(echo(denied).await, "deny_local");

    let outside_allowlist = start_scp("office_only", &shutdown).await;
    assert_rejected(echo(outside_allowlist).await, "office_only");

    let allowed = start_scp("allow_local", &shutdown).await;
    assert!(
        echo(allowed)
            .await
            .expect("a peer on the allowlist should get a C-ECHO reply"),
        "the C-ECHO should succeed"
    );

    shutdown.cancel();
}

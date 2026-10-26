//! A DIMSE client must get a failure status when the `dicom_scu` backend
//! fails (AURA-2574).
//!
//! The pipeline has a `dicom_scp` endpoint and a `dicom_scu` backend that
//! points at a closed local port, so every C-FIND, C-GET and C-MOVE fails at
//! the backend. Before the fix a failure could reach the client as Success
//! with zero matches, which a client cannot tell apart from an empty result.
//!
//! The backend answers HTTP 500 with `"success": false`. The status mapper
//! turns HTTP 500 into 0x0110 Processing failure, which PS3.4 defines for
//! C-FIND, C-GET and C-MOVE.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use dicom_dictionary_std::{tags, uids};
use dimse::scu::{command_builder, native_connection};
use dimse::types::{FindQuery, GetQuery, QueryLevel};
use dimse::{DimseConfig, DimseError, DimseScp, DimseScu, RemoteNode};
use futures_util::StreamExt;
use harmony::adapters::dimse::query_provider::PipelineQueryProvider;
use harmony::config::config::Config;
use tokio_util::sync::CancellationToken;

const SCP_AET: &str = "FAIL_SCP";
const CLIENT_AET: &str = "FAIL_CLIENT";
const MOVE_DEST_AET: &str = "MOVE_DEST";

/// Processing failure. The status the client should get for a backend that
/// answers HTTP 500.
const PROCESSING_FAILURE: u16 = 0x0110;

/// A local port with nothing listening on it.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    port
}

fn failing_backend_config(pacs_port: u16) -> Config {
    let toml = format!(
        r#"
[proxy]
id = "dimse-backend-failure"
store_dir = "./tmp"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.http]
bind_address = "127.0.0.1"
bind_port = 18081

[pipelines.bridge]
networks = ["default"]
endpoints = ["scp"]
backends = ["pacs"]
middleware = []

[endpoints.scp]
service = "dicom_scp"
[endpoints.scp.options]
local_aet = "{SCP_AET}"
enable_echo = true
enable_find = true
enable_get = true
enable_move = true

[backends.pacs]
service = "dicom_scu"
[backends.pacs.options]
aet = "UNREACHABLE_PACS"
host = "127.0.0.1"
port = {pacs_port}
local_aet = "BRIDGE_SCU"

[services.dicom_scp]
module = ""
[services.dicom_scu]
module = ""
[services.dicom]
module = ""
"#
    );
    let mut config: Config = toml::from_str(&toml).expect("TOML parse error");
    harmony::config::resolution::resolve_references(&mut config)
        .expect("references should resolve");
    config
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

/// Start an in-process SCP whose query provider runs the `bridge` pipeline.
///
/// The move destination must be known to the SCP, or it answers 0xA801
/// before the pipeline runs. Nothing listens there, and nothing needs to,
/// because the pipeline fails before any sub-operation starts.
async fn start_scp(shutdown: &CancellationToken) -> u16 {
    let port = closed_port();
    let mut aet_table = HashMap::new();
    aet_table.insert(
        MOVE_DEST_AET.to_string(),
        remote_node(MOVE_DEST_AET, closed_port()),
    );
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: SCP_AET.to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            enable_echo: true,
            enable_find: true,
            enable_get: true,
            enable_move: true,
            aet_table,
            ..Default::default()
        },
        Arc::new(PipelineQueryProvider::new("bridge", "scp")),
    );
    tokio::spawn(scp.run(shutdown.clone()));
    wait_for_port(port).await;
    port
}

fn remote_node(ae_title: &str, port: u16) -> RemoteNode {
    RemoteNode {
        ae_title: ae_title.to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    }
}

fn client_config() -> DimseConfig {
    DimseConfig {
        local_aet: CLIENT_AET.to_string(),
        ..Default::default()
    }
}

fn study_parameters() -> HashMap<String, String> {
    HashMap::from([("StudyInstanceUID".to_string(), "1.2.3.4.5".to_string())])
}

/// Read the status code out of an SCU error such as
/// `C-FIND failed with status: 0x0110`.
fn status_in(error: &DimseError) -> Option<u16> {
    let text = error.to_string();
    let hex = text.split("0x").nth(1)?.get(..4)?;
    u16::from_str_radix(hex, 16).ok()
}

/// Drain an SCU result stream. Returns the number of datasets and the first
/// error.
async fn drain<S>(mut stream: S) -> (usize, Option<DimseError>)
where
    S: futures_util::Stream<Item = dimse::Result<dimse::DatasetStream>> + Unpin,
{
    let mut count = 0;
    while let Some(item) = stream.next().await {
        match item {
            Ok(_) => count += 1,
            Err(e) => return (count, Some(e)),
        }
    }
    (count, None)
}

async fn c_find_outcome(port: u16) -> (usize, Option<DimseError>) {
    let scu = DimseScu::new(client_config());
    let stream = scu
        .find(
            &remote_node(SCP_AET, port),
            FindQuery {
                query_level: QueryLevel::Study,
                parameters: study_parameters(),
                max_results: 0,
            },
        )
        .await
        .expect("the C-FIND request should be sent");
    drain(stream).await
}

async fn c_get_outcome(port: u16) -> (usize, Option<DimseError>) {
    let scu = DimseScu::new(client_config());
    let output_dir = std::path::PathBuf::from("./tmp/dimse_backend_failure_get");
    let stream = scu
        .get_request(
            &remote_node(SCP_AET, port),
            GetQuery {
                query_level: QueryLevel::Study,
                parameters: study_parameters(),
            },
            Some(output_dir),
        )
        .await
        .expect("the C-GET request should be sent");
    drain(stream).await
}

/// Send one C-MOVE and return the status of the final response.
///
/// `DimseScu::move_request` runs DCMTK `movescu`, which hides the status, so
/// the request is sent directly.
async fn c_move_final_status(port: u16) -> u16 {
    let sop_class = uids::STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE;
    let mut association = native_connection::establish_association(
        &client_config(),
        &remote_node(SCP_AET, port),
        &[sop_class],
    )
    .await
    .expect("association with the SCP");
    let pc_id = native_connection::get_presentation_context_id(&association, sop_class)
        .expect("presentation context for C-MOVE");

    let message_id = 1;
    let mut request = command_builder::build_command_request(
        command_builder::command_fields::C_MOVE_RQ,
        message_id,
        true,
        sop_class,
    );
    request.put(dicom_core::DataElement::new(
        tags::MOVE_DESTINATION,
        dicom_core::VR::AE,
        dicom_core::PrimitiveValue::from(MOVE_DEST_AET),
    ));
    let identifier =
        command_builder::build_identifier_dataset(QueryLevel::Study, &study_parameters())
            .expect("identifier");
    command_builder::encode_and_send_request(&mut association, request, Some(&identifier), pc_id)
        .await
        .expect("send C-MOVE-RQ");

    loop {
        let (command, _, _) = command_builder::receive_dimse_message(&mut association)
            .await
            .expect("C-MOVE-RSP");
        let status = command_builder::extract_status(&command).expect("status in C-MOVE-RSP");
        if status != 0xFF00 {
            let _ = native_connection::release_association(association).await;
            return status;
        }
    }
}

/// One test, because the pipeline reads the process-wide config.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn failed_backend_gives_the_dimse_client_a_failure_status() {
    harmony::globals::set_config(Arc::new(failing_backend_config(closed_port())));
    let shutdown = CancellationToken::new();
    let port = start_scp(&shutdown).await;

    let (matches, error) = c_find_outcome(port).await;
    let error = error.unwrap_or_else(|| {
        panic!(
            "C-FIND against a failed backend reported success with {} matches",
            matches
        )
    });
    assert_eq!(matches, 0, "C-FIND should return no matches");
    assert_eq!(
        status_in(&error),
        Some(PROCESSING_FAILURE),
        "C-FIND error: {}",
        error
    );

    let (instances, error) = c_get_outcome(port).await;
    let error = error.unwrap_or_else(|| {
        panic!(
            "C-GET against a failed backend reported success with {} instances",
            instances
        )
    });
    assert_eq!(instances, 0, "C-GET should return no instances");
    assert_eq!(
        status_in(&error),
        Some(PROCESSING_FAILURE),
        "C-GET error: {}",
        error
    );

    let status = c_move_final_status(port).await;
    assert_eq!(
        status, PROCESSING_FAILURE,
        "C-MOVE final status was 0x{:04X}",
        status
    );

    shutdown.cancel();
}

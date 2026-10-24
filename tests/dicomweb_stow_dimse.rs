//! End-to-end STOW-RS through the DICOMweb to DIMSE bridge (AURA-2418).
//!
//! Each test posts a `multipart/related` STOW-RS request to a `dicomweb`
//! endpoint. The `dicomweb_bridge` middleware and a `dicom_scu` backend send
//! each part to a local PACS, and the endpoint must answer with the PS3.18
//! Store Instances Response.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;
use dimse::types::{DatasetStream, QueryLevel};
use dimse::DimseConfig;
use harmony::config::config::Config;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const BOUNDARY: &str = "stow-boundary";

/// SOP Instance UIDs the test PACS was handed, in order.
type Received = Arc<Mutex<Vec<String>>>;

struct RecordingPacs {
    received: Received,
}

#[async_trait]
impl dimse::scp::QueryProvider for RecordingPacs {
    async fn find(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<dimse::types::BoxDatasetStream> {
        Ok(dimse::types::dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<dimse::types::BoxDatasetStream> {
        Ok(dimse::types::dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<dimse::types::RetrieveStream> {
        Ok(dimse::types::RetrieveStream::new(
            dimse::types::dataset_stream_from_vec(Vec::new()),
        ))
    }

    async fn store(
        &self,
        _peer_addr: std::net::SocketAddr,
        dataset: DatasetStream,
    ) -> dimse::Result<()> {
        let uid = dataset
            .metadata()
            .sop_instance_uid
            .clone()
            .unwrap_or_default();
        self.received.lock().unwrap().push(uid);
        Ok(())
    }
}

/// Start a PACS on a free local port that accepts every transfer syntax.
async fn start_recording_pacs() -> (u16, Received, tempfile::TempDir) {
    let received: Received = Default::default();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind a free port")
        .local_addr()
        .expect("local address")
        .port();
    let storage_dir = tempfile::tempdir().expect("PACS storage dir");
    let scp = dimse::DimseScp::new(
        DimseConfig {
            local_aet: "STOW_PACS".to_string(),
            bind_addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port,
            enable_store: true,
            storage_dir: storage_dir.path().to_path_buf(),
            ..Default::default()
        },
        Arc::new(RecordingPacs {
            received: received.clone(),
        }),
    );
    tokio::spawn(scp.run(tokio_util::sync::CancellationToken::new()));
    for _ in 0..250 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return (port, received, storage_dir);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the PACS did not start listening on port {}", port);
}

/// A Secondary Capture Part 10 file with the given SOP Instance UID.
fn part10(sop_instance_uid: &str) -> Vec<u8> {
    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
        (tags::SOP_INSTANCE_UID, sop_instance_uid),
        (
            tags::STUDY_INSTANCE_UID,
            "1.2.826.0.1.3680043.9.7133.2418.1",
        ),
        (
            tags::SERIES_INSTANCE_UID,
            "1.2.826.0.1.3680043.9.7133.2418.2",
        ),
    ] {
        object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
    }
    let mut bytes = Vec::new();
    object
        .with_meta(
            dicom_object::meta::FileMetaTableBuilder::new()
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        )
        .expect("file meta group")
        .write_all(&mut bytes)
        .expect("encode the Part 10 file");
    bytes
}

fn multipart_body(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(
            format!("--{}\r\nContent-Type: application/dicom\r\n\r\n", BOUNDARY).as_bytes(),
        );
        body.extend_from_slice(part);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
    body
}

fn bridge_config(pacs_port: u16) -> Arc<Config> {
    let toml = format!(
        r#"
        [proxy]
        id = "dicomweb-stow-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8086

        [pipelines.bridge]
        description = "DICOMweb STOW-RS -> DIMSE C-STORE"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_bridge"]
        backends = ["pacs"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.pacs]
        service = "dicom_scu"
        [backends.pacs.options]
        aet = "STOW_PACS"
        host = "127.0.0.1"
        port = {pacs_port}
        local_aet = "STOW_SCU"

        [services.dicomweb]
        module = ""
        [services.dicom]
        module = ""

        [middleware.dicomweb_bridge]
        type = "dicomweb_bridge"

        [middleware_types.dicomweb_bridge]
        module = ""
    "#
    );
    let config: Config = toml::from_str(&toml).expect("TOML parse error");
    config.validate().expect("valid config");
    Arc::new(config)
}

/// Post `parts` as one STOW-RS request and return the status, content type
/// and JSON body.
async fn post_stow(config: Arc<Config>, parts: &[Vec<u8>]) -> (StatusCode, String, Value) {
    harmony::globals::set_config(config.clone());
    let app = harmony::router::build_network_router(config, "default").await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/dicomweb/studies")
                .header(
                    "content-type",
                    format!(
                        "multipart/related; type=\"application/dicom\"; boundary={}",
                        BOUNDARY
                    ),
                )
                .body(Body::from(multipart_body(parts)))
                .unwrap(),
        )
        .await
        .expect("router handled request");

    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or_else(|e| {
        panic!(
            "body is not JSON ({}): {}",
            e,
            String::from_utf8_lossy(&body)
        )
    });
    (status, content_type, json)
}

fn sequence_items<'a>(dataset: &'a Value, tag: &str) -> &'a Vec<Value> {
    assert_eq!(dataset[tag]["vr"], "SQ", "{} in {}", tag, dataset);
    dataset[tag]["Value"]
        .as_array()
        .unwrap_or_else(|| panic!("{} has no items in {}", tag, dataset))
}

#[tokio::test]
#[serial_test::serial]
async fn stow_of_two_instances_is_200_with_both_referenced() {
    let (port, received, _storage_dir) = start_recording_pacs().await;
    let first = "1.2.826.0.1.3680043.9.7133.2418.10";
    let second = "1.2.826.0.1.3680043.9.7133.2418.11";

    let (status, content_type, body) =
        post_stow(bridge_config(port), &[part10(first), part10(second)]).await;

    assert_eq!(status, StatusCode::OK, "body: {}", body);
    assert_eq!(content_type, "application/dicom+json");
    let items = sequence_items(&body, "00081199");
    assert_eq!(items.len(), 2, "body: {}", body);
    for (item, uid) in items.iter().zip([first, second]) {
        assert_eq!(
            item["00081150"],
            serde_json::json!({"vr": "UI", "Value": [uids::SECONDARY_CAPTURE_IMAGE_STORAGE]})
        );
        assert_eq!(
            item["00081155"],
            serde_json::json!({"vr": "UI", "Value": [uid]})
        );
        assert_eq!(item["00081190"], serde_json::json!({"vr": "UR"}));
    }
    assert!(body.get("00081198").is_none(), "body: {}", body);
    assert!(body.get("0008119A").is_none(), "body: {}", body);
    assert_eq!(*received.lock().unwrap(), vec![first, second]);
}

#[tokio::test]
#[serial_test::serial]
async fn stow_with_one_part_that_is_not_part10_is_202() {
    let (port, received, _storage_dir) = start_recording_pacs().await;
    let stored = "1.2.826.0.1.3680043.9.7133.2418.20";

    let (status, content_type, body) = post_stow(
        bridge_config(port),
        &[part10(stored), b"not a DICOM Part 10 file".to_vec()],
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "body: {}", body);
    assert_eq!(content_type, "application/dicom+json");
    let items = sequence_items(&body, "00081199");
    assert_eq!(items.len(), 1, "body: {}", body);
    assert_eq!(
        items[0]["00081155"],
        serde_json::json!({"vr": "UI", "Value": [stored]})
    );
    assert_eq!(
        sequence_items(&body, "0008119A"),
        &vec![serde_json::json!({"00081197": {"vr": "US", "Value": [49152]}})]
    );
    assert!(body.get("00081198").is_none(), "body: {}", body);
    assert_eq!(*received.lock().unwrap(), vec![stored]);
}

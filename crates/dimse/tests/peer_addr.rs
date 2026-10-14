//! The SCP passes the requestor's TCP address to the query provider.
//!
//! Harmony's IP allow and deny rules and its rate limits key on this address.
//! Without it every DIMSE client looks the same.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;
use dimse::scp::QueryProvider;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, FindQuery, GetQuery, QueryLevel,
    RetrieveStream,
};
use dimse::{DimseConfig, DimseScp, DimseScu, RemoteNode};
use futures::StreamExt;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

/// Records the peer address of every call it receives.
#[derive(Default)]
struct PeerRecordingProvider {
    peers: Mutex<Vec<(&'static str, SocketAddr)>>,
}

impl PeerRecordingProvider {
    fn record(&self, operation: &'static str, peer_addr: SocketAddr) {
        self.peers.lock().unwrap().push((operation, peer_addr));
    }
}

#[async_trait::async_trait]
impl QueryProvider for PeerRecordingProvider {
    async fn find(
        &self,
        peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        self.record("find", peer_addr);
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        self.record("locate", peer_addr);
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        self.record("get", peer_addr);
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(&self, peer_addr: SocketAddr, _dataset: DatasetStream) -> dimse::Result<()> {
        self.record("store", peer_addr);
        Ok(())
    }
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

async fn start_scp() -> (u16, Arc<PeerRecordingProvider>, tempfile::TempDir) {
    let provider = Arc::new(PeerRecordingProvider::default());
    let port = free_port();
    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "PEER_SCP".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            enable_find: true,
            enable_get: true,
            enable_store: true,
            storage_dir: storage_dir.path().to_path_buf(),
            ..Default::default()
        },
        provider.clone(),
    );
    tokio::spawn(scp.run(CancellationToken::new()));
    wait_for_port(port).await;
    (port, provider, storage_dir)
}

fn node(port: u16) -> RemoteNode {
    RemoteNode {
        ae_title: "PEER_SCP".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    }
}

fn scu() -> DimseScu {
    DimseScu::new(DimseConfig {
        local_aet: "PEER_SCU".to_string(),
        ..Default::default()
    })
}

/// The SCU connects from 127.0.0.1 on an ephemeral port. The provider must
/// see that address, not the SCP's own listening address.
fn assert_loopback_client(peer: SocketAddr, scp_port: u16) {
    assert_eq!(peer.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_ne!(peer.port(), 0, "the peer port should be the client's port");
    assert_ne!(
        peer.port(),
        scp_port,
        "the peer port should be the client's port, not the SCP's"
    );
}

#[tokio::test]
async fn c_store_passes_the_client_address_to_the_provider() {
    let (port, provider, _storage_dir) = start_scp().await;

    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
        (tags::SOP_INSTANCE_UID, "1.2.826.0.1.3680043.9.7133.2417.1"),
    ] {
        object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
    }
    let mut dataset = DatasetStream::from_object(object);
    dataset.metadata_mut().transfer_syntax = Some(uids::EXPLICIT_VR_LITTLE_ENDIAN.to_string());

    let stored = scu()
        .store(&node(port), dataset)
        .await
        .expect("the SCP should accept the C-STORE");
    assert!(stored);

    let peers = provider.peers.lock().unwrap().clone();
    assert_eq!(peers.len(), 1, "the provider should see exactly one call");
    assert_eq!(peers[0].0, "store");
    assert_loopback_client(peers[0].1, port);
}

#[tokio::test]
async fn c_find_passes_the_client_address_to_the_provider() {
    let (port, provider, _storage_dir) = start_scp().await;

    let mut results = scu()
        .find(&node(port), FindQuery::patient(Some("PEER-1".to_string())))
        .await
        .expect("the SCP should accept the C-FIND");
    while let Some(result) = results.next().await {
        result.expect("the provider returns no matches, so no result should fail");
    }

    let peers = provider.peers.lock().unwrap().clone();
    assert_eq!(peers.len(), 1, "the provider should see exactly one call");
    assert_eq!(peers[0].0, "find");
    assert_loopback_client(peers[0].1, port);
}

#[tokio::test]
async fn c_get_passes_the_client_address_to_the_provider() {
    let (port, provider, _storage_dir) = start_scp().await;

    let mut results = scu()
        .get_request(
            &node(port),
            GetQuery::new(QueryLevel::Study)
                .with_parameter("StudyInstanceUID", "1.2.826.0.1.3680043.9.7133.2417.2"),
            None,
        )
        .await
        .expect("the SCP should accept the C-GET");
    while let Some(result) = results.next().await {
        result.expect("the provider returns no instances, so no result should fail");
    }

    let peers = provider.peers.lock().unwrap().clone();
    assert_eq!(peers.len(), 1, "the provider should see exactly one call");
    assert_eq!(peers[0].0, "get");
    assert_loopback_client(peers[0].1, port);
}

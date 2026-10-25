//! The SCP asks the query provider whether a peer may open an association.
//!
//! A rejected peer gets an A-ASSOCIATE-RJ. It cannot send any command, so it
//! gets no C-ECHO reply and never reaches the provider's query methods.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use dimse::scp::{PeerAdmission, QueryProvider};
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, FindQuery, QueryLevel,
    RetrieveStream,
};
use dimse::{DimseConfig, DimseError, DimseScp, DimseScu, RemoteNode};
use futures::StreamExt;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

/// How the test provider answers `admit_peer`.
#[derive(Clone, Copy)]
enum Verdict {
    Admit,
    Reject,
    Fail,
}

/// Answers `admit_peer` with a fixed verdict and records every call.
struct GatekeeperProvider {
    verdict: Verdict,
    admitted_calls: Mutex<Vec<SocketAddr>>,
    query_calls: Mutex<Vec<&'static str>>,
}

impl GatekeeperProvider {
    fn new(verdict: Verdict) -> Self {
        Self {
            verdict,
            admitted_calls: Mutex::new(Vec::new()),
            query_calls: Mutex::new(Vec::new()),
        }
    }

    fn record_query(&self, operation: &'static str) {
        self.query_calls.lock().unwrap().push(operation);
    }
}

#[async_trait::async_trait]
impl QueryProvider for GatekeeperProvider {
    async fn find(
        &self,
        _peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        self.record_query("find");
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        self.record_query("locate");
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _peer_addr: SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        self.record_query("get");
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(&self, _peer_addr: SocketAddr, _dataset: DatasetStream) -> dimse::Result<()> {
        self.record_query("store");
        Ok(())
    }

    async fn admit_peer(&self, peer_addr: SocketAddr) -> dimse::Result<PeerAdmission> {
        self.admitted_calls.lock().unwrap().push(peer_addr);
        match self.verdict {
            Verdict::Admit => Ok(PeerAdmission::Admit),
            Verdict::Reject => Ok(PeerAdmission::Reject {
                reason: "test policy denies this peer".to_string(),
            }),
            Verdict::Fail => Err(DimseError::operation_failed("policy lookup failed")),
        }
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

async fn start_scp(verdict: Verdict) -> (u16, Arc<GatekeeperProvider>, tempfile::TempDir) {
    let provider = Arc::new(GatekeeperProvider::new(verdict));
    let port = free_port();
    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "GATE_SCP".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            enable_echo: true,
            enable_find: true,
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
        ae_title: "GATE_SCP".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    }
}

fn scu() -> DimseScu {
    DimseScu::new(DimseConfig {
        local_aet: "GATE_SCU".to_string(),
        ..Default::default()
    })
}

/// The SCU must see an association rejection, not a dropped connection.
fn assert_association_rejected(result: dimse::Result<impl std::fmt::Debug>) {
    let err = result.expect_err("the SCP should refuse the association");
    match &err {
        DimseError::DicomUl(message) => assert!(
            message.to_lowercase().contains("reject"),
            "expected an A-ASSOCIATE-RJ, got: {}",
            message
        ),
        other => panic!("expected an association error, got: {:?}", other),
    }
}

#[tokio::test]
async fn rejected_peer_gets_no_c_echo_reply() {
    let (port, provider, _storage_dir) = start_scp(Verdict::Reject).await;

    assert_association_rejected(scu().echo(&node(port)).await);

    let admitted = provider.admitted_calls.lock().unwrap().clone();
    assert!(!admitted.is_empty(), "the SCP should have asked the provider");
    for peer in admitted {
        assert_eq!(peer.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_ne!(peer.port(), port, "the provider should see the client's port");
    }
}

#[tokio::test]
async fn rejected_peer_never_reaches_the_query_methods() {
    let (port, provider, _storage_dir) = start_scp(Verdict::Reject).await;

    // The SCU's C-FIND opens the association in a background task. The
    // rejection arrives as the only item on the result stream.
    let mut results = scu()
        .find(&node(port), FindQuery::patient(Some("GATE-1".to_string())))
        .await
        .expect("the SCU starts the C-FIND before it connects");
    let first = results
        .next()
        .await
        .expect("the rejection should arrive on the C-FIND stream");
    assert_association_rejected(first);
    assert!(
        results.next().await.is_none(),
        "a rejected peer should get no C-FIND results"
    );

    assert!(
        provider.query_calls.lock().unwrap().is_empty(),
        "a rejected peer must not reach find, get, locate or store"
    );
}

#[tokio::test]
async fn failed_admission_check_rejects_the_peer() {
    let (port, _provider, _storage_dir) = start_scp(Verdict::Fail).await;

    assert_association_rejected(scu().echo(&node(port)).await);
}

#[tokio::test]
async fn admitted_peer_gets_a_c_echo_reply_and_can_query() {
    let (port, provider, _storage_dir) = start_scp(Verdict::Admit).await;

    let echoed = scu()
        .echo(&node(port))
        .await
        .expect("an admitted peer should get a C-ECHO reply");
    assert!(echoed);

    let mut results = scu()
        .find(&node(port), FindQuery::patient(Some("GATE-2".to_string())))
        .await
        .expect("an admitted peer should be able to run C-FIND");
    while let Some(result) = results.next().await {
        result.expect("the provider returns no matches, so no result should fail");
    }
    assert_eq!(provider.query_calls.lock().unwrap().as_slice(), ["find"]);
}

//! The C-GET path streams end to end: the SCP sends each dataset as the
//! provider produces it, and the native SCU hands each instance to its caller
//! as soon as it has been received.
//!
//! The provider yields one dataset immediately and holds the second one back
//! until the test opens a gate. The SCU must deliver the first instance while
//! the gate is still closed. A collecting SCP or SCU would deliver nothing
//! until the gate opens.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;
use dimse::scp::QueryProvider;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, GetQuery, QueryLevel, RetrieveStream,
};
use dimse::{DimseConfig, DimseScp, DimseScu, RemoteNode};
use futures::StreamExt;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

const FIRST_INSTANCE_UID: &str = "1.2.826.0.1.3680043.9.7133.1";
const SECOND_INSTANCE_UID: &str = "1.2.826.0.1.3680043.9.7133.2";

fn instance(sop_instance_uid: &str) -> DatasetStream {
    let mut object = InMemDicomObject::new_empty();
    object.put(DataElement::new(
        tags::SOP_CLASS_UID,
        VR::UI,
        PrimitiveValue::from(uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
    ));
    object.put(DataElement::new(
        tags::SOP_INSTANCE_UID,
        VR::UI,
        PrimitiveValue::from(sop_instance_uid),
    ));
    object.put(DataElement::new(
        tags::PATIENT_NAME,
        VR::PN,
        PrimitiveValue::from("Streaming^Test"),
    ));
    DatasetStream::from_object(object)
}

/// Yields the first instance at once and the second only after the gate opens.
struct GatedProvider {
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait::async_trait]
impl QueryProvider for GatedProvider {
    async fn find(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _peer_addr: std::net::SocketAddr,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        let gate = self
            .gate
            .lock()
            .expect("gate mutex")
            .take()
            .expect("get is called once");
        let datasets = futures::stream::iter(vec![Ok(instance(FIRST_INSTANCE_UID))]).chain(
            futures::stream::once(async move {
                gate.await.expect("gate opened");
                Ok(instance(SECOND_INSTANCE_UID))
            }),
        );
        Ok(RetrieveStream::new(Box::pin(datasets)))
    }

    async fn store(
        &self,
        _peer_addr: std::net::SocketAddr,
        _dataset: DatasetStream,
    ) -> dimse::Result<()> {
        Ok(())
    }
}

async fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn c_get_delivers_the_first_instance_before_the_second_exists() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp_config = DimseConfig {
        local_aet: "GATED_SCP".to_string(),
        bind_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        port,
        enable_get: true,
        storage_dir: storage_dir.path().to_path_buf(),
        ..Default::default()
    };

    let (open_gate, gate) = oneshot::channel();
    let provider = Arc::new(GatedProvider {
        gate: Mutex::new(Some(gate)),
    });
    let scp = DimseScp::new(scp_config, provider);

    let shutdown = CancellationToken::new();
    let scp_task = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { scp.run(shutdown).await }
    });
    assert!(wait_for_port(port, 5000).await, "SCP should be listening");

    let scu = DimseScu::new(DimseConfig {
        local_aet: "GATED_SCU".to_string(),
        ..Default::default()
    });
    let node = RemoteNode::new("GATED_SCP", "127.0.0.1", port);
    let query = GetQuery::new(QueryLevel::Study).with_parameter("StudyInstanceUID", "1.2.3");

    let mut instances = scu
        .get_request(&node, query, None)
        .await
        .expect("C-GET should start");

    // The first instance has to arrive while the provider is still blocked on
    // the gate. Five seconds is far longer than one C-STORE takes locally.
    let first = timeout(Duration::from_secs(5), instances.next())
        .await
        .expect("first instance must arrive before the gate opens")
        .expect("stream must not end before the second instance")
        .expect("first instance must be an Ok item");
    assert_eq!(
        first.metadata().sop_instance_uid.as_deref(),
        Some(FIRST_INSTANCE_UID)
    );

    open_gate.send(()).expect("open the gate");

    let second = timeout(Duration::from_secs(5), instances.next())
        .await
        .expect("second instance must arrive after the gate opens")
        .expect("stream must carry the second instance")
        .expect("second instance must be an Ok item");
    assert_eq!(
        second.metadata().sop_instance_uid.as_deref(),
        Some(SECOND_INSTANCE_UID)
    );

    let end = timeout(Duration::from_secs(5), instances.next())
        .await
        .expect("stream must end after the final C-GET response");
    assert!(end.is_none(), "no further items expected, got {:?}", end);

    shutdown.cancel();
    let _ = timeout(Duration::from_secs(5), scp_task).await;
}

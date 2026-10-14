//! A C-STORE whose data set spans several P-DATA PDUs arrives whole (AURA-2496).
//!
//! The SCP must wait for the data fragment that carries the last-fragment
//! flag before it stores an instance and acknowledges it.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use dicom_core::value::{PixelFragmentSequence, Value};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;
use dimse::scp::QueryProvider;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, QueryLevel, RetrieveStream,
};
use dimse::{DimseConfig, DimseScp, DimseScu, RemoteNode};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

const JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";
const SOP_INSTANCE_UID: &str = "1.2.826.0.1.3680043.9.7133.2496.1";

/// The SCP offers this PDU length, so the SCU splits a data set at it.
const MAX_PDU: u32 = 16384;

/// Far above the size of any instance these tests send.
const ROOMY_LIMIT: u64 = 64 * 1024 * 1024;

/// Bytes that differ along their whole length, so a fragment that is lost,
/// repeated or out of order changes the result.
fn patterned_bytes(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

fn instance(pixel_data: DataElement<InMemDicomObject>) -> InMemDicomObject {
    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
        (tags::SOP_INSTANCE_UID, SOP_INSTANCE_UID),
    ] {
        object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
    }
    object.put(pixel_data);
    object
}

fn dataset(object: InMemDicomObject, transfer_syntax: &str) -> DatasetStream {
    let mut dataset = DatasetStream::from_object(object);
    dataset.metadata_mut().transfer_syntax = Some(transfer_syntax.to_string());
    dataset
}

struct RecordingProvider {
    received: Arc<Mutex<Vec<InMemDicomObject>>>,
}

#[async_trait::async_trait]
impl QueryProvider for RecordingProvider {
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
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(
        &self,
        _peer_addr: std::net::SocketAddr,
        dataset: DatasetStream,
    ) -> dimse::Result<()> {
        let object = dataset.to_object().await?;
        self.received.lock().unwrap().push(object);
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

async fn start_recording_scp(
    max_dataset_bytes: u64,
) -> (u16, Arc<Mutex<Vec<InMemDicomObject>>>, tempfile::TempDir) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let port = free_port();
    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "RECORDING".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            max_pdu: MAX_PDU,
            max_dataset_bytes,
            enable_store: true,
            storage_dir: storage_dir.path().to_path_buf(),
            ..Default::default()
        },
        Arc::new(RecordingProvider {
            received: received.clone(),
        }),
    );
    tokio::spawn(scp.run(CancellationToken::new()));
    wait_for_port(port).await;
    (port, received, storage_dir)
}

async fn store(port: u16, dataset: DatasetStream) -> dimse::Result<bool> {
    let scu = DimseScu::new(DimseConfig {
        local_aet: "STORE_SCU".to_string(),
        ..Default::default()
    });
    let node = RemoteNode {
        ae_title: "RECORDING".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    };
    scu.store(&node, dataset).await
}

#[tokio::test]
async fn a_native_instance_larger_than_one_pdu_arrives_whole() {
    let (port, received, _storage_dir) = start_recording_scp(ROOMY_LIMIT).await;
    let pixels = patterned_bytes(10 * MAX_PDU as usize);

    let stored = store(
        port,
        dataset(
            instance(DataElement::new(
                tags::PIXEL_DATA,
                VR::OB,
                PrimitiveValue::from(pixels.clone()),
            )),
            uids::EXPLICIT_VR_LITTLE_ENDIAN,
        ),
    )
    .await
    .expect("the SCP should accept a data set that spans several PDUs");
    assert!(stored);

    let received = received.lock().unwrap();
    assert_eq!(
        received.len(),
        1,
        "the SCP should hold exactly one instance"
    );
    let received_pixels = received[0]
        .element(tags::PIXEL_DATA)
        .expect("PixelData missing")
        .to_bytes()
        .expect("PixelData bytes");
    assert_eq!(received_pixels.len(), pixels.len());
    assert!(
        received_pixels.as_ref() == pixels.as_slice(),
        "the pixel data changed on the way"
    );
}

#[tokio::test]
async fn a_compressed_instance_larger_than_one_pdu_arrives_whole() {
    let (port, received, _storage_dir) = start_recording_scp(ROOMY_LIMIT).await;
    // Nothing on the path decodes the fragments. They only have to arrive
    // unchanged. Each has an even length, as DICOM requires of a fragment.
    let fragments = vec![
        patterned_bytes(6 * MAX_PDU as usize),
        patterned_bytes(3 * MAX_PDU as usize + 2),
    ];

    let stored = store(
        port,
        dataset(
            instance(DataElement::new(
                tags::PIXEL_DATA,
                VR::OB,
                Value::PixelSequence(PixelFragmentSequence::new_fragments(fragments.clone())),
            )),
            JPEG_LOSSLESS_SV1,
        ),
    )
    .await
    .expect("the SCP should accept a data set that spans several PDUs");
    assert!(stored);

    let received = received.lock().unwrap();
    assert_eq!(
        received.len(),
        1,
        "the SCP should hold exactly one instance"
    );
    let received_fragments = match received[0]
        .element(tags::PIXEL_DATA)
        .expect("PixelData missing")
        .value()
    {
        Value::PixelSequence(sequence) => sequence.fragments().to_vec(),
        other => panic!("PixelData is not encapsulated: {:?}", other),
    };
    let lengths = |fragments: &[Vec<u8>]| fragments.iter().map(Vec::len).collect::<Vec<_>>();
    assert_eq!(lengths(&received_fragments), lengths(&fragments));
    assert!(
        received_fragments == fragments,
        "the pixel fragments changed on the way"
    );
}

fn compressed_dataset(pixel_bytes: usize) -> DatasetStream {
    dataset(
        instance(DataElement::new(
            tags::PIXEL_DATA,
            VR::OB,
            Value::PixelSequence(PixelFragmentSequence::new_fragments(vec![patterned_bytes(
                pixel_bytes,
            )])),
        )),
        JPEG_LOSSLESS_SV1,
    )
}

/// The instances are compressed because a truncated compressed data set still
/// parses. Only the size limit can refuse the second one.
#[tokio::test]
async fn a_compressed_instance_over_the_size_limit_is_refused() {
    let limit = 4 * MAX_PDU as u64;
    let (port, received, _storage_dir) = start_recording_scp(limit).await;

    let stored = store(port, compressed_dataset(2 * MAX_PDU as usize))
        .await
        .expect("an instance under the limit is accepted");
    assert!(stored);
    assert_eq!(received.lock().unwrap().len(), 1);

    let result = store(port, compressed_dataset(10 * MAX_PDU as usize)).await;
    assert!(
        !matches!(result, Ok(true)),
        "an instance over the limit must not be acknowledged as stored"
    );
    assert_eq!(
        received.lock().unwrap().len(),
        1,
        "nothing over the limit should reach the provider"
    );
}

#[tokio::test]
async fn a_data_set_for_a_disabled_operation_is_refused() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let port = free_port();
    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "RECORDING".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            max_pdu: MAX_PDU,
            enable_store: false,
            storage_dir: storage_dir.path().to_path_buf(),
            ..Default::default()
        },
        Arc::new(RecordingProvider {
            received: received.clone(),
        }),
    );
    tokio::spawn(scp.run(CancellationToken::new()));
    wait_for_port(port).await;

    let result = store(port, compressed_dataset(10 * MAX_PDU as usize)).await;
    assert!(
        !matches!(result, Ok(true)),
        "a store must not be acknowledged when C-STORE is disabled"
    );
    assert!(received.lock().unwrap().is_empty());
}

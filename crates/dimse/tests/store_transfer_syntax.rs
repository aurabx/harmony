//! A C-STORE keeps the transfer syntax of the instance (AURA-2470).
//!
//! The SCP records the transfer syntax an instance arrived in. The SCU
//! proposes the transfer syntax of the instance it sends, and never sends
//! encapsulated pixel data in any other transfer syntax.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use dicom_core::value::{PixelFragmentSequence, Value};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::meta::FileMetaTableBuilder;
use dicom_object::InMemDicomObject;
use dicom_ul::association::server::ServerAssociationOptions;
use dicom_ul::pdu::PresentationContextResultReason;
use dicom_ul::Pdu;
use dimse::scp::QueryProvider;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, QueryLevel, RetrieveStream,
};
use dimse::{DimseConfig, DimseScp, DimseScu, RemoteNode};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

const JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";
const SOP_INSTANCE_UID: &str = "1.2.826.0.1.3680043.9.7133.2470.1";

/// The fragments are not a real JPEG stream. Nothing on the path decodes
/// them, and they only have to arrive unchanged. Each has an even length, as
/// DICOM requires of a fragment.
fn fragments() -> Vec<Vec<u8>> {
    vec![
        vec![0xFF, 0xD8, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xD9],
        vec![0xFF, 0xD8, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0xFF, 0xD9],
    ]
}

fn instance_without_pixel_data() -> InMemDicomObject {
    let mut object = InMemDicomObject::new_empty();
    for (tag, value) in [
        (tags::SOP_CLASS_UID, uids::SECONDARY_CAPTURE_IMAGE_STORAGE),
        (tags::SOP_INSTANCE_UID, SOP_INSTANCE_UID),
    ] {
        object.put(DataElement::new(tag, VR::UI, PrimitiveValue::from(value)));
    }
    object.put(DataElement::new(
        tags::PATIENT_NAME,
        VR::PN,
        PrimitiveValue::from("Compressed^Store"),
    ));
    object
}

fn instance_with_encapsulated_pixel_data() -> InMemDicomObject {
    let mut object = instance_without_pixel_data();
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        Value::PixelSequence(PixelFragmentSequence::new_fragments(fragments())),
    ));
    object
}

fn instance_with_native_pixel_data() -> InMemDicomObject {
    let mut object = instance_without_pixel_data();
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::from(vec![1u8, 2, 3, 4]),
    ));
    object
}

fn dataset(object: InMemDicomObject, transfer_syntax: Option<&str>) -> DatasetStream {
    let mut dataset = DatasetStream::from_object(object);
    dataset.metadata_mut().transfer_syntax = transfer_syntax.map(str::to_string);
    dataset
}

/// What the SCP handed to its provider for one C-STORE.
struct Received {
    transfer_syntax: Option<String>,
    object: InMemDicomObject,
}

struct RecordingProvider {
    received: Arc<Mutex<Vec<Received>>>,
}

#[async_trait::async_trait]
impl QueryProvider for RecordingProvider {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
        _max_results: u32,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<BoxDatasetStream> {
        Ok(dataset_stream_from_vec(Vec::new()))
    }

    async fn get(
        &self,
        _query_level: QueryLevel,
        _parameters: &HashMap<String, String>,
    ) -> dimse::Result<RetrieveStream> {
        Ok(RetrieveStream::new(dataset_stream_from_vec(Vec::new())))
    }

    async fn store(&self, dataset: DatasetStream) -> dimse::Result<()> {
        let received = Received {
            transfer_syntax: dataset.metadata().transfer_syntax.clone(),
            object: dataset.to_object().await?,
        };
        self.received.lock().unwrap().push(received);
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
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing started listening on port {}", port);
}

/// Start the Harmony SCP, which accepts every presentation context, and
/// return its port and what it receives.
async fn start_recording_scp() -> (u16, Arc<Mutex<Vec<Received>>>, tempfile::TempDir) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let port = free_port();
    let storage_dir = tempfile::tempdir().expect("storage dir");
    let scp = DimseScp::new(
        DimseConfig {
            local_aet: "RECORDING".to_string(),
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
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

fn scu() -> DimseScu {
    DimseScu::new(DimseConfig {
        local_aet: "STORE_SCU".to_string(),
        ..Default::default()
    })
}

fn node(ae_title: &str, port: u16) -> RemoteNode {
    RemoteNode {
        ae_title: ae_title.to_string(),
        host: "127.0.0.1".to_string(),
        port,
        use_tls: false,
        connect_timeout_ms: None,
        max_pdu: None,
    }
}

fn pixel_fragments(object: &InMemDicomObject) -> Vec<Vec<u8>> {
    match object
        .element(tags::PIXEL_DATA)
        .expect("PixelData missing")
        .value()
    {
        Value::PixelSequence(sequence) => sequence.fragments().to_vec(),
        other => panic!("PixelData is not encapsulated: {:?}", other),
    }
}

#[tokio::test]
async fn a_compressed_instance_arrives_in_its_own_transfer_syntax() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    let stored = scu()
        .store(
            &node("RECORDING", port),
            dataset(
                instance_with_encapsulated_pixel_data(),
                Some(JPEG_LOSSLESS_SV1),
            ),
        )
        .await
        .expect("the SCP accepts every transfer syntax");
    assert!(stored);

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(JPEG_LOSSLESS_SV1),
        "the SCP should record the negotiated transfer syntax"
    );
    assert_eq!(
        pixel_fragments(&received[0].object),
        fragments(),
        "the pixel fragments changed on the way"
    );
}

#[tokio::test]
async fn the_scp_records_the_transfer_syntax_of_an_uncompressed_instance() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    scu()
        .store(
            &node("RECORDING", port),
            dataset(
                instance_with_native_pixel_data(),
                Some(uids::EXPLICIT_VR_LITTLE_ENDIAN),
            ),
        )
        .await
        .expect("the SCP accepts every transfer syntax");

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    // The SCU proposes the instance's transfer syntax first, and the SCP
    // takes the first one it supports.
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(uids::EXPLICIT_VR_LITTLE_ENDIAN)
    );
}

#[tokio::test]
async fn a_deflated_instance_is_sent_as_explicit_vr() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    // The label is enough. The object is already decoded, so nothing here
    // inflates or deflates.
    scu()
        .store(
            &node("RECORDING", port),
            dataset(
                instance_with_native_pixel_data(),
                Some(uids::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN),
            ),
        )
        .await
        .expect("the SCP accepts every transfer syntax");

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(uids::EXPLICIT_VR_LITTLE_ENDIAN),
        "a deflated data set must not be put in P-DATA fragments"
    );
}

#[tokio::test]
async fn an_instance_without_a_recorded_transfer_syntax_is_sent_as_implicit_vr() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    scu()
        .store(
            &node("RECORDING", port),
            dataset(instance_with_native_pixel_data(), None),
        )
        .await
        .expect("the SCP accepts every transfer syntax");

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(uids::IMPLICIT_VR_LITTLE_ENDIAN)
    );
}

#[tokio::test]
async fn encapsulated_pixel_data_without_a_recorded_transfer_syntax_is_not_sent() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    let error = scu()
        .store(
            &node("RECORDING", port),
            dataset(instance_with_encapsulated_pixel_data(), None),
        )
        .await
        .expect_err("the transfer syntax of the pixel data is not known");

    let message = error.to_string();
    assert!(
        message.contains("no recorded transfer syntax") && message.contains(SOP_INSTANCE_UID),
        "the error should say why the instance was refused: {}",
        message
    );
    assert!(
        received.lock().unwrap().is_empty(),
        "nothing should reach the SCP"
    );
}

#[tokio::test]
async fn a_compressed_part10_file_arrives_in_its_own_transfer_syntax() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("compressed.dcm");
    instance_with_encapsulated_pixel_data()
        .with_meta(FileMetaTableBuilder::new().transfer_syntax(JPEG_LOSSLESS_SV1))
        .expect("file meta group")
        .write_to_file(&path)
        .expect("write the Part 10 file");

    // The metadata records no transfer syntax. The SCU must read it from the
    // file meta group.
    let stored = scu()
        .store(
            &node("RECORDING", port),
            DatasetStream::from_file(path, false),
        )
        .await
        .expect("the SCP accepts every transfer syntax");
    assert!(stored);

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(JPEG_LOSSLESS_SV1)
    );
    assert_eq!(pixel_fragments(&received[0].object), fragments());
}

#[tokio::test]
async fn encapsulated_pixel_data_labelled_as_uncompressed_is_not_sent() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    let error = scu()
        .store(
            &node("RECORDING", port),
            dataset(
                instance_with_encapsulated_pixel_data(),
                Some(uids::EXPLICIT_VR_LITTLE_ENDIAN),
            ),
        )
        .await
        .expect_err("Explicit VR Little Endian cannot carry compressed fragments");

    let message = error.to_string();
    assert!(
        message.contains("has encapsulated pixel data")
            && message.contains("1.2.840.10008.1.2.1 takes native pixel data"),
        "the error should name the negotiated syntax and the form of the pixel data: {}",
        message
    );
    assert!(
        received.lock().unwrap().is_empty(),
        "nothing should reach the SCP"
    );
}

#[tokio::test]
async fn native_pixel_data_labelled_as_compressed_is_not_sent() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    let error = scu()
        .store(
            &node("RECORDING", port),
            dataset(instance_with_native_pixel_data(), Some(JPEG_LOSSLESS_SV1)),
        )
        .await
        .expect_err("JPEG Lossless cannot carry native pixel values");

    let message = error.to_string();
    assert!(
        message.contains("has native pixel data")
            && message.contains("1.2.840.10008.1.2.4.70 takes encapsulated pixel data"),
        "the error should name the negotiated syntax and the form of the pixel data: {}",
        message
    );
    assert!(
        received.lock().unwrap().is_empty(),
        "nothing should reach the SCP"
    );
}

#[tokio::test]
async fn an_instance_without_pixel_data_is_sent_in_a_compressed_syntax() {
    let (port, received, _storage_dir) = start_recording_scp().await;

    // With no Pixel Data element there is nothing the transfer syntax could
    // mismatch, so the instance goes out as labelled.
    scu()
        .store(
            &node("RECORDING", port),
            dataset(instance_without_pixel_data(), Some(JPEG_LOSSLESS_SV1)),
        )
        .await
        .expect("an instance without pixel data is valid in any transfer syntax");

    let received = received.lock().unwrap();
    assert_eq!(received.len(), 1, "the SCP should hold exactly one instance");
    assert_eq!(
        received[0].transfer_syntax.as_deref(),
        Some(JPEG_LOSSLESS_SV1)
    );
}

/// What the uncompressed-only PACS saw on its one association.
struct Served {
    /// The transfer syntaxes of the presentation contexts it accepted.
    accepted_transfer_syntaxes: Vec<String>,
    /// The number of P-DATA PDUs it was sent.
    p_data_pdus: usize,
}

/// A PACS that takes Secondary Capture instances in the two uncompressed
/// little endian transfer syntaxes and nothing else. It serves one
/// association and answers no request: it closes the connection at the first
/// P-DATA PDU.
async fn start_uncompressed_only_pacs() -> (u16, tokio::task::JoinHandle<Served>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind PACS");
    let port = listener.local_addr().expect("local address").port();

    let served = tokio::spawn(async move {
        let p_data_pdus = AtomicUsize::new(0);
        let mut accepted_transfer_syntaxes = Vec::new();
        let (stream, _) = listener.accept().await.expect("accept the SCU");
        let options = ServerAssociationOptions::new()
            .accept_any()
            .ae_title("PLAIN_PACS")
            .with_abstract_syntax(uids::SECONDARY_CAPTURE_IMAGE_STORAGE)
            .with_transfer_syntax(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .with_transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN);
        // The association ends when the SCU aborts or the connection closes.
        // Either can surface as an error from `establish_async` or `receive`.
        if let Ok(mut association) = options.establish_async(stream).await {
            accepted_transfer_syntaxes = association
                .presentation_contexts()
                .iter()
                .filter(|pc| pc.reason == PresentationContextResultReason::Acceptance)
                .map(|pc| pc.transfer_syntax.clone())
                .collect();
            loop {
                match association.receive().await {
                    Ok(Pdu::PData { .. }) => {
                        p_data_pdus.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                    Ok(Pdu::ReleaseRQ) => {
                        association
                            .send(&Pdu::ReleaseRP)
                            .await
                            .expect("answer the release");
                        break;
                    }
                    Ok(_) | Err(_) => break,
                }
            }
        }
        Served {
            accepted_transfer_syntaxes,
            p_data_pdus: p_data_pdus.load(Ordering::Relaxed),
        }
    });

    (port, served)
}

#[tokio::test]
async fn a_pacs_without_the_compressed_syntax_gets_nothing_and_the_store_fails() {
    let (port, served) = start_uncompressed_only_pacs().await;

    let error = scu()
        .store(
            &node("PLAIN_PACS", port),
            dataset(
                instance_with_encapsulated_pixel_data(),
                Some(JPEG_LOSSLESS_SV1),
            ),
        )
        .await
        .expect_err("the PACS does not accept JPEG Lossless");

    let message = error.to_string();
    assert!(
        message.contains("accepted no presentation context"),
        "the error should say the PACS refused the context: {}",
        message
    );
    for expected in [uids::SECONDARY_CAPTURE_IMAGE_STORAGE, JPEG_LOSSLESS_SV1] {
        assert!(
            message.contains(expected),
            "the error should name {}: {}",
            expected,
            message
        );
    }
    assert!(
        message.ends_with(&format!("[{}]", JPEG_LOSSLESS_SV1)),
        "only the instance's transfer syntax should be proposed for compressed pixel data: {}",
        message
    );

    let served = timeout(Duration::from_secs(5), served)
        .await
        .expect("the PACS should see the association end")
        .expect("PACS task");
    assert_eq!(
        served.p_data_pdus, 0,
        "no command or data set should reach the PACS"
    );
}

#[tokio::test]
async fn a_pacs_without_the_compressed_syntax_still_takes_an_uncompressed_instance() {
    let (port, served) = start_uncompressed_only_pacs().await;

    // The PACS in this test answers no C-STORE, so the store cannot succeed.
    // The request reaching it shows that a context was negotiated.
    let error = timeout(
        Duration::from_secs(5),
        scu().store(
            &node("PLAIN_PACS", port),
            dataset(
                instance_with_native_pixel_data(),
                Some(uids::EXPLICIT_VR_LITTLE_ENDIAN),
            ),
        ),
    )
    .await
    .expect("the store should end when the PACS closes the connection")
    .expect_err("the PACS never answers the C-STORE");
    assert!(
        !error.to_string().contains("accepted no presentation context"),
        "the PACS should accept an uncompressed transfer syntax: {}",
        error
    );

    let served = timeout(Duration::from_secs(5), served)
        .await
        .expect("the PACS should see the association end")
        .expect("PACS task");
    assert_eq!(
        served.accepted_transfer_syntaxes,
        vec![uids::EXPLICIT_VR_LITTLE_ENDIAN],
        "the PACS should take the instance's own transfer syntax, proposed first"
    );
    assert_eq!(
        served.p_data_pdus, 1,
        "the C-STORE request should reach the PACS"
    );
}

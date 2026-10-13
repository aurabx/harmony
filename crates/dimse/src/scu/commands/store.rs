//! C-STORE command handler

use std::sync::atomic::{AtomicU16, Ordering};

use dicom_core::value::Value;
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::client::AsyncClientAssociation;
use dicom_ul::{Pdu};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::scu::command_builder;
use crate::scu::native_connection;
use crate::types::DatasetStream;
use crate::{DimseError, Result};

// Message ID counter (thread-safe)
static MESSAGE_ID_COUNTER: AtomicU16 = AtomicU16::new(1);

fn next_message_id() -> u16 {
    MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// An instance to send, with what the C-STORE needs to know about it.
struct StoreInstance {
    object: InMemDicomObject<StandardDataDictionary>,
    sop_class_uid: String,
    sop_instance_uid: String,
    /// The transfer syntax the instance is encoded in, when it is recorded.
    transfer_syntax: Option<String>,
}

/// A UID without the padding DICOM allows at its end.
fn trim_uid(uid: &str) -> &str {
    uid.trim_end_matches(|c: char| c.is_whitespace() || c == '\0')
}

/// Choose the transfer syntaxes to propose for the C-STORE of an instance
/// encoded in `instance_transfer_syntax`, in order of preference.
///
/// - Encapsulated transfer syntax: only that one. The pixel data is not
///   transcoded, so no other transfer syntax can carry it.
/// - Any other transfer syntax: that one, then Implicit VR Little Endian,
///   which every node must accept. The data set is re-encoded as negotiated.
/// - None recorded: Implicit VR Little Endian.
///
/// Fails when the transfer syntax is not in the registry.
pub(crate) fn transfer_syntaxes_to_propose(
    instance_transfer_syntax: Option<&str>,
) -> Result<Vec<String>> {
    let Some(instance_ts) = instance_transfer_syntax.map(trim_uid) else {
        return Ok(vec![uids::IMPLICIT_VR_LITTLE_ENDIAN.to_string()]);
    };

    let ts = TransferSyntaxRegistry.get(instance_ts).ok_or_else(|| {
        DimseError::operation_failed(format!(
            "The instance is in transfer syntax {}, which is not known, so it cannot be proposed for C-STORE",
            instance_ts
        ))
    })?;

    let mut proposal = vec![instance_ts.to_string()];
    if !ts.is_encapsulated_pixel_data() && instance_ts != uids::IMPLICIT_VR_LITTLE_ENDIAN {
        proposal.push(uids::IMPLICIT_VR_LITTLE_ENDIAN.to_string());
    }
    Ok(proposal)
}

/// Whether the Pixel Data of `object` is encapsulated: a sequence of
/// compressed fragments instead of native pixel values.
pub(crate) fn has_encapsulated_pixel_data(object: &InMemDicomObject<StandardDataDictionary>) -> bool {
    object
        .element(tags::PIXEL_DATA)
        .is_ok_and(|element| matches!(element.value(), Value::PixelSequence(_)))
}

/// Refuse an instance whose pixel data would not match the transfer syntax
/// it is sent in.
///
/// Encapsulated pixel data is sent as it is, so it is only valid in the
/// transfer syntax of the instance. The check fails when the negotiated
/// transfer syntax is a different one, or when the instance has no recorded
/// transfer syntax. Native pixel data is re-encoded and always passes.
pub(crate) fn check_pixel_data_matches_negotiated(
    sop_instance_uid: &str,
    encapsulated_pixel_data: bool,
    instance_transfer_syntax: Option<&str>,
    negotiated_transfer_syntax: &str,
) -> Result<()> {
    if !encapsulated_pixel_data {
        return Ok(());
    }
    let negotiated = trim_uid(negotiated_transfer_syntax);
    match instance_transfer_syntax.map(trim_uid) {
        Some(instance_ts) if instance_ts == negotiated => Ok(()),
        Some(instance_ts) => Err(DimseError::operation_failed(format!(
            "Instance {} has encapsulated pixel data in transfer syntax {}, but the C-STORE was negotiated in {}.              The pixel data is not transcoded, so the instance was not sent",
            sop_instance_uid, instance_ts, negotiated
        ))),
        None => Err(DimseError::operation_failed(format!(
            "Instance {} has encapsulated pixel data and no recorded transfer syntax,              so it cannot be sent in the negotiated transfer syntax {}. The instance was not sent",
            sop_instance_uid, negotiated
        ))),
    }
}

/// Read the instance to send from `dataset`: the DICOM object, its SOP UIDs
/// and the transfer syntax it is encoded in.
///
/// The transfer syntax comes from the metadata for `Object` and `Memory`
/// datasets and from the file meta group for a `File` dataset.
async fn parse_dataset(dataset: &DatasetStream) -> Result<StoreInstance> {
    let (obj, transfer_syntax) = match dataset {
        DatasetStream::Object { object, metadata } => {
            (object.clone(), metadata.transfer_syntax.clone())
        }
        DatasetStream::Memory { data, metadata } => {
            // Parse from bytes
            // Try to detect transfer syntax from metadata or default to Implicit VR Little Endian
            let ts_uid = metadata
                .transfer_syntax
                .as_deref()
                .unwrap_or(uids::IMPLICIT_VR_LITTLE_ENDIAN);
            
            let ts = TransferSyntaxRegistry
                .get(ts_uid)
                .ok_or_else(|| DimseError::parse(format!("Transfer syntax not found: {}", ts_uid)))?;
            
            let cursor = std::io::Cursor::new(data.as_ref());
            let object = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
                cursor,
                ts,
                SpecificCharacterSet::default(),
            )
            .map_err(|e| DimseError::parse(format!("Failed to parse dataset: {}", e)))?;
            (object, metadata.transfer_syntax.clone())
        }
        DatasetStream::File { path, .. } => {
            let file = dicom_object::open_file(path)
                .map_err(|e| DimseError::parse(format!("Failed to open DICOM file: {}", e)))?;
            let transfer_syntax = file.meta().transfer_syntax().to_string();
            (file.into_inner(), Some(transfer_syntax))
        }
    };

    // Extract SOP Class UID (required for C-STORE)
    let sop_class_uid = obj
        .element_by_name("SOPClassUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::operation_failed("Dataset missing SOPClassUID"))?;

    // Extract SOP Instance UID (required for C-STORE)
    let sop_instance_uid = obj
        .element_by_name("SOPInstanceUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::operation_failed("Dataset missing SOPInstanceUID"))?;

    Ok(StoreInstance {
        object: obj,
        sop_class_uid,
        sop_instance_uid,
        transfer_syntax,
    })
}

/// The presentation context the remote node accepted for `sop_class_uid`:
/// its ID and its transfer syntax.
fn accepted_store_context(
    association: &AsyncClientAssociation<TcpStream>,
    sop_class_uid: &str,
) -> Option<(u8, String)> {
    association
        .presentation_contexts()
        .iter()
        .find(|pc| trim_uid(&pc.abstract_syntax) == trim_uid(sop_class_uid))
        .map(|pc| (pc.id, pc.transfer_syntax.clone()))
}

/// Handle C-STORE request using native DICOM UL
///
/// The association proposes the transfer syntax of the instance. See
/// `transfer_syntaxes_to_propose`. Pixel data is never transcoded: an
/// instance with encapsulated pixel data is sent in its own transfer syntax
/// or not at all.
pub async fn handle_store(
    config: &DimseConfig,
    node: &RemoteNode,
    dataset: DatasetStream,
) -> Result<bool> {
    info!(
        "Sending C-STORE to {}@{}:{}",
        node.ae_title, node.host, node.port
    );

    node.validate()?;

    // Parse dataset to extract SOP Class and Instance UIDs
    let StoreInstance {
        object: dicom_obj,
        sop_class_uid,
        sop_instance_uid,
        transfer_syntax: instance_transfer_syntax,
    } = parse_dataset(&dataset).await?;

    debug!(
        "C-STORE: SOP Class UID = {}, SOP Instance UID = {}, transfer syntax = {:?}",
        sop_class_uid, sop_instance_uid, instance_transfer_syntax
    );

    let proposed = transfer_syntaxes_to_propose(instance_transfer_syntax.as_deref())?;
    let mut association =
        native_connection::establish_store_association(config, node, &sop_class_uid, &proposed)
            .await?;

    let Some((pc_id, negotiated_transfer_syntax)) =
        accepted_store_context(&association, &sop_class_uid)
    else {
        native_connection::abort_association(association).await;
        return Err(native_connection::store_context_not_accepted(
            node,
            &sop_class_uid,
            &proposed,
        ));
    };

    if let Err(e) = check_pixel_data_matches_negotiated(
        &sop_instance_uid,
        has_encapsulated_pixel_data(&dicom_obj),
        instance_transfer_syntax.as_deref(),
        &negotiated_transfer_syntax,
    ) {
        // Nothing was sent, so the association can end with a normal release.
        if let Err(release_error) = native_connection::release_association(association).await {
            warn!(
                "Failed to release the association after refusing instance {}: {}",
                sop_instance_uid, release_error
            );
        }
        return Err(e);
    }

    let message_id = next_message_id();

    // Build C-STORE request
    // C-STORE-RQ includes: Command Field, Message ID, Affected SOP Class UID, Command Data Set Type, Affected SOP Instance UID
    let mut request = command_builder::build_command_request(
        0x0001, // C-STORE-RQ
        message_id,
        true, // Has dataset
        &sop_class_uid,
    );

    // Add Affected SOP Instance UID (0000,1000) - required for C-STORE
    request.put(DataElement::new(
        tags::AFFECTED_SOP_INSTANCE_UID,
        VR::UI,
        PrimitiveValue::from(sop_instance_uid.clone()),
    ));

    debug!("Sending C-STORE request (message ID: {}, SOP Instance: {})", message_id, sop_instance_uid);

    // The data set is encoded in the transfer syntax of the presentation context.

    // Send the request with dataset
    if let Err(e) = command_builder::encode_and_send_request(&mut association, request, Some(&dicom_obj), pc_id).await {
        native_connection::abort_association(association).await;
        return Err(e);
    }

    // Wait for response
    debug!("Waiting for C-STORE response");
    loop {
        match association.receive().await {
            Ok(Pdu::PData { data }) => {
                // Parse the response
                let (response_obj, _dataset, _pc_id) = match command_builder::parse_response_command(data) {
                    Ok(result) => result,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                };

                // Verify this is the response to our request
                let responded_to = match command_builder::extract_message_id_being_responded_to(&response_obj) {
                    Ok(id) => id,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                };

                if responded_to != message_id {
                    debug!(
                        "Received response for different message ID (expected: {}, got: {}), continuing",
                        message_id, responded_to
                    );
                    continue;
                }

                // Check command field (should be 0x8001 for C-STORE-RSP)
                let command_field = response_obj
                    .element(tags::COMMAND_FIELD)
                    .ok()
                    .and_then(|e| e.uint16().ok())
                    .unwrap_or(0);

                if command_field != 0x8001 {
                    native_connection::abort_association(association).await;
                    return Err(DimseError::operation_failed(format!(
                        "Unexpected command field in response: 0x{:04X}",
                        command_field
                    )));
                }

                // Extract status
                let status = match command_builder::extract_status(&response_obj) {
                    Ok(s) => s,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                };

                // Release association (release takes ownership)
                let _ = native_connection::release_association(association).await;

                // Check status (0x0000 = success)
                if status == 0x0000 {
                    info!("C-STORE completed successfully");
                    return Ok(true);
                } else {
                    return Err(DimseError::operation_failed(format!(
                        "C-STORE failed with status: 0x{:04X}",
                        status
                    )));
                }
            }
            Ok(Pdu::ReleaseRQ) => {
                // Unexpected release
                if let Err(e) = association.send(&Pdu::ReleaseRP).await {
                    native_connection::abort_association(association).await;
                    return Err(DimseError::network(format!("Failed to send release response: {}", e)));
                }
                return Err(DimseError::operation_failed(
                    "Association released unexpectedly during C-STORE",
                ));
            }
            Ok(Pdu::AbortRQ { .. }) => {
                return Err(DimseError::operation_failed(
                    "Association aborted during C-STORE",
                ));
            }
            Ok(pdu) => {
                debug!("Unexpected PDU received: {:?}", pdu);
                // Continue waiting for P-Data
            }
            Err(e) => {
                native_connection::abort_association(association).await;
                return Err(DimseError::network(format!(
                    "Error receiving C-STORE response: {}",
                    e
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_core::value::PixelFragmentSequence;

    const JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";

    fn object_with_pixel_data(value: Value<InMemDicomObject, Vec<u8>>, vr: VR) -> InMemDicomObject {
        let mut object = InMemDicomObject::new_empty();
        object.put(DataElement::new(tags::PIXEL_DATA, vr, value));
        object
    }

    #[test]
    fn proposes_implicit_vr_only_when_no_transfer_syntax_is_recorded() {
        assert_eq!(
            transfer_syntaxes_to_propose(None).unwrap(),
            vec![uids::IMPLICIT_VR_LITTLE_ENDIAN]
        );
    }

    #[test]
    fn proposes_implicit_vr_once_for_an_implicit_vr_instance() {
        assert_eq!(
            transfer_syntaxes_to_propose(Some(uids::IMPLICIT_VR_LITTLE_ENDIAN)).unwrap(),
            vec![uids::IMPLICIT_VR_LITTLE_ENDIAN]
        );
    }

    #[test]
    fn proposes_the_instance_syntax_then_implicit_vr_for_an_uncompressed_instance() {
        assert_eq!(
            transfer_syntaxes_to_propose(Some(uids::EXPLICIT_VR_LITTLE_ENDIAN)).unwrap(),
            vec![
                uids::EXPLICIT_VR_LITTLE_ENDIAN,
                uids::IMPLICIT_VR_LITTLE_ENDIAN
            ]
        );
    }

    #[test]
    fn proposes_only_the_instance_syntax_for_a_compressed_instance() {
        assert_eq!(
            transfer_syntaxes_to_propose(Some(JPEG_LOSSLESS_SV1)).unwrap(),
            vec![JPEG_LOSSLESS_SV1]
        );
    }

    #[test]
    fn proposal_drops_the_padding_of_a_recorded_uid() {
        assert_eq!(
            transfer_syntaxes_to_propose(Some("1.2.840.10008.1.2.4.70\0")).unwrap(),
            vec![JPEG_LOSSLESS_SV1]
        );
    }

    #[test]
    fn proposal_fails_for_a_transfer_syntax_the_registry_does_not_know() {
        let error = transfer_syntaxes_to_propose(Some("1.2.3.4.5.999")).unwrap_err();
        assert!(
            error.to_string().contains("1.2.3.4.5.999"),
            "the error should name the transfer syntax: {}",
            error
        );
    }

    #[test]
    fn a_pixel_sequence_is_encapsulated_pixel_data() {
        let fragments = PixelFragmentSequence::new_fragments(vec![vec![1u8, 2, 3, 4]]);
        let object = object_with_pixel_data(Value::PixelSequence(fragments), VR::OB);
        assert!(has_encapsulated_pixel_data(&object));
    }

    #[test]
    fn native_pixel_values_are_not_encapsulated_pixel_data() {
        let object = object_with_pixel_data(
            Value::Primitive(PrimitiveValue::from(vec![1u8, 2, 3, 4])),
            VR::OB,
        );
        assert!(!has_encapsulated_pixel_data(&object));
    }

    #[test]
    fn an_object_without_pixel_data_has_no_encapsulated_pixel_data() {
        assert!(!has_encapsulated_pixel_data(&InMemDicomObject::new_empty()));
    }

    #[test]
    fn native_pixel_data_may_be_sent_in_any_negotiated_syntax() {
        check_pixel_data_matches_negotiated(
            "1.2.3",
            false,
            Some(uids::EXPLICIT_VR_LITTLE_ENDIAN),
            uids::IMPLICIT_VR_LITTLE_ENDIAN,
        )
        .expect("native pixel data is re-encoded");
        check_pixel_data_matches_negotiated("1.2.3", false, None, uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .expect("native pixel data is re-encoded");
    }

    #[test]
    fn encapsulated_pixel_data_may_be_sent_in_its_own_syntax() {
        check_pixel_data_matches_negotiated(
            "1.2.3",
            true,
            Some("1.2.840.10008.1.2.4.70\0"),
            JPEG_LOSSLESS_SV1,
        )
        .expect("the negotiated syntax is the instance's own");
    }

    #[test]
    fn encapsulated_pixel_data_is_refused_in_another_syntax() {
        let error = check_pixel_data_matches_negotiated(
            "1.2.3",
            true,
            Some(JPEG_LOSSLESS_SV1),
            uids::IMPLICIT_VR_LITTLE_ENDIAN,
        )
        .unwrap_err();
        let message = error.to_string();
        for expected in ["1.2.3", JPEG_LOSSLESS_SV1, uids::IMPLICIT_VR_LITTLE_ENDIAN] {
            assert!(
                message.contains(expected),
                "the error should name {}: {}",
                expected,
                message
            );
        }
    }

    #[test]
    fn encapsulated_pixel_data_is_refused_without_a_recorded_syntax() {
        let error = check_pixel_data_matches_negotiated(
            "1.2.3",
            true,
            None,
            uids::IMPLICIT_VR_LITTLE_ENDIAN,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("no recorded transfer syntax"),
            "the error should say the transfer syntax is missing: {}",
            error
        );
    }
}

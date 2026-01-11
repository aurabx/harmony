//! C-STORE command handler

use std::sync::atomic::{AtomicU16, Ordering};

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::{Pdu};
use tracing::{debug, info};

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

/// Parse dataset from DatasetStream to extract DICOM object and UIDs
async fn parse_dataset(dataset: &DatasetStream) -> Result<(InMemDicomObject<StandardDataDictionary>, String, String)> {
    use dicom_dictionary_std::uids;
    
    let obj = match dataset {
        DatasetStream::Object { object, .. } => object.clone(),
        DatasetStream::Memory { data, .. } => {
            // Parse from bytes
            // Try to detect transfer syntax from metadata or default to Implicit VR Little Endian
            let ts_uid = dataset.metadata()
                .transfer_syntax
                .as_deref()
                .unwrap_or(uids::IMPLICIT_VR_LITTLE_ENDIAN);
            
            let ts = TransferSyntaxRegistry
                .get(ts_uid)
                .ok_or_else(|| DimseError::parse(format!("Transfer syntax not found: {}", ts_uid)))?;
            
            let cursor = std::io::Cursor::new(data.as_ref());
            InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
                cursor,
                ts,
                SpecificCharacterSet::default(),
            )
            .map_err(|e| DimseError::parse(format!("Failed to parse dataset: {}", e)))?
        }
        DatasetStream::File { path, .. } => {
            // Parse from file
            dicom_object::open_file(path)
                .map_err(|e| DimseError::parse(format!("Failed to open DICOM file: {}", e)))?
                .into_inner()
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

    Ok((obj, sop_class_uid, sop_instance_uid))
}

/// Handle C-STORE request using native DICOM UL
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
    let (dicom_obj, sop_class_uid, sop_instance_uid) = parse_dataset(&dataset).await?;

    debug!(
        "C-STORE: SOP Class UID = {}, SOP Instance UID = {}",
        sop_class_uid, sop_instance_uid
    );

    // Establish association with the dataset's SOP Class
    let sop_class_uids = vec![&sop_class_uid as &str];
    let mut association = native_connection::establish_association(config, node, &sop_class_uids)
        .await?;

    // Get presentation context ID
    let pc_id = match native_connection::get_presentation_context_id(&association, &sop_class_uid) {
        Ok(id) => id,
        Err(e) => {
            native_connection::abort_association(association).await;
            return Err(e);
        }
    };

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

    // Transfer syntax will be determined from the presentation context when encoding

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

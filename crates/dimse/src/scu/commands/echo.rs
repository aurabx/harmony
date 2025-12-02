//! C-ECHO command handler

use std::sync::atomic::{AtomicU16, Ordering};

use dicom_ul::Pdu;
use tracing::{debug, info};

use crate::config::{DimseConfig, RemoteNode};
use crate::scu::command_builder;
use crate::scu::native_connection;
use crate::{DimseError, Result};

// Verification SOP Class UID
const VERIFICATION_SOP_CLASS: &str = "1.2.840.10008.1.1";

// Message ID counter (thread-safe)
static MESSAGE_ID_COUNTER: AtomicU16 = AtomicU16::new(1);

fn next_message_id() -> u16 {
    MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Handle C-ECHO request using native DICOM UL
pub async fn handle_echo(
    config: &DimseConfig,
    node: &RemoteNode,
) -> Result<bool> {
    info!(
        "Sending C-ECHO to {}@{}:{}",
        node.ae_title, node.host, node.port
    );

    node.validate()?;

    // Establish association with Verification SOP Class
    let sop_class_uids = vec![VERIFICATION_SOP_CLASS];
    let mut association = native_connection::establish_association(config, node, &sop_class_uids)
        .await?;

    // Get presentation context ID
    let pc_id = match native_connection::get_presentation_context_id(&association, VERIFICATION_SOP_CLASS) {
        Ok(id) => id,
        Err(e) => {
            let _ = association.abort().await;
            return Err(e);
        }
    };

    let message_id = next_message_id();

    // Build C-ECHO request
    let request = command_builder::build_command_request(
        0x0030, // C-ECHO-RQ
        message_id,
        false, // No dataset
        VERIFICATION_SOP_CLASS,
    );

    debug!("Sending C-ECHO request (message ID: {})", message_id);

    // Send the request
    if let Err(e) = command_builder::encode_and_send_request(&mut association, request, None, pc_id).await {
        let _ = association.abort().await;
        return Err(e);
    }

    // Wait for response
    debug!("Waiting for C-ECHO response");
    loop {
        match association.receive().await {
            Ok(Pdu::PData { data }) => {
                // Parse the response
                let (response_obj, _dataset, _pc_id) = match command_builder::parse_response_command(data) {
                    Ok(result) => result,
                    Err(e) => {
                        let _ = association.abort().await;
                        return Err(e);
                    }
                };

                // Verify this is the response to our request
                let responded_to = match command_builder::extract_message_id_being_responded_to(&response_obj) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = association.abort().await;
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

                // Check command field (should be 0x8030 for C-ECHO-RSP)
                let command_field = response_obj
                    .element(dicom_dictionary_std::tags::COMMAND_FIELD)
                    .ok()
                    .and_then(|e| e.uint16().ok())
                    .unwrap_or(0);

                if command_field != 0x8030 {
                    let _ = association.abort().await;
                    return Err(DimseError::operation_failed(format!(
                        "Unexpected command field in response: 0x{:04X}",
                        command_field
                    )));
                }

                // Extract status
                let status = match command_builder::extract_status(&response_obj) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = association.abort().await;
                        return Err(e);
                    }
                };

                // Release association (release takes ownership, so we move it)
                let _ = native_connection::release_association(association).await;

                // Check status (0x0000 = success)
                if status == 0x0000 {
                    info!("C-ECHO completed successfully");
                    return Ok(true);
                } else {
                    return Err(DimseError::operation_failed(format!(
                        "C-ECHO failed with status: 0x{:04X}",
                        status
                    )));
                }
            }
            Ok(Pdu::ReleaseRQ) => {
                // Unexpected release - send release response
                if let Err(e) = association.send(&Pdu::ReleaseRP).await {
                    let _ = association.abort().await;
                    return Err(DimseError::network(format!("Failed to send release response: {}", e)));
                }
                return Err(DimseError::operation_failed(
                    "Association released unexpectedly during C-ECHO",
                ));
            }
            Ok(Pdu::AbortRQ { .. }) => {
                // Association was aborted by remote
                return Err(DimseError::operation_failed(
                    "Association aborted during C-ECHO",
                ));
            }
            Ok(pdu) => {
                debug!("Unexpected PDU received: {:?}", pdu);
                // Continue waiting for P-Data
            }
            Err(e) => {
                // Abort takes ownership
                native_connection::abort_association(association).await;
                return Err(DimseError::network(format!(
                    "Error receiving C-ECHO response: {}",
                    e
                )));
            }
        }
    }
}

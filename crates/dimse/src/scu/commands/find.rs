//! C-FIND command handler

use std::sync::atomic::{AtomicU16, Ordering};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info};

use crate::config::{DimseConfig, RemoteNode};
use crate::scu::command_builder;
use crate::scu::native_connection;
use crate::types::{DatasetStream, FindQuery};
use crate::{DimseError, Result};

// Message ID counter (thread-safe)
static MESSAGE_ID_COUNTER: AtomicU16 = AtomicU16::new(1);

fn next_message_id() -> u16 {
    MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// Study Root Query/Retrieve Information Model - FIND SOP Class
const FIND_SOP_CLASS_STUDY: &str = "1.2.840.10008.5.1.4.1.2.2.1";
// Patient Root Query/Retrieve Information Model - FIND SOP Class
const FIND_SOP_CLASS_PATIENT: &str = "1.2.840.10008.5.1.4.1.2.1.1";

/// Handle C-FIND request using native DICOM UL
pub async fn handle_find(
    config: &DimseConfig,
    node: &RemoteNode,
    query: FindQuery,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    info!(
        "Sending C-FIND to {}@{}:{} (level: {}, max_results: {})",
        node.ae_title, node.host, node.port, query.query_level, query.max_results
    );

    node.validate()?;
    debug!("C-FIND query parameters: {:?}", query.parameters);

    // Build streaming channel for results
    let (tx, rx) = mpsc::channel(100);

    // Spawn task to handle the C-FIND operation
    let config_clone = config.clone();
    let node_clone = node.clone();
    let query_clone = query.clone();
    let tx_clone = tx.clone();

    tokio::spawn(async move {
        let result = perform_find(&config_clone, &node_clone, &query_clone, tx_clone).await;
        if let Err(e) = result {
            tracing::error!("C-FIND operation failed: {}", e);
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(stream)
}

/// Perform the actual C-FIND operation
async fn perform_find(
    config: &DimseConfig,
    node: &RemoteNode,
    query: &FindQuery,
    tx: mpsc::Sender<Result<DatasetStream>>,
) -> Result<()> {
    // Establish association with Query/Retrieve FIND SOP Class
    // Select model based on query level:
    // - Patient level queries require Patient Root
    // - Study/Series/Image level queries can use Study Root (preferred) or Patient Root
    let sop_class_uids: Vec<&str> = match query.query_level {
        crate::types::QueryLevel::Patient => vec![FIND_SOP_CLASS_PATIENT],
        _ => vec![FIND_SOP_CLASS_STUDY, FIND_SOP_CLASS_PATIENT], // Study/Series/Image prefer Study Root
    };
    
    let mut association = native_connection::establish_association(config, node, &sop_class_uids)
        .await?;

    // Get presentation context ID - prefer the model that matches the query level
    let pc_id = match query.query_level {
        crate::types::QueryLevel::Patient => {
            match native_connection::get_presentation_context_id(&association, FIND_SOP_CLASS_PATIENT) {
                Ok(id) => (id, FIND_SOP_CLASS_PATIENT),
                Err(e) => {
                    native_connection::abort_association(association).await;
                    return Err(e);
                }
            }
        }
        _ => {
            // Prefer Study Root, fall back to Patient Root
            match native_connection::get_presentation_context_id(&association, FIND_SOP_CLASS_STUDY) {
                Ok(id) => (id, FIND_SOP_CLASS_STUDY),
                Err(_) => match native_connection::get_presentation_context_id(&association, FIND_SOP_CLASS_PATIENT) {
                    Ok(id) => (id, FIND_SOP_CLASS_PATIENT),
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                }
            }
        }
    };

    let message_id = next_message_id();

    // Build identifier dataset from query parameters
    let identifier = command_builder::build_identifier_dataset(query.query_level, &query.parameters)?;

    // Build C-FIND request with the accepted SOP class
    let (pc_id, accepted_sop_class) = pc_id;
    let request = command_builder::build_command_request(
        0x0020, // C-FIND-RQ
        message_id,
        true, // Has dataset
        accepted_sop_class,
    );

    debug!("Sending C-FIND request (message ID: {})", message_id);

    // Send the request with identifier dataset
    if let Err(e) = command_builder::encode_and_send_request(&mut association, request, Some(&identifier), pc_id).await {
        native_connection::abort_association(association).await;
        let _ = tx.send(Err(e)).await;
        return Ok(()); // Channel is closed
    }

    // Receive and stream responses
    let mut results_count = 0u32;
    let max_results = if query.max_results > 0 {
        query.max_results
    } else {
        u32::MAX // No limit
    };

    loop {
        // Use receive_dimse_message to handle split PDUs
        match command_builder::receive_dimse_message(&mut association).await {
            Ok((response_obj, dataset_data, _pc_id)) => {
                // Verify this is the response to our request
                let responded_to = match command_builder::extract_message_id_being_responded_to(&response_obj) {
                    Ok(id) => id,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                };

                if responded_to != message_id {
                    debug!(
                        "Received response for different message ID (expected: {}, got: {}), continuing",
                        message_id, responded_to
                    );
                    continue;
                }

                // Check command field (should be 0x8020 for C-FIND-RSP)
                let command_field = response_obj
                    .element(dicom_dictionary_std::tags::COMMAND_FIELD)
                    .ok()
                    .and_then(|e| e.uint16().ok())
                    .unwrap_or(0);

                if command_field != 0x8020 {
                    native_connection::abort_association(association).await;
                    let _ = tx.send(Err(DimseError::operation_failed(format!(
                        "Unexpected command field in response: 0x{:04X}",
                        command_field
                    )))).await;
                    break;
                }

                // Extract status
                let status = match command_builder::extract_status(&response_obj) {
                    Ok(s) => s,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                };

                // Handle different status codes
                match status {
                    0x0000 => {
                        // Success - no more results
                        debug!("C-FIND completed successfully (received {} results)", results_count);
                        // Release association
                        let _ = native_connection::release_association(association).await;
                        break;
                    }
                    0xFF00 | 0xFF01 => {
                        // Pending - more results to come
                        if let Some(dataset_bytes) = dataset_data {
                            results_count += 1;
                            if results_count > max_results {
                                // Reached max results, close channel and release
                                let _ = native_connection::release_association(association).await;
                                break;
                            }

                            // Parse dataset bytes to DICOM object
                            match command_builder::parse_dataset_bytes(dataset_bytes, pc_id, &association) {
                                Ok(dicom_obj) => {
                                    // Convert to DatasetStream
                                    let dataset = DatasetStream::from_object(dicom_obj);
                                    if tx.send(Ok(dataset)).await.is_err() {
                                        // Receiver closed, abort and exit
                                        native_connection::abort_association(association).await;
                                        break;
                                    }
                                }
                                Err(e) => {
                                    debug!("Failed to parse C-FIND result dataset: {}", e);
                                    // Continue with next response
                                }
                            }
                        }
                        // Continue waiting for more responses
                    }
                    _ => {
                        // Failure or cancellation
                        native_connection::abort_association(association).await;
                        let _ = tx.send(Err(DimseError::operation_failed(format!(
                            "C-FIND failed with status: 0x{:04X}",
                            status
                        )))).await;
                        break;
                    }
                }
            }
            Err(e) => {
                native_connection::abort_association(association).await;
                let _ = tx.send(Err(DimseError::network(format!(
                    "Error receiving C-FIND response: {}",
                    e
                )))).await;
                break;
            }
        }
    }

    info!("C-FIND completed ({} results)", results_count);
    Ok(())
}


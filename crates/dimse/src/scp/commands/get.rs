//! C-GET command handler

use std::net::SocketAddr;

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use futures::StreamExt;
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

use crate::types::DatasetStream;
use crate::{DimseError, Result};

use crate::scp::response_builder::{self, SubOperationCounts};
use crate::scp::DimseScp;

use super::extract_query_parameters;

/// Send a Pending response after this many sub-operations. Pending responses
/// carry the remaining count, so they are only sent when the total is known.
const PENDING_RESPONSE_INTERVAL: u16 = 10;

/// Outcome of one C-STORE sub-operation towards the client.
enum SubOperationResult {
    Completed,
    Warning,
    Failed,
}

/// Handle C-GET request
pub async fn handle_c_get(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    peer_addr: SocketAddr,
    message_id: u16,
    identifier_data: Vec<u8>,
    presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_get {
        return Err(DimseError::operation_failed("C-GET not enabled"));
    }

    debug!(
        "Handling C-GET request (message ID: {}, identifier size: {} bytes)",
        message_id,
        identifier_data.len()
    );

    // Parse identifier
    let ts = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == presentation_context_id)
        .and_then(|pc| TransferSyntaxRegistry.get(&pc.transfer_syntax))
        .ok_or_else(|| {
            DimseError::parse(format!(
                "Transfer syntax not found for presentation context {}",
                presentation_context_id
            ))
        })?;

    let cursor = std::io::Cursor::new(&identifier_data);
    let identifier = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| DimseError::parse(format!("Failed to parse identifier dataset: {}", e)))?;

    // Extract parameters
    let (query_level, parameters) = extract_query_parameters(&identifier);

    debug!(
        "C-GET query: level={}, params={:?}",
        query_level, parameters
    );

    // Start the retrieval. Datasets arrive as a stream and each one is sent to
    // the client as soon as it is available.
    let retrieve = match scp
        .query_provider
        .get(peer_addr, query_level, &parameters)
        .await
    {
        Ok(retrieve) => retrieve,
        Err(e) => {
            error!("C-GET query failed: {}", e);
            response_builder::send_get_response(
                association,
                message_id,
                e.status_code().unwrap_or(0xC000),
                SubOperationCounts::default(),
                presentation_context_id,
            )
            .await?;
            return Ok(());
        }
    };

    let expected_total = retrieve.expected_total;
    let mut datasets = retrieve.datasets;
    debug!(
        "C-GET streaming sub-operations (expected total: {:?})",
        expected_total
    );

    let mut counts = SubOperationCounts {
        remaining: expected_total,
        ..SubOperationCounts::default()
    };
    let mut sub_operation: u16 = 0;
    // Status of the first stream item that carried no dataset because the
    // retrieval itself failed, when the error states one.
    let mut first_item_failure_status: Option<u16> = None;

    while let Some(item) = datasets.next().await {
        if expected_total.is_some()
            && sub_operation > 0
            && sub_operation % PENDING_RESPONSE_INTERVAL == 0
        {
            response_builder::send_get_response(
                association,
                message_id,
                0xFF00, // Pending
                counts.clone(),
                presentation_context_id,
            )
            .await?;
        }
        sub_operation = sub_operation.wrapping_add(1);

        let outcome = match item {
            Ok(dataset) => send_sub_operation(association, sub_operation, &dataset).await?,
            Err(e) => {
                error!(
                    "C-GET sub-operation {} has no dataset: {}",
                    sub_operation, e
                );
                if first_item_failure_status.is_none() {
                    first_item_failure_status = e.status_code();
                }
                SubOperationResult::Failed
            }
        };

        match outcome {
            SubOperationResult::Completed => counts.completed += 1,
            SubOperationResult::Warning => counts.warning += 1,
            SubOperationResult::Failed => counts.failed += 1,
        }
        counts.remaining = expected_total
            .map(|total| total.saturating_sub(counts.completed + counts.warning + counts.failed));
    }

    // The final response never carries a remaining count.
    counts.remaining = None;
    let status = match first_item_failure_status {
        // Nothing reached the client because the retrieval failed, so the
        // retrieval's own status is the answer.
        Some(code) if counts.completed == 0 && counts.warning == 0 => code,
        _ if counts.failed > 0 => {
            if counts.completed > 0 {
                0xB000 // One or more failures (Warning)
            } else {
                0xA702 // Unable to perform sub-operations (Refused)
            }
        }
        _ => 0x0000, // Success
    };

    info!(
        "C-GET completed with status 0x{:04X} ({} completed, {} warning, {} failed)",
        status, counts.completed, counts.warning, counts.failed
    );

    response_builder::send_get_response(
        association,
        message_id,
        status,
        counts,
        presentation_context_id,
    )
    .await?;

    Ok(())
}

/// Send one dataset to the client as a C-STORE sub-operation and wait for its
/// response. A lost client connection is returned as an error so the C-GET
/// stops instead of failing every remaining instance one by one.
async fn send_sub_operation(
    association: &mut AsyncServerAssociation<TcpStream>,
    message_id: u16,
    dataset: &DatasetStream,
) -> Result<SubOperationResult> {
    // The SOP Class UID selects the presentation context for the sub-operation
    let (sop_class_uid, sop_instance_uid) = match parse_dataset_uids(dataset).await {
        Ok(uids) => uids,
        Err(e) => {
            error!("Failed to parse dataset UIDs: {}", e);
            return Ok(SubOperationResult::Failed);
        }
    };

    let Some(storage_pc_id) = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.abstract_syntax == sop_class_uid)
        .map(|pc| pc.id)
    else {
        error!(
            "No presentation context accepted for SOP Class: {}",
            sop_class_uid
        );
        return Ok(SubOperationResult::Failed);
    };

    if let Err(e) = response_builder::send_store_suboperation(
        association,
        message_id,
        dataset,
        &sop_class_uid,
        &sop_instance_uid,
        storage_pc_id,
    )
    .await
    {
        if matches!(e, DimseError::Network(_)) {
            return Err(e);
        }
        error!("Failed to send C-STORE sub-operation: {}", e);
        return Ok(SubOperationResult::Failed);
    }

    match response_builder::receive_store_response(association).await {
        Ok(0x0000) => Ok(SubOperationResult::Completed),
        Ok(status) if status & 0xF000 == 0xB000 => {
            warn!("C-STORE sub-operation warning status: 0x{:04X}", status);
            Ok(SubOperationResult::Warning)
        }
        Ok(status) => {
            error!("C-STORE sub-operation failed with status: 0x{:04X}", status);
            Ok(SubOperationResult::Failed)
        }
        Err(e) => {
            if matches!(e, DimseError::Network(_)) {
                return Err(e);
            }
            error!("Failed to receive C-STORE response: {}", e);
            Ok(SubOperationResult::Failed)
        }
    }
}

/// Helper to extract UIDs from DatasetStream
async fn parse_dataset_uids(dataset: &DatasetStream) -> Result<(String, String)> {
    let obj = dataset.to_object().await?;

    let sop_class = obj
        .element_by_name("SOPClassUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::parse("Missing SOPClassUID"))?;

    let sop_instance = obj
        .element_by_name("SOPInstanceUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::parse("Missing SOPInstanceUID"))?;

    Ok((sop_class, sop_instance))
}

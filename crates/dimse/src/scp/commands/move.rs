//! C-MOVE command handler

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use futures::StreamExt;
use tokio::net::TcpStream;
use tracing::{debug, error, info};

use crate::{DimseError, Result};

use crate::scp::response_builder::{self, SubOperationCounts};
use crate::scp::DimseScp;

use super::extract_query_parameters;

/// Send a Pending response after this many sub-operations. Pending responses
/// carry the remaining count, so they are only sent when the total is known.
const PENDING_RESPONSE_INTERVAL: u16 = 10;

/// Handle C-MOVE request
pub async fn handle_c_move(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    message_id: u16,
    identifier_data: Vec<u8>,
    presentation_context_id: u8,
    move_destination: Option<String>,
) -> Result<()> {
    if !scp.config.enable_move {
        return Err(DimseError::operation_failed("C-MOVE not enabled"));
    }

    // MoveDestination comes from the command set (tag 0000,0600), not the identifier
    let move_destination = match move_destination {
        Some(dest) if !dest.is_empty() => dest,
        _ => {
            error!("C-MOVE request missing MoveDestination in command set");
            response_builder::send_move_response(
                association,
                message_id,
                0xC000, // Unable to process
                SubOperationCounts::default(),
                presentation_context_id,
            )
            .await?;
            return Err(DimseError::parse("Missing MoveDestination"));
        }
    };

    debug!(
        "Handling C-MOVE request (message ID: {}, identifier size: {} bytes, dest: {})",
        message_id,
        identifier_data.len(),
        move_destination
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

    // Resolve Destination AET
    let Some(dest_node) = scp.config.aet_table.get(&move_destination) else {
        error!("Unknown C-MOVE destination AET: {}", move_destination);
        response_builder::send_move_response(
            association,
            message_id,
            0xA801, // Refused: Move Destination Unknown
            SubOperationCounts::default(),
            presentation_context_id,
        )
        .await?;
        return Ok(());
    };

    // Extract parameters
    let (query_level, parameters) = extract_query_parameters(&identifier);

    debug!(
        "C-MOVE query: dest={}, level={}, params={:?}",
        move_destination, query_level, parameters
    );

    // Start the retrieval. Each dataset is stored to the destination as soon
    // as it arrives.
    let retrieve = match scp.query_provider.get(query_level, &parameters).await {
        Ok(retrieve) => retrieve,
        Err(e) => {
            error!("C-MOVE query failed: {}", e);
            response_builder::send_move_response(
                association,
                message_id,
                0xC000, // Unable to process
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
        "C-MOVE streaming sub-operations (expected total: {:?})",
        expected_total
    );

    let mut counts = SubOperationCounts {
        remaining: expected_total,
        ..SubOperationCounts::default()
    };
    let mut sub_operation: u16 = 0;

    // Initialize SCU
    let scu = crate::DimseScu::new(scp.config.clone());

    while let Some(item) = datasets.next().await {
        if expected_total.is_some()
            && sub_operation > 0
            && sub_operation % PENDING_RESPONSE_INTERVAL == 0
        {
            response_builder::send_move_response(
                association,
                message_id,
                0xFF00, // Pending
                counts.clone(),
                presentation_context_id,
            )
            .await?;
        }
        sub_operation = sub_operation.wrapping_add(1);

        match item {
            Ok(dataset) => match scu.store(dest_node, dataset).await {
                Ok(true) => counts.completed += 1,
                Ok(false) => {
                    counts.failed += 1;
                    error!("C-STORE to {} failed", move_destination);
                }
                Err(e) => {
                    counts.failed += 1;
                    error!("C-STORE to {} failed: {}", move_destination, e);
                }
            },
            Err(e) => {
                counts.failed += 1;
                error!(
                    "C-MOVE sub-operation {} has no dataset: {}",
                    sub_operation, e
                );
            }
        }
        counts.remaining = expected_total.map(|total| {
            total.saturating_sub(counts.completed + counts.warning + counts.failed)
        });
    }

    // The final response never carries a remaining count.
    counts.remaining = None;
    let status = if counts.failed > 0 {
        0xB000 // Warning (one or more failures)
    } else {
        0x0000 // Success
    };

    info!(
        "C-MOVE completed with status 0x{:04X} ({} completed, {} failed)",
        status, counts.completed, counts.failed
    );

    response_builder::send_move_response(
        association,
        message_id,
        status,
        counts,
        presentation_context_id,
    )
    .await?;

    Ok(())
}

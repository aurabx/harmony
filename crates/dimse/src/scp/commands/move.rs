//! C-MOVE command handler

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, error, info};

use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder::{self, SubOperationCounts};

use super::extract_query_parameters;

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
    let dest_node = scp.config.aet_table.get(&move_destination);

    if dest_node.is_none() {
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
    }
    let dest_node = dest_node.unwrap();

    // Extract parameters
    let (query_level, parameters) = extract_query_parameters(&identifier);

    debug!(
        "C-MOVE query: dest={}, level={}, params={:?}",
        move_destination, query_level, parameters
    );

    // Retrieve datasets
    let datasets = match scp.query_provider.get(query_level, &parameters).await {
        Ok(ds) => ds,
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

    let total = datasets.len();
    debug!("C-MOVE found {} datasets to move", total);

    let mut counts = SubOperationCounts {
        remaining: total as u16,
        completed: 0,
        failed: 0,
        warning: 0,
    };

    // Initialize SCU
    let scu = crate::DimseScu::new(scp.config.clone());

    // Iterate and move datasets
    for (i, dataset) in datasets.into_iter().enumerate() {
        // Send pending response every 10 items
        if i > 0 && i % 10 == 0 {
            response_builder::send_move_response(
                association,
                message_id,
                0xFF00, // Pending
                counts.clone(),
                presentation_context_id,
            )
            .await?;
        }

        // Perform C-STORE to destination using SCU
        match scu.store(dest_node, dataset).await {
            Ok(success) => {
                if success {
                    counts.completed += 1;
                } else {
                    counts.failed += 1;
                    error!("C-STORE to {} failed", move_destination);
                }
            }
            Err(e) => {
                error!("C-STORE to {} failed: {}", move_destination, e);
                counts.failed += 1;
            }
        }

        counts.remaining -= 1;
    }

    // Send final response
    let status = if counts.failed > 0 {
        0xB000 // Warning (one or more failures)
    } else {
        0x0000 // Success
    };

    response_builder::send_move_response(
        association,
        message_id,
        status,
        counts,
        presentation_context_id,
    )
    .await?;

    info!("C-MOVE completed with status 0x{:04X}", status);
    Ok(())
}

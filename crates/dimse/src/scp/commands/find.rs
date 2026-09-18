//! C-FIND command handler

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use futures::StreamExt;
use tokio::net::TcpStream;
use tracing::{debug, error, info};

use crate::{DimseError, Result};

use super::extract_query_parameters;
use crate::scp::response_builder;
use crate::scp::DimseScp;

/// Handle C-FIND request
pub async fn handle_c_find(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    message_id: u16,
    identifier_data: Vec<u8>,
    presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_find {
        return Err(DimseError::operation_failed("C-FIND not enabled"));
    }

    debug!(
        "Handling C-FIND request (message ID: {}, identifier size: {} bytes)",
        message_id,
        identifier_data.len()
    );

    // Check if we have identifier data
    if identifier_data.is_empty() {
        tracing::warn!("C-FIND request has no identifier data");
        // Send failure response
        response_builder::send_find_response(
            association,
            message_id,
            0xC000, // Failure
            None,
            presentation_context_id,
        )
        .await?;
        return Ok(());
    }

    // Get the transfer syntax for this presentation context
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

    // Extract query level and parameters
    let (query_level, parameters) = extract_query_parameters(&identifier);

    debug!(
        "C-FIND query: level={}, params={:?}",
        query_level, parameters
    );

    // Query the provider. Matches are sent as they arrive.
    let mut datasets = match scp.query_provider.find(query_level, &parameters, 0).await {
        Ok(datasets) => datasets,
        Err(e) => {
            error!("C-FIND query failed: {}", e);
            // Send failure response (status 0xC000)
            response_builder::send_find_response(
                association,
                message_id,
                0xC000, // Failure
                None,
                presentation_context_id,
            )
            .await?;
            return Ok(());
        }
    };

    let mut sent = 0usize;
    let mut failed = 0usize;
    while let Some(item) = datasets.next().await {
        match item {
            Ok(dataset) => {
                // Send each dataset as a pending response (status 0xFF00)
                response_builder::send_find_response(
                    association,
                    message_id,
                    0xFF00, // Pending
                    Some(&dataset),
                    presentation_context_id,
                )
                .await?;
                sent += 1;
            }
            Err(e) => {
                error!("C-FIND result could not be produced: {}", e);
                failed += 1;
            }
        }
    }

    // A result that could not be produced makes the whole query a failure,
    // otherwise the client would take a partial answer for a complete one.
    let status = if failed > 0 { 0xC000 } else { 0x0000 };
    response_builder::send_find_response(
        association,
        message_id,
        status,
        None,
        presentation_context_id,
    )
    .await?;

    info!(
        "C-FIND completed with status 0x{:04X} ({} results, {} failed)",
        status, sent, failed
    );
    Ok(())
}

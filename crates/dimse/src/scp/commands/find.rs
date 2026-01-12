//! C-FIND command handler

use dicom_core::header::Header;
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, error, info};

use crate::types::QueryLevel;
use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder;

/// Extract query parameters from identifier dataset
///
/// This extracts ALL tags from the identifier, not just a hardcoded subset.
/// In DICOM C-FIND, any tag present in the identifier with a value is a filter,
/// and any tag with an empty value is a "return key" (requested in results).
fn extract_query_parameters(
    identifier: &InMemDicomObject<StandardDataDictionary>,
) -> (QueryLevel, std::collections::HashMap<String, String>) {
    // Extract query level from QueryRetrieveLevel (0008,0052)
    let query_level_str = identifier
        .element_by_name("QueryRetrieveLevel")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "STUDY".to_string());

    let query_level = query_level_str
        .parse::<QueryLevel>()
        .unwrap_or(QueryLevel::Study);

    // Extract ALL query parameters from the identifier
    // We iterate over all elements and convert tags to DICOM JSON format (GGGGEEEE)
    let mut parameters = std::collections::HashMap::new();

    for elem in identifier.iter() {
        let tag = elem.tag();
        // Format tag as 8-character hex string (e.g., "00100020" for PatientID)
        let tag_str = format!("{:04X}{:04X}", tag.group(), tag.element());
        
        // Skip the QueryRetrieveLevel tag itself - it's handled separately
        if tag_str == "00080052" {
            continue;
        }
        
        // Try to extract string value
        let value = elem.to_str()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        
        parameters.insert(tag_str, value);
    }

    debug!("Extracted {} parameters from identifier", parameters.len());
    (query_level, parameters)
}

/// Handle C-FIND request
pub async fn handle_c_find(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
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

    // Query the provider
    match scp.query_provider.find(query_level, &parameters, 0).await {
        Ok(datasets) => {
            debug!("Found {} matching datasets", datasets.len());

            // Send each dataset as a pending response (status 0xFF00)
            for dataset in &datasets {
                response_builder::send_find_response(
                    association,
                    message_id,
                    0xFF00, // Pending
                    Some(dataset),
                    presentation_context_id,
                )
                .await?;
            }

            // Send final success response (status 0x0000)
            response_builder::send_find_response(
                association,
                message_id,
                0x0000, // Success
                None,
                presentation_context_id,
            )
            .await?;

            info!("C-FIND completed with {} results", datasets.len());
            Ok(())
        }
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
            Ok(())
        }
    }
}

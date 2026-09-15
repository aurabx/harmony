//! C-GET command handler

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, error, info, warn};

use crate::types::DatasetStream;
use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder::{self, SubOperationCounts};

use super::extract_query_parameters;

/// Handle C-GET request
pub async fn handle_c_get(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
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

    // Retrieve datasets
    let datasets = match scp.query_provider.get(query_level, &parameters).await {
        Ok(ds) => ds,
        Err(e) => {
            error!("C-GET query failed: {}", e);
            response_builder::send_get_response(
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
    debug!("C-GET found {} datasets to retrieve", total);

    let mut counts = SubOperationCounts {
        remaining: total as u16,
        completed: 0,
        failed: 0,
        warning: 0,
    };

    // Iterate and send sub-operations
    for (i, dataset) in datasets.into_iter().enumerate() {
        // Send pending response every 10 items to keep association alive/informed
        if i > 0 && i % 10 == 0 {
            response_builder::send_get_response(
                association,
                message_id,
                0xFF00, // Pending
                counts.clone(),
                presentation_context_id,
            )
            .await?;
        }

        // Parse dataset to get SOP Class UID
        // We need this to select the presentation context for the C-STORE sub-operation
        let (sop_class_uid, sop_instance_uid) = match parse_dataset_uids(&dataset).await {
            Ok(uids) => uids,
            Err(e) => {
                error!("Failed to parse dataset UIDs: {}", e);
                counts.failed += 1;
                counts.remaining -= 1;
                continue;
            }
        };

        // Find presentation context for this SOP Class
        let storage_pc_id = association
            .presentation_contexts()
            .iter()
            .find(|pc| pc.abstract_syntax == sop_class_uid)
            .map(|pc| pc.id);

        if let Some(storage_pc_id) = storage_pc_id {
            // Send C-STORE-RQ sub-operation
            match response_builder::send_store_suboperation(
                association,
                (i as u16) + 1, // New Message ID for sub-operation
                &dataset,
                &sop_class_uid,
                &sop_instance_uid,
                storage_pc_id,
            )
            .await
            {
                Ok(_) => {
                    // Wait for C-STORE-RSP
                    match response_builder::receive_store_response(association).await {
                        Ok(status) => {
                            if status == 0x0000 {
                                counts.completed += 1;
                            } else if status & 0xB000 == 0xB000 {
                                counts.warning += 1;
                                warn!("C-STORE sub-operation warning status: 0x{:04X}", status);
                            } else {
                                counts.failed += 1;
                                error!("C-STORE sub-operation failed with status: 0x{:04X}", status);
                            }
                        }
                        Err(e) => {
                            error!("Failed to receive C-STORE response: {}", e);
                            counts.failed += 1;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to send C-STORE sub-operation: {}", e);
                    counts.failed += 1;
                }
            }
        } else {
            error!("No presentation context accepted for SOP Class: {}", sop_class_uid);
            counts.failed += 1;
        }

        counts.remaining -= 1;
    }

    // Send final response
    let status = if counts.failed > 0 {
        if counts.completed > 0 {
             0xB000 // One or more failures (Warning)
        } else {
             0xA702 // Unable to perform sub-operations (Refused)
        }
    } else {
        0x0000 // Success
    };

    response_builder::send_get_response(
        association,
        message_id,
        status,
        counts,
        presentation_context_id,
    )
    .await?;

    info!("C-GET completed with status 0x{:04X}", status);
    Ok(())
}

/// Helper to extract UIDs from DatasetStream
async fn parse_dataset_uids(dataset: &DatasetStream) -> Result<(String, String)> {
    let obj = dataset.to_object().await?;
    
    let sop_class = obj.element_by_name("SOPClassUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::parse("Missing SOPClassUID"))?;
        
    let sop_instance = obj.element_by_name("SOPInstanceUID")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DimseError::parse("Missing SOPInstanceUID"))?;
        
    Ok((sop_class, sop_instance))
}

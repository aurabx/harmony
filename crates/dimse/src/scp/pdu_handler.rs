//! PDU parsing, accumulation, and routing

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_dictionary_std::{tags, uids};
use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;
use tracing::debug;

use crate::{DimseError, Result};

use super::DimseScp;
use super::commands;

/// Handle P-DATA PDU containing DIMSE command
pub async fn handle_pdata(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
    pdata: Vec<dicom_ul::pdu::PDataValue>,
    pending_command: &mut Option<(u16, u16)>,
    accumulated_identifier: &mut Vec<u8>,
) -> Result<()> {
    // Separate command and data PDUs, track presentation context ID
    let mut command_data = Vec::new();
    let mut identifier_data = Vec::new();
    let mut presentation_context_id = 1u8; // Default to 1

    for pdata_value in pdata {
        presentation_context_id = pdata_value.presentation_context_id;
        match pdata_value.value_type {
            dicom_ul::pdu::PDataValueType::Command => {
                command_data.extend_from_slice(&pdata_value.data);
            }
            dicom_ul::pdu::PDataValueType::Data => {
                identifier_data.extend_from_slice(&pdata_value.data);
            }
        }
    }

    // Check if we have command data
    if command_data.is_empty() {
        // This is a data-only PDU - accumulate it for pending command
        if !identifier_data.is_empty() {
            debug!(
                "Received data-only P-DATA ({} bytes), accumulating",
                identifier_data.len()
            );
            accumulated_identifier.extend_from_slice(&identifier_data);

            // Check if we have a pending command to dispatch
            if let Some((command_field, message_id)) = *pending_command {
                // Dispatch the command with accumulated data
                debug!(
                    "Dispatching pending command 0x{:04X} with {} bytes of data",
                    command_field,
                    accumulated_identifier.len()
                );
                commands::dispatch_command(
                    scp,
                    association,
                    command_field,
                    message_id,
                    accumulated_identifier.clone(),
                    presentation_context_id,
                )
                .await?;
                *pending_command = None;
                accumulated_identifier.clear();
            }
        }
        return Ok(());
    }

    // Parse the command dataset using Implicit VR Little Endian (DICOM command PDUs use this)
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

    let cursor = std::io::Cursor::new(&command_data);
    let command_obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| {
        DimseError::parse(format!(
            "Failed to parse command dataset ({} bytes): {}",
            command_data.len(),
            e
        ))
    })?;

    // Extract command field to determine operation type
    let command_field = command_obj
        .element(tags::COMMAND_FIELD)
        .map_err(|_| DimseError::parse("Missing command field"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid command field: {}", e)))?;

    // Extract message ID for response correlation
    let message_id = command_obj
        .element(tags::MESSAGE_ID)
        .map_err(|_| DimseError::parse("Missing message ID"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid message ID: {}", e)))?;

    debug!(
        "Received DIMSE command: 0x{:04X}, message ID: {}",
        command_field, message_id
    );

    // Check if this command expects a dataset
    let expects_dataset = command_obj
        .element(tags::COMMAND_DATA_SET_TYPE)
        .ok()
        .and_then(|e| e.uint16().ok())
        .map(|v| v != 0x0101) // 0x0101 = no dataset present
        .unwrap_or(false);

    // If we have identifier data in this PDU, use it immediately
    if !identifier_data.is_empty() {
        debug!(
            "Command has {} bytes of identifier data in same PDU",
            identifier_data.len()
        );
        return commands::dispatch_command(
            scp,
            association,
            command_field,
            message_id,
            identifier_data,
            presentation_context_id,
        )
        .await;
    }

    // If command expects dataset but we don't have it yet, buffer the command
    if expects_dataset {
        debug!("Command expects dataset, buffering command for next PDU");
        *pending_command = Some((command_field, message_id));
        return Ok(());
    }

    // No dataset expected, dispatch immediately
    commands::dispatch_command(
        scp,
        association,
        command_field,
        message_id,
        Vec::new(),
        presentation_context_id,
    )
    .await
}

//! P-DATA handling: reassembly of DIMSE messages and routing to commands

use dicom_dictionary_std::tags;
use dicom_ul::association::server::AsyncServerAssociation;
use tokio::net::TcpStream;
use tracing::debug;

use crate::{DimseError, Result};

use super::commands;
use super::message_assembler::{DimseMessage, MessageAssembler};
use super::DimseScp;

/// Handle one P-DATA PDU.
///
/// Each PDV goes to the assembler. A command is dispatched only when its
/// message is complete, so a data set that spans several PDUs reaches the
/// command handler whole.
pub async fn handle_pdata(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    pdata: Vec<dicom_ul::pdu::PDataValue>,
    assembler: &mut MessageAssembler,
) -> Result<()> {
    for pdata_value in pdata {
        if let Some(message) = assembler.push(pdata_value)? {
            dispatch_message(scp, association, message).await?;
        }
    }
    Ok(())
}

/// Read the routing fields of a complete message and run its command.
async fn dispatch_message(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    message: DimseMessage,
) -> Result<()> {
    let command_field = message
        .command
        .element(tags::COMMAND_FIELD)
        .map_err(|_| DimseError::parse("Missing command field"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid command field: {}", e)))?;

    // The message ID correlates the response with the request.
    let message_id = message
        .command
        .element(tags::MESSAGE_ID)
        .map_err(|_| DimseError::parse("Missing message ID"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid message ID: {}", e)))?;

    // Only C-MOVE requests carry MoveDestination (0000,0600).
    let move_destination = message
        .command
        .element(tags::MOVE_DESTINATION)
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.trim().to_string());

    debug!(
        "Dispatching DIMSE command 0x{:04X}, message ID: {}, move_dest: {:?}, data set: {} bytes",
        command_field,
        message_id,
        move_destination,
        message.data.len()
    );

    commands::dispatch_command(
        scp,
        association,
        command_field,
        message_id,
        message.data,
        message.presentation_context_id,
        move_destination,
    )
    .await
}

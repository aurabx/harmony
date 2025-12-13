//! Command handlers for DIMSE operations

pub mod echo;
pub mod find;
pub mod get;
pub mod r#move;
pub mod store;

use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;

use crate::DimseError;
use crate::Result;

use crate::scp::DimseScp;

/// Dispatch a DIMSE command to the appropriate handler
pub async fn dispatch_command(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
    command_field: u16,
    message_id: u16,
    identifier_data: Vec<u8>,
    presentation_context_id: u8,
) -> Result<()> {
    // Dispatch based on command type
    match command_field {
        0x0030 => {
            // C-ECHO-RQ
            echo::handle_c_echo(scp, association, message_id, presentation_context_id).await
        }
        0x0020 => {
            // C-FIND-RQ
            find::handle_c_find(
                scp,
                association,
                message_id,
                identifier_data,
                presentation_context_id,
            )
            .await
        }
        0x0021 => {
            // C-MOVE-RQ
            r#move::handle_c_move(
                scp,
                association,
                message_id,
                identifier_data,
                presentation_context_id,
            )
            .await
        }
        0x0010 => {
            // C-GET-RQ
            get::handle_c_get(
                scp,
                association,
                message_id,
                identifier_data,
                presentation_context_id,
            )
            .await
        }
        0x0001 => {
            // C-STORE-RQ
            store::handle_c_store(
                scp,
                association,
                message_id,
                identifier_data,
                presentation_context_id,
            )
            .await
        }
        _ => {
            tracing::warn!("Unknown DIMSE command: 0x{:04X}", command_field);
            Err(DimseError::operation_failed(format!(
                "Unsupported command: 0x{:04X}",
                command_field
            )))
        }
    }
}

//! C-ECHO command handler

use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;
use tracing::debug;

use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder;

/// Handle C-ECHO request
pub async fn handle_c_echo(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_echo {
        return Err(DimseError::operation_failed("C-ECHO not enabled"));
    }

    debug!("Handling C-ECHO request (message ID: {})", message_id);

    response_builder::send_echo_response(association, message_id, presentation_context_id).await
}

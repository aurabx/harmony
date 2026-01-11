//! C-GET command handler

use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder::{self, SubOperationCounts};

/// Handle C-GET request (stub - not fully implemented)
pub async fn handle_c_get(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    _identifier_data: Vec<u8>,
    presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_get {
        return Err(DimseError::operation_failed("C-GET not enabled"));
    }

    debug!(
        "Handling C-GET request (message ID: {}, identifier size: {} bytes)",
        message_id,
        _identifier_data.len()
    );
    warn!("C-GET operation not fully implemented - returning 'Unable to perform sub-operations' status");

    // Send failure response with "Unable to perform sub-operations" status
    // Status 0xA702 = Unable to perform sub-operations
    response_builder::send_get_response(
        association,
        message_id,
        0xA702,
        SubOperationCounts {
            remaining: 0,
            completed: 0,
            failed: 0,
            warning: 0,
        },
        presentation_context_id,
    )
    .await?;

    info!("C-GET request handled with 'not implemented' status");
    Ok(())
}

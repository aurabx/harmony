//! C-MOVE command handler

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_dictionary_std::uids;
use dicom_ul::ServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder::{self, SubOperationCounts};

/// Handle C-MOVE request (stub - not fully implemented)
pub async fn handle_c_move(
    scp: &DimseScp,
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    identifier_data: Vec<u8>,
    _presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_move {
        return Err(DimseError::operation_failed("C-MOVE not enabled"));
    }

    debug!(
        "Handling C-MOVE request (message ID: {}, identifier size: {} bytes)",
        message_id,
        identifier_data.len()
    );
    warn!("C-MOVE operation not fully implemented - returning 'Unable to perform sub-operations' status");

    // Parse identifier to log query info
    if !identifier_data.is_empty() {
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

        if let Ok(identifier) =
            InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
                std::io::Cursor::new(&identifier_data),
                ts,
                SpecificCharacterSet::default(),
            )
        {
            // Try to extract move destination
            if let Ok(dest) = identifier.element_by_name("MoveDestination") {
                if let Ok(dest_aet) = dest.to_str() {
                    debug!("C-MOVE destination AET: {}", dest_aet);
                }
            }
        }
    }

    // Send failure response with "Unable to perform sub-operations" status
    // Status 0xA702 = Unable to perform sub-operations
    response_builder::send_move_response(
        association,
        message_id,
        0xA702,
        SubOperationCounts {
            remaining: 0,
            completed: 0,
            failed: 0,
            warning: 0,
        },
        _presentation_context_id,
    )
    .await?;

    info!("C-MOVE request handled with 'not implemented' status");
    Ok(())
}

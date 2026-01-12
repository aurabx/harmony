//! Association establishment and lifecycle management

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use dicom_ul::association::server::{AcceptAny, ServerAssociationOptions};
use dicom_ul::Pdu;
use tracing::{error, info, warn};

use crate::{DimseError, Result};

use super::DimseScp;
use super::pdu_handler;

/// Handle a single association
pub async fn handle_association(
    scp: &DimseScp,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
) -> Result<()> {
    // Increment active associations
    {
        scp.active_associations.fetch_add(1, Ordering::Relaxed);
    }

    let result = handle_association_inner(scp, stream, peer_addr).await;

    // Decrement active associations
    scp.active_associations.fetch_sub(1, Ordering::Relaxed);


    result
}

/// Inner association handler
async fn handle_association_inner(
    scp: &DimseScp,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
) -> Result<()> {
    info!("Starting association with {}", peer_addr);

    // Build server association options based on config
    // Use promiscuous mode to accept any presentation context
    let scp_options = ServerAssociationOptions::new()
        .ae_title(scp.config.local_aet.as_str())
        .ae_access_control(AcceptAny)
        .max_pdu_length(scp.config.max_pdu)
        .promiscuous(true);

    // Establish the association
    let mut association = scp_options.establish_async(stream).await.map_err(|e| {
        DimseError::association(format!("Failed to establish association: {}", e))
    })?;

    // Log negotiated PDU sizes for debugging
    info!(
        "Association established with {} (calling AET: {}, requestor_max_pdu: {})",
        peer_addr,
        association.client_ae_title(),
        association.requestor_max_pdu_length()
    );

    // Buffer for accumulating identifier data across multiple PDUs
    let mut pending_command: Option<(u16, u16)> = None; // (command_field, message_id)
    let mut accumulated_identifier = Vec::new();

    // Process PDUs until association is released or aborted
    loop {
        match association.receive().await {
            Ok(Pdu::PData { data }) => {
                // Handle P-DATA PDU containing DIMSE commands
                if let Err(e) = pdu_handler::handle_pdata(
                    scp,
                    &mut association,
                    data,
                    &mut pending_command,
                    &mut accumulated_identifier,
                )
                .await
                {
                    error!("Error handling P-DATA: {}", e);
                    // Send abort and break
                    let _ = association.abort().await;
                    break;
                }
            }
            Ok(Pdu::ReleaseRQ) => {
                // Association release requested by SCU
                info!("Association release requested by {}", peer_addr);
                association.send(&Pdu::ReleaseRP).await.map_err(|e| {
                    DimseError::network(format!("Failed to send release: {}", e))
                })?;
                break;
            }
            Ok(Pdu::AbortRQ { source }) => {
                // Association aborted by SCU
                warn!("Association aborted by {}: {:?}", peer_addr, source);
                break;
            }
            Ok(pdu) => {
                // Unexpected PDU
                warn!("Unexpected PDU received: {:?}", pdu);
                let _ = association.abort().await;
                break;
            }
            Err(e) => {
                error!("Error receiving PDU from {}: {}", peer_addr, e);
                let _ = association.abort().await;
                break;
            }
        }
    }

    info!("Association with {} completed", peer_addr);
    Ok(())
}

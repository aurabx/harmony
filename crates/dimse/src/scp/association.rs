//! Association establishment and lifecycle management

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use dicom_ul::association::server::{AccessControl, ServerAssociationOptions};
use dicom_ul::pdu::{AssociationRJServiceUserReason, UserIdentity};
use dicom_ul::prelude::*;
use dicom_ul::Pdu;
use tracing::{debug, error, info, warn};

use crate::{DimseError, Result};

use super::commands;
use super::message_assembler::MessageAssembler;
use super::pdu_handler;
use super::{DimseScp, PeerAdmission};

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

    let verdict = peer_verdict(scp, peer_addr).await;

    // Build server association options based on config
    // Use promiscuous mode to accept any presentation context
    let scp_options = ServerAssociationOptions::new()
        .ae_title(scp.config.local_aet.as_str())
        .ae_access_control(verdict)
        .max_pdu_length(scp.config.max_pdu)
        .promiscuous(true);

    if !verdict.admit {
        // The rejection was logged by `peer_verdict`. Reading the request only
        // serves to answer it with an A-ASSOCIATE-RJ, so a silent peer is cut
        // off after the connect timeout instead of holding an association slot.
        match tokio::time::timeout(
            scp.config.connect_timeout(),
            scp_options.establish_async(stream),
        )
        .await
        {
            Ok(Ok(_)) => {
                error!(
                    "Association from {} was accepted although the peer was rejected; closing it",
                    peer_addr
                );
            }
            Ok(Err(e)) => debug!("Sent A-ASSOCIATE-RJ to {}: {}", peer_addr, e),
            Err(_) => debug!(
                "Rejected peer {} sent no association request in time",
                peer_addr
            ),
        }
        return Ok(());
    }

    // Establish the association
    let mut association = scp_options
        .establish_async(stream)
        .await
        .map_err(|e| DimseError::association(format!("Failed to establish association: {}", e)))?;

    // Log negotiated PDU sizes for debugging
    info!(
        "Association established with {} (calling AET: {}, requestor_max_pdu: {})",
        peer_addr,
        association.peer_ae_title(),
        association.requestor_max_pdu_length()
    );

    // A DIMSE message can span several PDUs, so its fragments are collected
    // across iterations of the loop below.
    let mut assembler = MessageAssembler::new(
        scp.config.max_dataset_bytes,
        commands::commands_with_data_sets(&scp.config),
    );

    // Process PDUs until association is released or aborted
    loop {
        match association.receive().await {
            Ok(Pdu::PData { data }) => {
                // Handle P-DATA PDU containing DIMSE commands
                if let Err(e) = pdu_handler::handle_pdata(
                    scp,
                    &mut association,
                    peer_addr,
                    data,
                    &mut assembler,
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
                association
                    .send(&Pdu::ReleaseRP)
                    .await
                    .map_err(|e| DimseError::network(format!("Failed to send release: {}", e)))?;
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

/// Ask the query provider whether `peer_addr` may open an association.
///
/// A provider error counts as a reject, so the SCP fails closed.
async fn peer_verdict(scp: &DimseScp, peer_addr: SocketAddr) -> PeerVerdict {
    match scp.query_provider.admit_peer(peer_addr).await {
        Ok(PeerAdmission::Admit) => PeerVerdict { admit: true },
        Ok(PeerAdmission::Reject { reason }) => {
            warn!("Association from {} rejected: {}", peer_addr, reason);
            PeerVerdict { admit: false }
        }
        Err(e) => {
            error!(
                "Association from {} rejected: peer admission check failed: {}",
                peer_addr, e
            );
            PeerVerdict { admit: false }
        }
    }
}

/// Access control for one association, holding a decision made in advance.
///
/// `dicom_ul` checks access synchronously and without the peer address, so
/// the async peer check runs first and its result is carried in here. A
/// rejected peer gets a permanent A-ASSOCIATE-RJ from `dicom_ul`.
#[derive(Debug, Clone, Copy)]
struct PeerVerdict {
    admit: bool,
}

impl AccessControl for PeerVerdict {
    fn check_access(
        &self,
        _this_ae_title: &str,
        _calling_ae_title: &str,
        _called_ae_title: &str,
        _user_identity: Option<&UserIdentity>,
    ) -> std::result::Result<(), AssociationRJServiceUserReason> {
        if self.admit {
            Ok(())
        } else {
            Err(AssociationRJServiceUserReason::NoReasonGiven)
        }
    }
}

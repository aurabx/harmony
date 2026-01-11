//! Native client association management for SCU operations

use dicom_dictionary_std::uids;
use dicom_ul::association::client::ClientAssociationOptions;
use dicom_ul::ClientAssociation;
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::{DimseError, Result};

/// Establish a client association with a remote DICOM node
pub async fn establish_association(
    config: &DimseConfig,
    node: &RemoteNode,
    sop_class_uids: &[&str],
) -> Result<ClientAssociation<TcpStream>> {
    node.validate()?;

    let addr = format!("{}:{}", node.host, node.port);
    info!("Connecting to {}@{}", node.ae_title, addr);

    // Build client association options
    let mut client_options = ClientAssociationOptions::new()
        .calling_ae_title(config.local_aet.as_str())
        .called_ae_title(node.ae_title.as_str());

    // Set maximum PDU size
    let max_pdu = node.max_pdu.unwrap_or(config.max_pdu);
    client_options = client_options.max_pdu_length(max_pdu);

    // Add presentation contexts for each SOP class
    // Each SOP class gets its own presentation context with preferred transfer syntaxes
    for sop_class_uid in sop_class_uids {
        // Add presentation context with multiple transfer syntax options
        // @todo why this list?
        let transfer_syntaxes = vec![
            uids::IMPLICIT_VR_LITTLE_ENDIAN.into(),
            // uids::EXPLICIT_VR_LITTLE_ENDIAN.into(),
            // uids::EXPLICIT_VR_BIG_ENDIAN.into(),
        ];
        // Dereference &&str to &str
        client_options = client_options.with_presentation_context(*sop_class_uid, transfer_syntaxes);
    }

    // Establish the association (establish_async takes an address, not a stream)
    let association = client_options
        .establish_async(&addr)
        .await
        .map_err(|e| {
            DimseError::association(format!("Failed to establish association with {}: {}", addr, e))
        })?;

    info!(
        "Association established with {}@{} (called AET: {})",
        node.ae_title, addr, node.ae_title
    );

    Ok(association)
}

/// Release an association gracefully
/// Note: This consumes the association since release() takes ownership
pub async fn release_association(
    association: ClientAssociation<TcpStream>,
) -> Result<()> {
    association
        .release()
        .await
        .map_err(|e| DimseError::network(format!("Failed to release association: {}", e)))?;
    Ok(())
}

/// Abort an association
/// Note: This consumes the association since abort() takes ownership
pub async fn abort_association(association: ClientAssociation<TcpStream>) {
    if let Err(e) = association.abort().await {
        warn!("Failed to abort association: {}", e);
    }
}

/// Get presentation context ID for a SOP class from an established association
pub fn get_presentation_context_id(
    association: &ClientAssociation<TcpStream>,
    sop_class_uid: &str,
) -> Result<u8> {
    // Find the presentation context ID for this SOP class
    for pc in association.presentation_contexts() {
        // abstract_syntax is a field
        if pc.abstract_syntax == sop_class_uid {
            // id is the presentation context ID field
            return Ok(pc.id);
        }
    }

    Err(DimseError::association(format!(
        "No presentation context found for SOP class: {}",
        sop_class_uid
    )))
}


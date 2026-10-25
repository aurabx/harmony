//! Native client association management for SCU operations

use dicom_dictionary_std::uids;
use dicom_ul::association::client::ClientAssociationOptions;
use dicom_ul::association::client::AsyncClientAssociation;
use dicom_ul::association::Error as AssociationError;
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::config::{DimseConfig, RemoteNode};
use crate::{DimseError, Result};

/// Establish a client association with a remote DICOM node
pub async fn establish_association(
    config: &DimseConfig,
    node: &RemoteNode,
    sop_class_uids: &[&str],
) -> Result<AsyncClientAssociation<TcpStream>> {
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
        .map_err(|e| match e {
            // The node is up and talking DICOM but supports none of the
            // proposed SOP classes. Callers can tell this apart from a
            // connection failure, for example to try another operation.
            AssociationError::NoAcceptedPresentationContexts { .. } => {
                DimseError::NotSupported(format!(
                    "{}@{} accepted none of the proposed SOP classes [{}]",
                    node.ae_title,
                    addr,
                    sop_class_uids.join(", ")
                ))
            }
            other => DimseError::association(format!(
                "Failed to establish association with {}: {}",
                addr, other
            )),
        })?;

    info!(
        "Association established with {}@{} (called AET: {})",
        node.ae_title, addr, node.ae_title
    );

    Ok(association)
}

/// Establish a client association for the C-STORE of one instance.
///
/// One presentation context is proposed: `sop_class_uid` with
/// `transfer_syntaxes`, in order of preference. The caller chooses the list
/// from the instance, because an instance with encapsulated pixel data can
/// only be sent in its own transfer syntax.
///
/// Fails with an error that names the SOP class and the proposed transfer
/// syntaxes when the remote node accepts none of them. dicom-ul has already
/// aborted the association in that case.
pub async fn establish_store_association(
    config: &DimseConfig,
    node: &RemoteNode,
    sop_class_uid: &str,
    transfer_syntaxes: &[String],
) -> Result<AsyncClientAssociation<TcpStream>> {
    node.validate()?;
    if transfer_syntaxes.is_empty() {
        return Err(DimseError::internal(format!(
            "No transfer syntax to propose for the C-STORE of SOP class {}",
            sop_class_uid
        )));
    }

    let addr = format!("{}:{}", node.host, node.port);
    info!(
        "Connecting to {}@{} for C-STORE (SOP class {}, transfer syntaxes {})",
        node.ae_title,
        addr,
        sop_class_uid,
        transfer_syntaxes.join(", ")
    );

    let max_pdu = node.max_pdu.unwrap_or(config.max_pdu);
    let proposed: Vec<&str> = transfer_syntaxes.iter().map(String::as_str).collect();

    let association = ClientAssociationOptions::new()
        .calling_ae_title(config.local_aet.as_str())
        .called_ae_title(node.ae_title.as_str())
        .max_pdu_length(max_pdu)
        .with_presentation_context(sop_class_uid, proposed)
        .establish_async(&addr)
        .await
        .map_err(|e| match e {
            // dicom-ul reports a refused presentation context this way when
            // no other context was accepted. Only one is proposed here.
            AssociationError::NoAcceptedPresentationContexts { .. } => {
                store_context_not_accepted(node, sop_class_uid, transfer_syntaxes)
            }
            other => DimseError::association(format!(
                "Failed to establish association with {}: {}",
                addr, other
            )),
        })?;

    info!(
        "C-STORE association established with {}@{}",
        node.ae_title, addr
    );

    Ok(association)
}

/// The error for a C-STORE that cannot go ahead because the remote node did
/// not accept the presentation context for the instance.
pub fn store_context_not_accepted(
    node: &RemoteNode,
    sop_class_uid: &str,
    transfer_syntaxes: &[String],
) -> DimseError {
    DimseError::association(format!(
        "{}@{}:{} accepted no presentation context for SOP class {} with the proposed transfer syntaxes [{}]",
        node.ae_title,
        node.host,
        node.port,
        sop_class_uid,
        transfer_syntaxes.join(", ")
    ))
}

/// Establish a client association for a C-GET.
///
/// A C-GET SCU receives the retrieved instances as C-STORE sub-operations on
/// the same association. Besides the Query/Retrieve context it therefore
/// proposes one presentation context per storage SOP class, with the SCP role
/// requested for each of them, and offers `config.storage_transfer_syntaxes`
/// on those contexts. The SCP may accept only some of the storage classes.
pub async fn establish_retrieve_association(
    config: &DimseConfig,
    node: &RemoteNode,
    retrieve_sop_class: &str,
    storage_sop_classes: &[&str],
) -> Result<AsyncClientAssociation<TcpStream>> {
    node.validate()?;
    if config.storage_transfer_syntaxes.is_empty() {
        return Err(DimseError::config(
            "storage_transfer_syntaxes is empty, so no storage presentation context can be proposed for C-GET",
        ));
    }

    let addr = format!("{}:{}", node.host, node.port);
    info!(
        "Connecting to {}@{} for C-GET ({} storage SOP classes proposed)",
        node.ae_title,
        addr,
        storage_sop_classes.len()
    );

    let max_pdu = node.max_pdu.unwrap_or(config.max_pdu);
    let storage_transfer_syntaxes: Vec<&str> = config
        .storage_transfer_syntaxes
        .iter()
        .map(String::as_str)
        .collect();

    let mut client_options = ClientAssociationOptions::new()
        .calling_ae_title(config.local_aet.as_str())
        .called_ae_title(node.ae_title.as_str())
        .max_pdu_length(max_pdu)
        .with_presentation_context(retrieve_sop_class, vec![uids::IMPLICIT_VR_LITTLE_ENDIAN]);

    for sop_class_uid in storage_sop_classes {
        client_options = client_options
            .with_presentation_context(*sop_class_uid, storage_transfer_syntaxes.clone())
            .with_role_selection(*sop_class_uid, false, true);
    }

    let association = client_options
        .establish_async(&addr)
        .await
        .map_err(|e| {
            DimseError::association(format!(
                "Failed to establish C-GET association with {}: {}",
                addr, e
            ))
        })?;

    info!(
        "C-GET association established with {}@{} ({} presentation contexts accepted)",
        node.ae_title,
        addr,
        association.presentation_contexts().len()
    );

    Ok(association)
}

/// Release an association gracefully
/// Note: This consumes the association since release() takes ownership
pub async fn release_association(
    association: AsyncClientAssociation<TcpStream>,
) -> Result<()> {
    association
        .release()
        .await
        .map_err(|e| DimseError::network(format!("Failed to release association: {}", e)))?;
    Ok(())
}

/// Abort an association
/// Note: This consumes the association since abort() takes ownership
pub async fn abort_association(association: AsyncClientAssociation<TcpStream>) {
    if let Err(e) = association.abort().await {
        warn!("Failed to abort association: {}", e);
    }
}

/// Get presentation context ID for a SOP class from an established association
pub fn get_presentation_context_id(
    association: &AsyncClientAssociation<TcpStream>,
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

    Err(DimseError::NotSupported(format!(
        "No presentation context found for SOP class: {}",
        sop_class_uid
    )))
}


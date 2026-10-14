//! C-STORE command handler

use std::net::SocketAddr;

use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::AsyncServerAssociation;
use tokio::net::TcpStream;
use tracing::{debug, error};

use crate::types::DatasetStream;
use crate::{DimseError, Result};

use crate::scp::DimseScp;
use crate::scp::response_builder;

/// Handle C-STORE request
pub async fn handle_c_store(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    peer_addr: SocketAddr,
    message_id: u16,
    dataset_data: Vec<u8>,
    presentation_context_id: u8,
) -> Result<()> {
    if !scp.config.enable_store {
        return Err(DimseError::operation_failed("C-STORE not enabled"));
    }

    debug!(
        "Handling C-STORE request (message ID: {}, dataset size: {} bytes)",
        message_id,
        dataset_data.len()
    );

    // Get the transfer syntax for this presentation context
    let ts = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == presentation_context_id)
        .and_then(|pc| TransferSyntaxRegistry.get(&pc.transfer_syntax))
        .ok_or_else(|| {
            DimseError::parse(format!(
                "Transfer syntax not found for presentation context {}",
                presentation_context_id
            ))
        })?;

    // Parse the dataset
    let cursor = std::io::Cursor::new(&dataset_data);
    let obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| DimseError::parse(format!("Failed to parse C-STORE dataset: {}", e)))?;
    // The parsed object holds its own copy of every value. Free the encoded
    // bytes now, so a large instance is not held twice while it is stored.
    drop(dataset_data);

    // The parsed object keeps encapsulated pixel data as it arrived, so the
    // transfer syntax must travel with it. Without it a later writer would
    // label the instance with a transfer syntax its pixel data does not match.
    let mut dataset = DatasetStream::from_object(obj);
    dataset.metadata_mut().transfer_syntax = Some(ts.uid().to_string());

    // Store the dataset
    match scp.query_provider.store(peer_addr, dataset).await {
        Ok(()) => {
            // Send success response
            response_builder::send_store_response(association, message_id, 0x0000, presentation_context_id)
                .await
        }
        Err(e) => {
            error!("Failed to store dataset: {}", e);
            // Send failure response (0xC000 = Error: Cannot Understand)
            response_builder::send_store_response(association, message_id, 0xC000, presentation_context_id)
                .await
        }
    }
}

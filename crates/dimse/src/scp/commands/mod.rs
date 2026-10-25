//! Command handlers for DIMSE operations

pub mod echo;
pub mod find;
pub mod get;
pub mod r#move;
pub mod store;

use std::net::SocketAddr;

use dicom_ul::association::server::AsyncServerAssociation;
use tokio::net::TcpStream;

use crate::DimseError;
use crate::Result;

use crate::scp::DimseScp;
use crate::types::QueryLevel;
use dicom_core::header::Header;
use dicom_object::{InMemDicomObject, StandardDataDictionary};

/// Extract query parameters from identifier dataset
///
/// This extracts ALL tags from the identifier, not just a hardcoded subset.
/// In DICOM C-FIND, any tag present in the identifier with a value is a filter,
/// and any tag with an empty value is a "return key" (requested in results).
pub fn extract_query_parameters(
    identifier: &InMemDicomObject<StandardDataDictionary>,
) -> (QueryLevel, std::collections::HashMap<String, String>) {
    // Extract query level from QueryRetrieveLevel (0008,0052)
    let query_level_str = identifier
        .element_by_name("QueryRetrieveLevel")
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "STUDY".to_string());

    let query_level = query_level_str
        .parse::<QueryLevel>()
        .unwrap_or(QueryLevel::Study);

    // Extract ALL query parameters from the identifier
    // We iterate over all elements and convert tags to DICOM JSON format (GGGGEEEE)
    let mut parameters = std::collections::HashMap::new();

    for elem in identifier.iter() {
        let tag = elem.tag();
        // Format tag as 8-character hex string (e.g., "00100020" for PatientID)
        let tag_str = format!("{:04X}{:04X}", tag.group(), tag.element());

        // Skip the QueryRetrieveLevel tag itself - it's handled separately
        if tag_str == "00080052" {
            continue;
        }

        // Try to extract string value
        let value = elem
            .to_str()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        parameters.insert(tag_str, value);
    }

    tracing::debug!("Extracted {} parameters from identifier", parameters.len());
    (query_level, parameters)
}

const C_STORE_RQ: u16 = 0x0001;
const C_GET_RQ: u16 = 0x0010;
const C_FIND_RQ: u16 = 0x0020;
const C_MOVE_RQ: u16 = 0x0021;

/// The command fields whose data set the SCP reads, given what is enabled.
///
/// C-ECHO carries no data set, so it is never listed.
pub fn commands_with_data_sets(config: &crate::config::DimseConfig) -> Vec<u16> {
    [
        (config.enable_store, C_STORE_RQ),
        (config.enable_get, C_GET_RQ),
        (config.enable_find, C_FIND_RQ),
        (config.enable_move, C_MOVE_RQ),
    ]
    .into_iter()
    .filter(|(enabled, _)| *enabled)
    .map(|(_, command_field)| command_field)
    .collect()
}

/// Dispatch a DIMSE command to the appropriate handler
pub async fn dispatch_command(
    scp: &DimseScp,
    association: &mut AsyncServerAssociation<TcpStream>,
    peer_addr: SocketAddr,
    command_field: u16,
    message_id: u16,
    identifier_data: Vec<u8>,
    presentation_context_id: u8,
    move_destination: Option<String>,
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
                peer_addr,
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
                peer_addr,
                message_id,
                identifier_data,
                presentation_context_id,
                move_destination,
            )
            .await
        }
        0x0010 => {
            // C-GET-RQ
            get::handle_c_get(
                scp,
                association,
                peer_addr,
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
                peer_addr,
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

//! Response building and encoding helpers for DIMSE operations

use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_ul::{Pdu, ServerAssociation};
use tokio::net::TcpStream;

use crate::common::{
    build_response, command_fields, create_command_pdata, create_data_pdata, encode_command,
    status, DimseMessageBuilder,
};
use crate::types::DatasetStream;
use crate::{DimseError, Result};

// Re-export for backwards compatibility
pub use crate::common::SubOperationCounts;

/// Build a command response object with common fields
///
/// This is a convenience wrapper around the common build_response function.
pub fn build_command_response(
    command_field: u16,
    message_id: u16,
    status_code: u16,
    has_dataset: bool,
    sop_class_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    build_response(command_field, message_id, status_code, has_dataset, sop_class_uid)
}

/// Encode and send a response with optional dataset
///
/// This function properly fragments large datasets according to the negotiated
/// maximum PDU size from the requestor (client).
pub async fn encode_and_send_response(
    association: &mut ServerAssociation<TcpStream>,
    response: InMemDicomObject<StandardDataDictionary>,
    dataset: Option<&DatasetStream>,
    presentation_context_id: u8,
) -> Result<()> {
    let response_bytes = encode_command(&response)?;

    // Command PDV is always complete (not fragmented), so is_last must be true.
    // The presence of a following dataset is indicated by COMMAND_DATA_SET_TYPE field.
    let command_pdata = create_command_pdata(presentation_context_id, response_bytes, true);

    // Send command first
    association
        .send(&Pdu::PData {
            data: vec![command_pdata],
        })
        .await
        .map_err(|e| DimseError::network(format!("Failed to send command response: {}", e)))?;

    // If we have a dataset, encode and send it (possibly fragmented)
    if let Some(ds) = dataset {
        // Convert the dataset to a DICOM object
        let dicom_obj = ds.to_object().await?;

        // Encode the dataset using the negotiated transfer syntax for this presentation context
        use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
        use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
        let ts_uid = association
            .presentation_contexts()
            .iter()
            .find(|pc| pc.id == presentation_context_id)
            .map(|pc| pc.transfer_syntax.as_str())
            .ok_or_else(|| DimseError::operation_failed("Presentation context not found"))?;
        
        tracing::debug!("Encoding dataset with transfer syntax: {}", ts_uid);
        
        let ts = TransferSyntaxRegistry
            .get(ts_uid)
            .ok_or_else(|| DimseError::operation_failed(format!("Transfer syntax not found: {}", ts_uid)))?;

        let mut dataset_bytes = Vec::new();
        dicom_obj
            .write_dataset_with_ts(&mut dataset_bytes, ts)
            .map_err(|e| DimseError::operation_failed(format!("Failed to encode dataset: {}", e)))?;
        
        tracing::debug!("Encoded dataset: {} bytes", dataset_bytes.len());

        // Fragment dataset if larger than requestor's max PDU size
        // PDU header (6 bytes) + P-DATA-TF item header (4 bytes) + PDV header (6 bytes) = 16 bytes overhead
        const PDU_OVERHEAD: usize = 16;
        let max_pdu = association.requestor_max_pdu_length() as usize;
        let max_data_per_pdu = if max_pdu > PDU_OVERHEAD {
            max_pdu - PDU_OVERHEAD
        } else {
            // Fallback to small chunks if max_pdu is too small
            4096
        };

        let chunks: Vec<&[u8]> = dataset_bytes.chunks(max_data_per_pdu).collect();
        let num_chunks = chunks.len();

        tracing::debug!(
            "Sending dataset in {} chunk(s) (max_pdu: {}, max_data_per_pdu: {})",
            num_chunks, max_pdu, max_data_per_pdu
        );

        for (i, chunk) in chunks.into_iter().enumerate() {
            let is_last = i == num_chunks - 1;
            let data_pdata = create_data_pdata(presentation_context_id, chunk.to_vec(), is_last);

            association
                .send(&Pdu::PData {
                    data: vec![data_pdata],
                })
                .await
                .map_err(|e| DimseError::network(format!("Failed to send dataset fragment {}/{}: {}", i + 1, num_chunks, e)))?;
        }
    }

    Ok(())
}

/// Build and send a C-ECHO response
pub async fn send_echo_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    presentation_context_id: u8,
) -> Result<()> {
    let response = build_response(
        command_fields::C_ECHO_RSP,
        message_id,
        status::SUCCESS,
        false,
        "1.2.840.10008.1.1", // Verification SOP Class
    );

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    tracing::info!("C-ECHO response sent successfully");
    Ok(())
}

/// Build and send a C-FIND response
pub async fn send_find_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    dataset: Option<&DatasetStream>,
    presentation_context_id: u8,
) -> Result<()> {
    // Use the negotiated Abstract Syntax (SOP Class UID) for this presentation context.
    // This ensures we reply with Patient Root vs Study Root correctly based on the client's request.
    let sop_class_uid = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == presentation_context_id)
        .map(|pc| pc.abstract_syntax.as_str())
        // Fallback to Study Root FIND if not found (should not happen in practice)
        .unwrap_or("1.2.840.10008.5.1.4.1.2.2.1");

    let response = build_response(
        command_fields::C_FIND_RSP,
        message_id,
        status_code,
        dataset.is_some(),
        sop_class_uid,
    );

    encode_and_send_response(association, response, dataset, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-MOVE response
pub async fn send_move_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    counts: SubOperationCounts,
    presentation_context_id: u8,
) -> Result<()> {
    let response = DimseMessageBuilder::new()
        .command_field(command_fields::C_MOVE_RSP)
        .message_id_being_responded_to(message_id)
        .status(status_code)
        .has_dataset(false)
        .affected_sop_class_uid("1.2.840.10008.5.1.4.1.2.2.2") // Study Root Query/Retrieve - MOVE
        .sub_operation_counts(&counts)
        .build();

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-GET response
pub async fn send_get_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    counts: SubOperationCounts,
    presentation_context_id: u8,
) -> Result<()> {
    let response = DimseMessageBuilder::new()
        .command_field(command_fields::C_GET_RSP)
        .message_id_being_responded_to(message_id)
        .status(status_code)
        .has_dataset(false)
        .affected_sop_class_uid("1.2.840.10008.5.1.4.1.2.2.3") // Study Root Query/Retrieve - GET
        .sub_operation_counts(&counts)
        .build();

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-STORE response
pub async fn send_store_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    presentation_context_id: u8,
) -> Result<()> {
    // C-STORE-RSP is slightly different - it doesn't include Affected SOP Class UID
    let response = DimseMessageBuilder::new()
        .command_field(command_fields::C_STORE_RSP)
        .message_id_being_responded_to(message_id)
        .status(status_code)
        .has_dataset(false)
        .build();

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{command_fields, status};
    use dicom_dictionary_std::tags;

    #[test]
    fn test_build_command_response_echo() {
        let response = build_command_response(
            command_fields::C_ECHO_RSP,
            42,
            status::SUCCESS,
            false,
            "1.2.840.10008.1.1",
        );

        // Verify command field
        let cmd_field = response
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_ECHO_RSP);

        // Verify message ID being responded to
        let msg_id = response
            .element(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(msg_id, 42);

        // Verify status
        let status_val = response.element(tags::STATUS).unwrap().uint16().unwrap();
        assert_eq!(status_val, status::SUCCESS);

        // Verify no dataset (0x0101)
        let data_set_type = response
            .element(tags::COMMAND_DATA_SET_TYPE)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(data_set_type, 0x0101);
    }

    #[test]
    fn test_build_command_response_find_with_dataset() {
        let response = build_command_response(
            command_fields::C_FIND_RSP,
            1,
            status::PENDING,
            true, // has dataset
            "1.2.840.10008.5.1.4.1.2.2.1",
        );

        // Verify has dataset (0x0000)
        let data_set_type = response
            .element(tags::COMMAND_DATA_SET_TYPE)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(data_set_type, 0x0000);

        // Verify status is PENDING
        let status_val = response.element(tags::STATUS).unwrap().uint16().unwrap();
        assert_eq!(status_val, status::PENDING);
    }

    #[test]
    fn test_build_move_response_with_sub_operations() {
        let counts = SubOperationCounts {
            remaining: 10,
            completed: 5,
            failed: 2,
            warning: 1,
        };

        let response = DimseMessageBuilder::new()
            .command_field(command_fields::C_MOVE_RSP)
            .message_id_being_responded_to(100)
            .status(status::PENDING)
            .has_dataset(false)
            .affected_sop_class_uid("1.2.840.10008.5.1.4.1.2.2.2")
            .sub_operation_counts(&counts)
            .build();

        // Verify sub-operation counts
        let remaining = response
            .element(tags::NUMBER_OF_REMAINING_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(remaining, 10);

        let completed = response
            .element(tags::NUMBER_OF_COMPLETED_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(completed, 5);

        let failed = response
            .element(tags::NUMBER_OF_FAILED_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(failed, 2);

        let warning = response
            .element(tags::NUMBER_OF_WARNING_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(warning, 1);
    }

    #[test]
    fn test_build_get_response_with_sub_operations() {
        let counts = SubOperationCounts {
            remaining: 0,
            completed: 3,
            failed: 0,
            warning: 0,
        };

        let response = DimseMessageBuilder::new()
            .command_field(command_fields::C_GET_RSP)
            .message_id_being_responded_to(50)
            .status(status::SUCCESS)
            .has_dataset(false)
            .affected_sop_class_uid("1.2.840.10008.5.1.4.1.2.2.3")
            .sub_operation_counts(&counts)
            .build();

        // Verify command field is C-GET-RSP
        let cmd_field = response
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_GET_RSP);

        // Verify success status
        let status_val = response.element(tags::STATUS).unwrap().uint16().unwrap();
        assert_eq!(status_val, status::SUCCESS);

        // Verify completed count
        let completed = response
            .element(tags::NUMBER_OF_COMPLETED_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(completed, 3);
    }

    #[test]
    fn test_build_store_response() {
        let response = DimseMessageBuilder::new()
            .command_field(command_fields::C_STORE_RSP)
            .message_id_being_responded_to(77)
            .status(status::SUCCESS)
            .has_dataset(false)
            .build();

        // C-STORE-RSP should not have Affected SOP Class UID
        let cmd_field = response
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_STORE_RSP);

        // Verify message ID
        let msg_id = response
            .element(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(msg_id, 77);
    }

    #[test]
    fn test_sub_operation_counts_default() {
        let counts = SubOperationCounts::default();
        assert_eq!(counts.remaining, 0);
        assert_eq!(counts.completed, 0);
        assert_eq!(counts.failed, 0);
        assert_eq!(counts.warning, 0);
    }

    // PDU chunking tests

    /// Test helper: compute chunk count for given data size and max PDU
    fn compute_chunk_count(data_size: usize, max_pdu: usize) -> usize {
        const PDU_OVERHEAD: usize = 16;
        let max_data_per_pdu = if max_pdu > PDU_OVERHEAD {
            max_pdu - PDU_OVERHEAD
        } else {
            4096
        };
        (data_size + max_data_per_pdu - 1) / max_data_per_pdu // ceiling division
    }

    #[test]
    fn test_pdu_chunking_small_data_fits_in_one_pdu() {
        // Small data (100 bytes) with default PDU size (65536) should fit in one chunk
        let data_size = 100;
        let max_pdu = 65536;
        assert_eq!(compute_chunk_count(data_size, max_pdu), 1);
    }

    #[test]
    fn test_pdu_chunking_data_requires_multiple_chunks() {
        // 100KB data with 16KB PDU should require multiple chunks
        let data_size = 100_000;
        let max_pdu = 16384; // 16KB
        let max_data_per_pdu = max_pdu - 16; // 16368 bytes per chunk
        let expected_chunks = (data_size + max_data_per_pdu - 1) / max_data_per_pdu;
        assert_eq!(compute_chunk_count(data_size, max_pdu), expected_chunks);
        assert!(expected_chunks > 1, "Should require multiple chunks");
    }

    #[test]
    fn test_pdu_chunking_with_orthanc_default_pdu() {
        // Orthanc default max PDU is 16384 bytes
        let data_size = 50_000; // 50KB dataset
        let max_pdu = 16384;
        let chunks = compute_chunk_count(data_size, max_pdu);
        // 16384 - 16 = 16368 bytes per chunk
        // 50000 / 16368 = 3.05, so 4 chunks
        assert_eq!(chunks, 4);
    }

    #[test]
    fn test_pdu_chunking_exact_boundary() {
        // Data size exactly matches available space in one PDU
        let max_pdu = 16384;
        let max_data_per_pdu = max_pdu - 16; // 16368
        let data_size = max_data_per_pdu;
        assert_eq!(compute_chunk_count(data_size, max_pdu), 1);
        
        // One byte over should require 2 chunks
        assert_eq!(compute_chunk_count(data_size + 1, max_pdu), 2);
    }

    #[test]
    fn test_pdu_chunking_very_small_pdu_uses_fallback() {
        // If max_pdu is too small (less than overhead), fallback to 4096
        let data_size = 10_000;
        let max_pdu = 10; // Too small - less than 16 byte overhead
        // Should use fallback of 4096
        let expected_chunks = (data_size + 4095) / 4096; // ceiling division by 4096
        assert_eq!(compute_chunk_count(data_size, max_pdu), expected_chunks);
    }
}

//! Response building and encoding helpers for DIMSE operations

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::pdu::PDataValue;
use dicom_ul::{Pdu, ServerAssociation};
use tokio::net::TcpStream;

use crate::types::DatasetStream;
use crate::{DimseError, Result};

/// Sub-operation counts for C-MOVE and C-GET responses
pub struct SubOperationCounts {
    pub remaining: u16,
    pub completed: u16,
    pub failed: u16,
    pub warning: u16,
}

/// Build a command response object with common fields
pub fn build_command_response(
    command_field: u16,
    message_id: u16,
    status: u16,
    has_dataset: bool,
    sop_class_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    let mut response = InMemDicomObject::new_empty();

    // Command Field (0000,0100)
    response.put(DataElement::new(
        tags::COMMAND_FIELD,
        VR::US,
        PrimitiveValue::from(command_field),
    ));

    // Message ID Being Responded To (0000,0120)
    response.put(DataElement::new(
        tags::MESSAGE_ID_BEING_RESPONDED_TO,
        VR::US,
        PrimitiveValue::from(message_id),
    ));

    // Command Data Set Type (0000,0800)
    response.put(DataElement::new(
        tags::COMMAND_DATA_SET_TYPE,
        VR::US,
        PrimitiveValue::from(if has_dataset { 0x0000u16 } else { 0x0101u16 }),
    ));

    // Status (0000,0900)
    response.put(DataElement::new(
        tags::STATUS,
        VR::US,
        PrimitiveValue::from(status),
    ));

    // Affected SOP Class UID (0000,0002)
    response.put(DataElement::new(
        tags::AFFECTED_SOP_CLASS_UID,
        VR::UI,
        PrimitiveValue::from(sop_class_uid),
    ));

    response
}

/// Encode and send a response with optional dataset
pub async fn encode_and_send_response(
    association: &mut ServerAssociation<TcpStream>,
    response: InMemDicomObject<StandardDataDictionary>,
    dataset: Option<&DatasetStream>,
    presentation_context_id: u8,
) -> Result<()> {
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::operation_failed("Implicit VR Little Endian TS not found"))?;

    let mut response_bytes = Vec::new();
    response
        .write_dataset_with_ts(&mut response_bytes, ts)
        .map_err(|e| {
            DimseError::operation_failed(format!("Failed to encode response: {}", e))
        })?;

    let has_dataset = dataset.is_some();
    let command_pdata = PDataValue {
        presentation_context_id,
        value_type: dicom_ul::pdu::PDataValueType::Command,
        is_last: !has_dataset,
        data: response_bytes,
    };

    // If we have a dataset, send it as well
    if let Some(ds) = dataset {
        // Convert the dataset to a DICOM object
        let dicom_obj = ds.to_object().await?;

        // Encode the identifier dataset
        let mut identifier_bytes = Vec::new();
        dicom_obj
            .write_dataset_with_ts(&mut identifier_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode dataset: {}", e))
            })?;

        let data_pdata = PDataValue {
            presentation_context_id,
            value_type: dicom_ul::pdu::PDataValueType::Data,
            is_last: true,
            data: identifier_bytes,
        };

        association
            .send(&Pdu::PData {
                data: vec![command_pdata, data_pdata],
            })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send response: {}", e)))?;
    } else {
        association
            .send(&Pdu::PData {
                data: vec![command_pdata],
            })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send response: {}", e)))?;
    }

    Ok(())
}

/// Build and send a C-ECHO response
pub async fn send_echo_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    presentation_context_id: u8,
) -> Result<()> {
    let response = build_command_response(
        0x8030, // C-ECHO-RSP
        message_id,
        0x0000, // Success
        false,   // No dataset
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
    status: u16,
    dataset: Option<&DatasetStream>,
    presentation_context_id: u8,
) -> Result<()> {
    let response = build_command_response(
        0x8020, // C-FIND-RSP
        message_id,
        status,
        dataset.is_some(),
        "1.2.840.10008.5.1.4.1.2.2.1", // Study Root Query/Retrieve - FIND
    );

    encode_and_send_response(association, response, dataset, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-MOVE response
pub async fn send_move_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status: u16,
    counts: SubOperationCounts,
    presentation_context_id: u8,
) -> Result<()> {
    let mut response = build_command_response(
        0x8021, // C-MOVE-RSP
        message_id,
        status,
        false, // No dataset
        "1.2.840.10008.5.1.4.1.2.2.2", // Study Root Query/Retrieve - MOVE
    );

    // Add sub-operation status fields
    // Number of Remaining Sub-operations (0000,1020)
    response.put(DataElement::new(
        tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.remaining),
    ));

    // Number of Completed Sub-operations (0000,1021)
    response.put(DataElement::new(
        tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.completed),
    ));

    // Number of Failed Sub-operations (0000,1022)
    response.put(DataElement::new(
        tags::NUMBER_OF_FAILED_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.failed),
    ));

    // Number of Warning Sub-operations (0000,1023)
    response.put(DataElement::new(
        tags::NUMBER_OF_WARNING_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.warning),
    ));

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-GET response
pub async fn send_get_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status: u16,
    counts: SubOperationCounts,
    presentation_context_id: u8,
) -> Result<()> {
    let mut response = build_command_response(
        0x8010, // C-GET-RSP
        message_id,
        status,
        false, // No dataset
        "1.2.840.10008.5.1.4.1.2.2.3", // Study Root Query/Retrieve - GET
    );

    // Add sub-operation status fields
    // Number of Remaining Sub-operations (0000,1020)
    response.put(DataElement::new(
        tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.remaining),
    ));

    // Number of Completed Sub-operations (0000,1021)
    response.put(DataElement::new(
        tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.completed),
    ));

    // Number of Failed Sub-operations (0000,1022)
    response.put(DataElement::new(
        tags::NUMBER_OF_FAILED_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.failed),
    ));

    // Number of Warning Sub-operations (0000,1023)
    response.put(DataElement::new(
        tags::NUMBER_OF_WARNING_SUBOPERATIONS,
        VR::US,
        PrimitiveValue::from(counts.warning),
    ));

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

/// Build and send a C-STORE response
pub async fn send_store_response(
    association: &mut ServerAssociation<TcpStream>,
    message_id: u16,
    status: u16,
    presentation_context_id: u8,
) -> Result<()> {
    let mut response = InMemDicomObject::new_empty();

    // Command Field (0000,0100) = 0x8001 (C-STORE-RSP)
    response.put(DataElement::new(
        tags::COMMAND_FIELD,
        VR::US,
        PrimitiveValue::from(0x8001u16),
    ));

    // Message ID Being Responded To (0000,0120)
    response.put(DataElement::new(
        tags::MESSAGE_ID_BEING_RESPONDED_TO,
        VR::US,
        PrimitiveValue::from(message_id),
    ));

    // Command Data Set Type (0000,0800) = 0x0101 (no dataset)
    response.put(DataElement::new(
        tags::COMMAND_DATA_SET_TYPE,
        VR::US,
        PrimitiveValue::from(0x0101u16),
    ));

    // Status (0000,0900)
    response.put(DataElement::new(
        tags::STATUS,
        VR::US,
        PrimitiveValue::from(status),
    ));

    // Note: C-STORE-RSP does not include Affected SOP Class UID

    encode_and_send_response(association, response, None, presentation_context_id).await?;

    Ok(())
}

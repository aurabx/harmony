//! Common DIMSE message building utilities shared between SCP and SCU
//!
//! This module provides a unified approach to building DIMSE command messages
//! for both requests (SCU → SCP) and responses (SCP → SCU).

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::pdu::{PDataValue, PDataValueType};

use crate::{DimseError, Result};

/// DIMSE command field constants
pub mod command_fields {
    // Request command fields
    pub const C_STORE_RQ: u16 = 0x0001;
    pub const C_GET_RQ: u16 = 0x0010;
    pub const C_FIND_RQ: u16 = 0x0020;
    pub const C_MOVE_RQ: u16 = 0x0021;
    pub const C_ECHO_RQ: u16 = 0x0030;

    // Response command fields (request | 0x8000)
    pub const C_STORE_RSP: u16 = 0x8001;
    pub const C_GET_RSP: u16 = 0x8010;
    pub const C_FIND_RSP: u16 = 0x8020;
    pub const C_MOVE_RSP: u16 = 0x8021;
    pub const C_ECHO_RSP: u16 = 0x8030;
}

/// DIMSE status codes
pub mod status {
    pub const SUCCESS: u16 = 0x0000;
    pub const PENDING: u16 = 0xFF00;
    pub const PENDING_WARNING: u16 = 0xFF01;
    pub const CANCEL: u16 = 0xFE00;
}

/// DIMSE priority values
pub mod priority {
    pub const LOW: u16 = 0x0002;
    pub const MEDIUM: u16 = 0x0000;
    pub const HIGH: u16 = 0x0001;
}

/// Sub-operation counts for C-MOVE and C-GET responses
#[derive(Debug, Clone, Default)]
pub struct SubOperationCounts {
    pub remaining: u16,
    pub completed: u16,
    pub failed: u16,
    pub warning: u16,
}

/// Builder for DIMSE command messages (both requests and responses)
#[derive(Debug, Clone)]
pub struct DimseMessageBuilder {
    obj: InMemDicomObject<StandardDataDictionary>,
}

impl DimseMessageBuilder {
    /// Create a new empty DIMSE message builder
    pub fn new() -> Self {
        Self {
            obj: InMemDicomObject::new_empty(),
        }
    }

    /// Set the Command Field (0000,0100)
    pub fn command_field(mut self, field: u16) -> Self {
        self.obj.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(field),
        ));
        self
    }

    /// Set the Command Data Set Type (0000,0800)
    /// - `true` means a dataset follows (0x0000)
    /// - `false` means no dataset (0x0101)
    pub fn has_dataset(mut self, has_dataset: bool) -> Self {
        self.obj.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(if has_dataset { 0x0000u16 } else { 0x0101u16 }),
        ));
        self
    }

    /// Set the Affected SOP Class UID (0000,0002)
    pub fn affected_sop_class_uid(mut self, uid: &str) -> Self {
        self.obj.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from(uid),
        ));
        self
    }

    /// Set the Message ID (0000,0110) - for requests
    pub fn message_id(mut self, id: u16) -> Self {
        self.obj.put(DataElement::new(
            tags::MESSAGE_ID,
            VR::US,
            PrimitiveValue::from(id),
        ));
        self
    }

    /// Set the Message ID Being Responded To (0000,0120) - for responses
    pub fn message_id_being_responded_to(mut self, id: u16) -> Self {
        self.obj.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(id),
        ));
        self
    }

    /// Set the Priority (0000,0700) - for requests
    pub fn priority(mut self, priority: u16) -> Self {
        self.obj.put(DataElement::new(
            tags::PRIORITY,
            VR::US,
            PrimitiveValue::from(priority),
        ));
        self
    }

    /// Set the Status (0000,0900) - for responses
    pub fn status(mut self, status: u16) -> Self {
        self.obj.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));
        self
    }

    /// Set the Move Destination (0000,0600) - for C-MOVE requests
    pub fn move_destination(mut self, aet: &str) -> Self {
        self.obj.put(DataElement::new(
            tags::MOVE_DESTINATION,
            VR::AE,
            PrimitiveValue::from(aet),
        ));
        self
    }

    /// Add sub-operation counts (for C-MOVE and C-GET responses)
    pub fn sub_operation_counts(mut self, counts: &SubOperationCounts) -> Self {
        self.obj.put(DataElement::new(
            tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.remaining),
        ));
        self.obj.put(DataElement::new(
            tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.completed),
        ));
        self.obj.put(DataElement::new(
            tags::NUMBER_OF_FAILED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.failed),
        ));
        self.obj.put(DataElement::new(
            tags::NUMBER_OF_WARNING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.warning),
        ));
        self
    }

    /// Build the final DICOM object
    pub fn build(self) -> InMemDicomObject<StandardDataDictionary> {
        self.obj
    }
}

impl Default for DimseMessageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to build a DIMSE request command
pub fn build_request(
    command_field: u16,
    message_id: u16,
    has_dataset: bool,
    sop_class_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    DimseMessageBuilder::new()
        .command_field(command_field)
        .message_id(message_id)
        .has_dataset(has_dataset)
        .affected_sop_class_uid(sop_class_uid)
        .priority(priority::MEDIUM)
        .build()
}

/// Convenience function to build a DIMSE response command
pub fn build_response(
    command_field: u16,
    message_id: u16,
    status: u16,
    has_dataset: bool,
    sop_class_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    DimseMessageBuilder::new()
        .command_field(command_field)
        .message_id_being_responded_to(message_id)
        .has_dataset(has_dataset)
        .status(status)
        .affected_sop_class_uid(sop_class_uid)
        .build()
}

/// Encode a DIMSE command object to bytes using Implicit VR Little Endian
pub fn encode_command(
    command: &InMemDicomObject<StandardDataDictionary>,
) -> Result<Vec<u8>> {
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::operation_failed("Implicit VR Little Endian TS not found"))?;

    let mut bytes = Vec::new();
    command
        .write_dataset_with_ts(&mut bytes, ts)
        .map_err(|e| DimseError::operation_failed(format!("Failed to encode command: {}", e)))?;

    Ok(bytes)
}

/// Encode a dataset object to bytes using Implicit VR Little Endian
pub fn encode_dataset(
    dataset: &InMemDicomObject<StandardDataDictionary>,
) -> Result<Vec<u8>> {
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::operation_failed("Implicit VR Little Endian TS not found"))?;

    let mut bytes = Vec::new();
    dataset
        .write_dataset_with_ts(&mut bytes, ts)
        .map_err(|e| DimseError::operation_failed(format!("Failed to encode dataset: {}", e)))?;

    Ok(bytes)
}

/// Create a command PDataValue
pub fn create_command_pdata(
    presentation_context_id: u8,
    command_bytes: Vec<u8>,
    is_last: bool,
) -> PDataValue {
    PDataValue {
        presentation_context_id,
        value_type: PDataValueType::Command,
        is_last,
        data: command_bytes,
    }
}

/// Create a data PDataValue
pub fn create_data_pdata(
    presentation_context_id: u8,
    data_bytes: Vec<u8>,
    is_last: bool,
) -> PDataValue {
    PDataValue {
        presentation_context_id,
        value_type: PDataValueType::Data,
        is_last,
        data: data_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_request() {
        let request = build_request(
            command_fields::C_ECHO_RQ,
            1,
            false,
            "1.2.840.10008.1.1",
        );

        let cmd_field = request
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_ECHO_RQ);

        let msg_id = request
            .element(tags::MESSAGE_ID)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(msg_id, 1);
    }

    #[test]
    fn test_build_response() {
        let response = build_response(
            command_fields::C_ECHO_RSP,
            1,
            status::SUCCESS,
            false,
            "1.2.840.10008.1.1",
        );

        let cmd_field = response
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_ECHO_RSP);

        let msg_id = response
            .element(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(msg_id, 1);

        let status_val = response
            .element(tags::STATUS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(status_val, status::SUCCESS);
    }

    #[test]
    fn test_builder_with_sub_operations() {
        let counts = SubOperationCounts {
            remaining: 5,
            completed: 3,
            failed: 1,
            warning: 0,
        };

        let response = DimseMessageBuilder::new()
            .command_field(command_fields::C_MOVE_RSP)
            .message_id_being_responded_to(1)
            .status(status::PENDING)
            .has_dataset(false)
            .affected_sop_class_uid("1.2.840.10008.5.1.4.1.2.2.2")
            .sub_operation_counts(&counts)
            .build();

        let remaining = response
            .element(tags::NUMBER_OF_REMAINING_SUBOPERATIONS)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(remaining, 5);
    }

    #[test]
    fn test_encode_command() {
        let request = build_request(
            command_fields::C_ECHO_RQ,
            1,
            false,
            "1.2.840.10008.1.1",
        );

        let bytes = encode_command(&request).unwrap();
        assert!(!bytes.is_empty());
    }
}

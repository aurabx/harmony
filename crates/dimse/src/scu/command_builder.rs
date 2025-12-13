//! Command building helpers for SCU operations

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use dicom_ul::{ClientAssociation, Pdu};
use tokio::net::TcpStream;

use crate::common::{
    build_request, build_response, create_command_pdata, create_data_pdata, encode_command,
    encode_dataset, query_utils,
};
use crate::types::QueryLevel;
use crate::{DimseError, Result};

// Re-export for backwards compatibility
pub use crate::common::{command_fields, priority, status};

/// Build a command request object with common fields
/// 
/// This is a convenience wrapper around the common build_request function.
pub fn build_command_request(
    command_field: u16,
    message_id: u16,
    has_dataset: bool,
    sop_class_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    build_request(command_field, message_id, has_dataset, sop_class_uid)
}

/// Encode and send a command request with optional dataset
pub async fn encode_and_send_request(
    association: &mut ClientAssociation<TcpStream>,
    request: InMemDicomObject<StandardDataDictionary>,
    dataset: Option<&InMemDicomObject<StandardDataDictionary>>,
    presentation_context_id: u8,
) -> Result<()> {
    let command_bytes = encode_command(&request)?;

    let command_pdata = create_command_pdata(presentation_context_id, command_bytes, true);

    // Send command PDU
    association
        .send(&Pdu::PData {
            data: vec![command_pdata],
        })
        .await
        .map_err(|e| DimseError::network(format!("Failed to send command request: {}", e)))?;

    // If we have a dataset, send it as well
    if let Some(ds) = dataset {
        let dataset_bytes = encode_dataset(ds)?;
        let data_pdata = create_data_pdata(presentation_context_id, dataset_bytes, true);

        association
            .send(&Pdu::PData {
                data: vec![data_pdata],
            })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send dataset: {}", e)))?;
    }

    Ok(())
}

/// Receive a full DIMSE message (Command + optional Dataset) handling split PDUs
pub async fn receive_dimse_message(
    association: &mut ClientAssociation<TcpStream>,
) -> Result<(InMemDicomObject<StandardDataDictionary>, Option<Vec<u8>>, u8)> {
    use dicom_encoding::text::SpecificCharacterSet;
    use dicom_object::InMemDicomObject;

    // 1. Read Command PDU
    let mut command_data = Vec::new();
    let mut dataset_data = Vec::new();
    let mut presentation_context_id = 0u8;
    let mut expect_more_command = true;
    let mut data_complete = false;

    // Loop to read Command PDVs
    while expect_more_command {
        let pdu = association.receive().await.map_err(|e| {
            DimseError::network(format!("Failed to receive PDU: {}", e))
        })?;

        match pdu {
            Pdu::PData { data } => {
                for pdata_value in data {
                    if pdata_value.value_type == dicom_ul::pdu::PDataValueType::Command {
                        presentation_context_id = pdata_value.presentation_context_id;
                        command_data.extend_from_slice(&pdata_value.data);
                        if pdata_value.is_last {
                            expect_more_command = false;
                        }
                    } else {
                        // Data PDV
                        if !expect_more_command {
                            // This is Data PDV following Command PDV in same PDU
                            dataset_data.extend_from_slice(&pdata_value.data);
                            if pdata_value.is_last {
                                data_complete = true;
                            }
                        } else {
                            return Err(DimseError::parse("Received Data PDV while waiting for Command PDV"));
                        }
                    }
                }
            }
            Pdu::ReleaseRQ => return Err(DimseError::operation_failed("Association released unexpectedly")),
            Pdu::AbortRQ { .. } => return Err(DimseError::operation_failed("Association aborted")),
            _ => return Err(DimseError::parse("Unexpected PDU type")),
        }
    }

    if command_data.is_empty() {
        return Err(DimseError::parse("Received PDU with no command data"));
    }

    // Parse the command dataset using Implicit VR Little Endian
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

    let cursor = std::io::Cursor::new(&command_data);
    let command_obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| {
        DimseError::parse(format!(
            "Failed to parse response command dataset: {}",
            e
        ))
    })?;

    // Check if Dataset is expected
    let data_set_type = command_obj
        .element(tags::COMMAND_DATA_SET_TYPE)
        .ok()
        .and_then(|e| e.uint16().ok())
        .unwrap_or(0x0101); // Default to no dataset

    if data_set_type != 0x0101 {
        // Dataset expected
        // Check if we already received the full dataset
        if !data_complete {
            // Need to read more Data PDVs
            let mut expect_more_data = true;
            while expect_more_data {
                let pdu = association.receive().await.map_err(|e| {
                    DimseError::network(format!("Failed to receive PDU for dataset: {}", e))
                })?;

                match pdu {
                    Pdu::PData { data } => {
                        for pdata_value in data {
                            if pdata_value.value_type == dicom_ul::pdu::PDataValueType::Data {
                                dataset_data.extend_from_slice(&pdata_value.data);
                                if pdata_value.is_last {
                                    expect_more_data = false;
                                }
                            } else {
                                return Err(DimseError::parse("Received Command PDV while waiting for Data PDV"));
                            }
                        }
                    }
                    Pdu::ReleaseRQ => return Err(DimseError::operation_failed("Association released unexpectedly")),
                    Pdu::AbortRQ { .. } => return Err(DimseError::operation_failed("Association aborted")),
                    _ => return Err(DimseError::parse("Unexpected PDU type")),
                }
            }
        }
        
        Ok((command_obj, Some(dataset_data), presentation_context_id))
    } else {
        // No dataset expected
        Ok((command_obj, None, presentation_context_id))
    }
}

/// Parse a response command from received PDUs
pub fn parse_response_command(
    pdata: Vec<dicom_ul::pdu::PDataValue>,
) -> Result<(InMemDicomObject<StandardDataDictionary>, Option<Vec<u8>>, u8)> {
    use dicom_encoding::text::SpecificCharacterSet;
    use dicom_object::InMemDicomObject;

    let mut command_data = Vec::new();
    let mut dataset_data = Vec::new();
    let mut presentation_context_id = 1u8;

    // Separate command and data PDUs
    for pdata_value in pdata {
        presentation_context_id = pdata_value.presentation_context_id;
        match pdata_value.value_type {
            dicom_ul::pdu::PDataValueType::Command => {
                command_data.extend_from_slice(&pdata_value.data);
            }
            dicom_ul::pdu::PDataValueType::Data => {
                dataset_data.extend_from_slice(&pdata_value.data);
            }
        }
    }

    if command_data.is_empty() {
        return Err(DimseError::parse("Received PDU with no command data"));
    }

    // Parse the command dataset using Implicit VR Little Endian
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

    let cursor = std::io::Cursor::new(&command_data);
    let command_obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| {
        DimseError::parse(format!(
            "Failed to parse response command dataset: {}",
            e
        ))
    })?;

    let dataset = if dataset_data.is_empty() {
        None
    } else {
        Some(dataset_data)
    };

    Ok((command_obj, dataset, presentation_context_id))
}

/// Extract status code from a response command object
pub fn extract_status(
    response: &InMemDicomObject<StandardDataDictionary>,
) -> Result<u16> {
    response
        .element(tags::STATUS)
        .map_err(|_| DimseError::parse("Missing status field in response"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid status field: {}", e)))
}

/// Extract message ID being responded to from a response command object
pub fn extract_message_id_being_responded_to(
    response: &InMemDicomObject<StandardDataDictionary>,
) -> Result<u16> {
    response
        .element(tags::MESSAGE_ID_BEING_RESPONDED_TO)
        .map_err(|_| DimseError::parse("Missing message ID being responded to field"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid message ID being responded to: {}", e)))
}

/// Parse dataset bytes from a response into a DICOM object
pub fn parse_dataset_bytes(
    dataset_bytes: Vec<u8>,
    presentation_context_id: u8,
    association: &ClientAssociation<TcpStream>,
) -> Result<InMemDicomObject<StandardDataDictionary>> {
    use dicom_encoding::text::SpecificCharacterSet;
    
    // Get transfer syntax from presentation context
    let ts_uid = association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == presentation_context_id)
        .map(|pc| &pc.transfer_syntax)
        .ok_or_else(|| DimseError::parse("Presentation context not found"))?;
    
    let ts = TransferSyntaxRegistry
        .get(ts_uid)
        .ok_or_else(|| DimseError::parse(format!("Transfer syntax not found: {}", ts_uid)))?;
    
    let cursor = std::io::Cursor::new(&dataset_bytes);
    InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        cursor,
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| DimseError::parse(format!("Failed to parse dataset: {}", e)))
}

/// Build an identifier dataset from query parameters
pub fn build_identifier_dataset(
    query_level: QueryLevel,
    parameters: &std::collections::HashMap<String, String>,
) -> Result<InMemDicomObject<StandardDataDictionary>> {
    let mut identifier = InMemDicomObject::new_empty();
    
    // Add QueryRetrieveLevel (0008,0052)
    let level_str = query_utils::query_level_to_string(query_level);
    identifier.put(DataElement::new(
        Tag(0x0008, 0x0052),
        VR::CS,
        PrimitiveValue::from(level_str),
    ));
    
    // Add query parameters
    // Map common tag strings to proper tags and VRs
    for (tag_str, value) in parameters {
        let (tag, vr) = match tag_str.as_str() {
            "00100010" | "PatientName" => (tags::PATIENT_NAME, VR::PN),
            "00100020" | "PatientID" => (tags::PATIENT_ID, VR::LO),
            "0020000D" | "StudyInstanceUID" => (tags::STUDY_INSTANCE_UID, VR::UI),
            "0020000E" | "SeriesInstanceUID" => (tags::SERIES_INSTANCE_UID, VR::UI),
            "00080018" | "SOPInstanceUID" => (tags::SOP_INSTANCE_UID, VR::UI),
            "00080020" | "StudyDate" => (tags::STUDY_DATE, VR::DA),
            "00080030" | "StudyTime" => (tags::STUDY_TIME, VR::TM),
            "00080060" | "Modality" => (tags::MODALITY, VR::CS),
            "00080050" | "AccessionNumber" => (tags::ACCESSION_NUMBER, VR::SH),
            _ => {
                // Try to parse as hex tag (format: "GGGGEEEE" or "GGGG,EEEE")
                let normalized = query_utils::normalize_tag(tag_str);
                let parts: Vec<&str> = normalized.split(',').collect();
                if parts.len() == 2 {
                    if let (Ok(group), Ok(element)) = (
                        u16::from_str_radix(parts[0], 16),
                        u16::from_str_radix(parts[1], 16),
                    ) {
                        let tag = Tag(group, element);
                        // Default to UN (Universal) VR for unknown tags
                        (tag, VR::UN)
                    } else {
                        // Skip invalid tags
                        continue;
                    }
                } else {
                    // Skip unknown tag formats
                    continue;
                }
            }
        };
        
        // Add the element - empty value means return key
        if value.is_empty() {
            // Return key - add with empty value
            identifier.put(DataElement::new(tag, vr, PrimitiveValue::Empty));
        } else {
            // Add with value
            identifier.put(DataElement::new(tag, vr, PrimitiveValue::from(value.as_str())));
        }
    }
    
    Ok(identifier)
}

/// Send C-STORE-RSP response from client (when receiving C-STORE-RQ during C-GET)
pub async fn send_store_response_from_client(
    association: &mut ClientAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    presentation_context_id: u8,
) -> Result<()> {
    use crate::common::command_fields;

    // Use the common build_response function
    let response = build_response(
        command_fields::C_STORE_RSP,
        message_id,
        status_code,
        false, // no dataset
        "",    // C-STORE-RSP doesn't need SOP Class UID
    );

    let response_bytes = encode_command(&response)?;
    let command_pdata = create_command_pdata(presentation_context_id, response_bytes, true);

    association
        .send(&Pdu::PData {
            data: vec![command_pdata],
        })
        .await
        .map_err(|e| DimseError::network(format!("Failed to send C-STORE-RSP: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{command_fields, status};
    use dicom_ul::pdu::{PDataValue, PDataValueType};

    #[test]
    fn test_build_command_request() {
        let request = build_command_request(
            command_fields::C_ECHO_RQ,
            1,
            false,
            "1.2.840.10008.1.1",
        );

        // Verify command field
        let cmd_field = request
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_ECHO_RQ);

        // Verify message ID
        let msg_id = request.element(tags::MESSAGE_ID).unwrap().uint16().unwrap();
        assert_eq!(msg_id, 1);

        // Verify priority (default MEDIUM = 0)
        let priority = request.element(tags::PRIORITY).unwrap().uint16().unwrap();
        assert_eq!(priority, 0);
    }

    #[test]
    fn test_build_command_request_with_dataset() {
        let request = build_command_request(
            command_fields::C_FIND_RQ,
            42,
            true, // has dataset
            "1.2.840.10008.5.1.4.1.2.2.1",
        );

        // Verify has dataset (0x0000)
        let data_set_type = request
            .element(tags::COMMAND_DATA_SET_TYPE)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(data_set_type, 0x0000);
    }

    #[test]
    fn test_parse_response_command_only() {
        // Build a valid C-ECHO-RSP command
        let response = build_response(
            command_fields::C_ECHO_RSP,
            1,
            status::SUCCESS,
            false,
            "1.2.840.10008.1.1",
        );

        // Encode it
        let command_bytes = encode_command(&response).unwrap();

        // Create PDataValue
        let pdata = vec![PDataValue {
            presentation_context_id: 1,
            value_type: PDataValueType::Command,
            is_last: true,
            data: command_bytes,
        }];

        // Parse it
        let (parsed, dataset, pc_id) = parse_response_command(pdata).unwrap();

        // Verify parsed correctly
        assert!(dataset.is_none());
        assert_eq!(pc_id, 1);

        let cmd_field = parsed
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, command_fields::C_ECHO_RSP);
    }

    #[test]
    fn test_parse_response_command_with_dataset() {
        // Build a C-FIND-RSP with dataset
        let response = build_response(
            command_fields::C_FIND_RSP,
            5,
            status::PENDING,
            true,
            "1.2.840.10008.5.1.4.1.2.2.1",
        );

        let command_bytes = encode_command(&response).unwrap();

        // Create mock dataset bytes (just some arbitrary data for testing)
        let dataset_bytes = vec![0x08, 0x00, 0x52, 0x00]; // Start of QueryRetrieveLevel tag

        let pdata = vec![
            PDataValue {
                presentation_context_id: 3,
                value_type: PDataValueType::Command,
                is_last: true,
                data: command_bytes,
            },
            PDataValue {
                presentation_context_id: 3,
                value_type: PDataValueType::Data,
                is_last: true,
                data: dataset_bytes.clone(),
            },
        ];

        let (parsed, dataset, pc_id) = parse_response_command(pdata).unwrap();

        assert_eq!(pc_id, 3);
        assert!(dataset.is_some());
        assert_eq!(dataset.unwrap(), dataset_bytes);

        let status_val = parsed.element(tags::STATUS).unwrap().uint16().unwrap();
        assert_eq!(status_val, status::PENDING);
    }

    #[test]
    fn test_parse_response_command_empty_fails() {
        let pdata: Vec<PDataValue> = vec![];
        let result = parse_response_command(pdata);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response_command_data_only_fails() {
        // Only data PDV, no command
        let pdata = vec![PDataValue {
            presentation_context_id: 1,
            value_type: PDataValueType::Data,
            is_last: true,
            data: vec![0x01, 0x02, 0x03],
        }];

        let result = parse_response_command(pdata);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_status() {
        let response = build_response(
            command_fields::C_ECHO_RSP,
            1,
            status::SUCCESS,
            false,
            "1.2.840.10008.1.1",
        );

        let extracted = extract_status(&response).unwrap();
        assert_eq!(extracted, status::SUCCESS);
    }

    #[test]
    fn test_extract_status_pending() {
        let response = build_response(
            command_fields::C_FIND_RSP,
            1,
            status::PENDING,
            true,
            "1.2.840.10008.5.1.4.1.2.2.1",
        );

        let extracted = extract_status(&response).unwrap();
        assert_eq!(extracted, status::PENDING);
    }

    #[test]
    fn test_extract_message_id_being_responded_to() {
        let response = build_response(
            command_fields::C_ECHO_RSP,
            999,
            status::SUCCESS,
            false,
            "1.2.840.10008.1.1",
        );

        let extracted = extract_message_id_being_responded_to(&response).unwrap();
        assert_eq!(extracted, 999);
    }

    #[test]
    fn test_build_identifier_dataset() {
        use std::collections::HashMap;

        let mut params = HashMap::new();
        params.insert("PatientID".to_string(), "12345".to_string());
        params.insert("PatientName".to_string(), "DOE^JOHN".to_string());
        params.insert("StudyDate".to_string(), "".to_string()); // Return key

        let identifier = build_identifier_dataset(QueryLevel::Patient, &params).unwrap();

        // Verify QueryRetrieveLevel is set
        let qr_level = identifier
            .element(Tag(0x0008, 0x0052))
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(qr_level.trim(), "PATIENT");

        // Verify PatientID
        let patient_id = identifier
            .element(tags::PATIENT_ID)
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(patient_id, "12345");

        // Verify PatientName
        let patient_name = identifier
            .element(tags::PATIENT_NAME)
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(patient_name, "DOE^JOHN");

        // Verify StudyDate is present (even if empty - return key)
        assert!(identifier.element(tags::STUDY_DATE).is_ok());
    }

    #[test]
    fn test_build_identifier_dataset_with_hex_tags() {
        use std::collections::HashMap;

        let mut params = HashMap::new();
        params.insert("00100020".to_string(), "HEX_ID".to_string());
        params.insert("0020000D".to_string(), "1.2.3.4.5".to_string());

        let identifier = build_identifier_dataset(QueryLevel::Study, &params).unwrap();

        // Verify PatientID via hex tag
        let patient_id = identifier
            .element(tags::PATIENT_ID)
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(patient_id, "HEX_ID");

        // Verify StudyInstanceUID via hex tag
        let study_uid = identifier
            .element(tags::STUDY_INSTANCE_UID)
            .unwrap()
            .string()
            .unwrap();
        assert_eq!(study_uid.trim(), "1.2.3.4.5");
    }
}

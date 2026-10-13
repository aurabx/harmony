//! Reassembly of a DIMSE message from the P-DATA fragments it arrives in.
//!
//! A DIMSE message is a command set followed, for some commands, by a data
//! set. A peer may split either part across any number of PDVs, and those
//! PDVs across any number of P-DATA PDUs. The last PDV of each part carries
//! the last-fragment flag (PS3.8, Annex E). A part is complete only when
//! that flag arrives.

use dicom_dictionary_std::{tags, uids};
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::pdu::{PDataValue, PDataValueType};

use crate::{DimseError, Result};

/// The value of CommandDataSetType (0000,0800) that means no data set follows.
const NO_DATA_SET: u16 = 0x0101;

/// Largest command set the assembler buffers. A command set holds a handful
/// of short elements and is a few hundred bytes, so this is generous. It is
/// fixed, and separate from the data set limit, so that neither a peer nor a
/// low data set limit can affect it.
const MAX_COMMAND_SET_BYTES: u64 = 64 * 1024;

/// A complete DIMSE message.
#[derive(Debug)]
pub struct DimseMessage {
    /// The parsed command set.
    pub command: InMemDicomObject<StandardDataDictionary>,
    /// The encoded data set, in the transfer syntax of the presentation
    /// context. Empty when the command declares that no data set follows.
    pub data: Vec<u8>,
    /// The presentation context the message arrived on.
    pub presentation_context_id: u8,
}

/// Which part of a message the next PDV must belong to.
enum Progress {
    /// Collecting command fragments.
    Command { bytes: Vec<u8> },
    /// The command is complete and declares a data set. Collecting its
    /// fragments.
    Data {
        command: InMemDicomObject<StandardDataDictionary>,
        presentation_context_id: u8,
        bytes: Vec<u8>,
    },
}

/// Collects the PDVs of one association into complete DIMSE messages.
///
/// One assembler serves one association for its whole life. It holds the
/// fragments of at most one message at a time.
pub struct MessageAssembler {
    progress: Progress,
    max_data_set_bytes: u64,
    commands_with_data_sets: Vec<u16>,
}

impl MessageAssembler {
    /// `max_data_set_bytes` is the largest data set the assembler will
    /// buffer. DICOM sets no limit, so without it a peer could send fragments
    /// without end.
    ///
    /// `commands_with_data_sets` lists the command fields whose data set the
    /// assembler will buffer. A data set declared by any other command is
    /// refused when the command arrives, before any of it is buffered, so a
    /// peer cannot fill memory through an operation that is switched off.
    pub fn new(max_data_set_bytes: u64, commands_with_data_sets: Vec<u16>) -> Self {
        Self {
            progress: Progress::Command { bytes: Vec::new() },
            max_data_set_bytes,
            commands_with_data_sets,
        }
    }

    /// Fail when the command declares a data set that will not be buffered.
    fn check_data_set_accepted(
        &self,
        command: &InMemDicomObject<StandardDataDictionary>,
    ) -> Result<()> {
        let command_field = command
            .element(tags::COMMAND_FIELD)
            .map_err(|_| DimseError::parse("Missing command field"))?
            .uint16()
            .map_err(|e| DimseError::parse(format!("Invalid command field: {}", e)))?;
        if !self.commands_with_data_sets.contains(&command_field) {
            return Err(DimseError::operation_failed(format!(
                "Command 0x{:04X} declares a data set, but the operation is not enabled or not supported",
                command_field
            )));
        }
        Ok(())
    }

    /// Add the next PDV received on the association.
    ///
    /// Returns the message when this PDV completes one, and `None` while
    /// more fragments are needed. An error means the peer sent PDVs in an
    /// order the protocol does not allow. The assembler must not be used
    /// again after an error; the caller is expected to abort the association.
    pub fn push(&mut self, pdv: PDataValue) -> Result<Option<DimseMessage>> {
        let progress =
            std::mem::replace(&mut self.progress, Progress::Command { bytes: Vec::new() });
        match (progress, pdv.value_type) {
            (Progress::Command { mut bytes }, PDataValueType::Command) => {
                check_size(
                    "command set",
                    bytes.len(),
                    pdv.data.len(),
                    MAX_COMMAND_SET_BYTES,
                )?;
                bytes.extend_from_slice(&pdv.data);
                if !pdv.is_last {
                    self.progress = Progress::Command { bytes };
                    return Ok(None);
                }
                let command = parse_command(&bytes)?;
                if !declares_data_set(&command) {
                    return Ok(Some(DimseMessage {
                        command,
                        data: Vec::new(),
                        presentation_context_id: pdv.presentation_context_id,
                    }));
                }
                self.check_data_set_accepted(&command)?;
                self.progress = Progress::Data {
                    command,
                    presentation_context_id: pdv.presentation_context_id,
                    bytes: Vec::new(),
                };
                Ok(None)
            }
            (Progress::Command { .. }, PDataValueType::Data) => Err(DimseError::parse(format!(
                "Received a data fragment of {} bytes on presentation context {} with no command that declares a data set",
                pdv.data.len(),
                pdv.presentation_context_id
            ))),
            (
                Progress::Data {
                    command,
                    presentation_context_id,
                    mut bytes,
                },
                PDataValueType::Data,
            ) => {
                if pdv.presentation_context_id != presentation_context_id {
                    return Err(DimseError::parse(format!(
                        "Received a data fragment on presentation context {} for a command sent on presentation context {}",
                        pdv.presentation_context_id, presentation_context_id
                    )));
                }
                check_size(
                    "data set",
                    bytes.len(),
                    pdv.data.len(),
                    self.max_data_set_bytes,
                )?;
                bytes.extend_from_slice(&pdv.data);
                if !pdv.is_last {
                    self.progress = Progress::Data {
                        command,
                        presentation_context_id,
                        bytes,
                    };
                    return Ok(None);
                }
                Ok(Some(DimseMessage {
                    command,
                    data: bytes,
                    presentation_context_id,
                }))
            }
            (Progress::Data { bytes, .. }, PDataValueType::Command) => Err(DimseError::parse(format!(
                "Received a command fragment while the data set of the previous command was incomplete ({} bytes received)",
                bytes.len()
            ))),
        }
    }
}

/// Fail when adding `incoming` bytes to a part of `buffered` bytes would pass
/// `limit`.
fn check_size(part: &str, buffered: usize, incoming: usize, limit: u64) -> Result<()> {
    let total = (buffered as u64).saturating_add(incoming as u64);
    if total > limit {
        return Err(DimseError::parse(format!(
            "The {} of a DIMSE message reached {} bytes, over the limit of {} bytes",
            part, total, limit
        )));
    }
    Ok(())
}

/// Parse a command set. Command sets are always Implicit VR Little Endian.
fn parse_command(bytes: &[u8]) -> Result<InMemDicomObject<StandardDataDictionary>> {
    let ts = TransferSyntaxRegistry
        .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;
    InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
        std::io::Cursor::new(bytes),
        ts,
        SpecificCharacterSet::default(),
    )
    .map_err(|e| {
        DimseError::parse(format!(
            "Failed to parse command dataset ({} bytes): {}",
            bytes.len(),
            e
        ))
    })
}

/// Whether the command says a data set follows it.
///
/// A command without CommandDataSetType is treated as having no data set.
fn declares_data_set(command: &InMemDicomObject<StandardDataDictionary>) -> bool {
    command
        .element(tags::COMMAND_DATA_SET_TYPE)
        .ok()
        .and_then(|e| e.uint16().ok())
        .map(|value| value != NO_DATA_SET)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_core::{DataElement, PrimitiveValue, VR};

    const C_STORE_RQ: u16 = 0x0001;
    const C_ECHO_RQ: u16 = 0x0030;
    const DATA_SET_PRESENT: u16 = 0x0000;
    const C_FIND_RQ: u16 = 0x0020;
    const NO_LIMIT: u64 = u64::MAX;

    fn assembler_with_limit(max_data_set_bytes: u64) -> MessageAssembler {
        MessageAssembler::new(max_data_set_bytes, vec![C_STORE_RQ])
    }

    fn command_bytes(command_field: u16, data_set_type: u16) -> Vec<u8> {
        let mut command = InMemDicomObject::new_empty();
        for (tag, value) in [
            (tags::COMMAND_FIELD, command_field),
            (tags::MESSAGE_ID, 7),
            (tags::COMMAND_DATA_SET_TYPE, data_set_type),
        ] {
            command.put(DataElement::new(tag, VR::US, PrimitiveValue::from(value)));
        }
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .expect("Implicit VR Little Endian");
        let mut bytes = Vec::new();
        command
            .write_dataset_with_ts(&mut bytes, ts)
            .expect("encode command");
        bytes
    }

    fn pdv(value_type: PDataValueType, data: &[u8], is_last: bool) -> PDataValue {
        pdv_on(1, value_type, data, is_last)
    }

    fn pdv_on(
        presentation_context_id: u8,
        value_type: PDataValueType,
        data: &[u8],
        is_last: bool,
    ) -> PDataValue {
        PDataValue {
            presentation_context_id,
            value_type,
            is_last,
            data: data.to_vec(),
        }
    }

    fn command_field(message: &DimseMessage) -> u16 {
        message
            .command
            .element(tags::COMMAND_FIELD)
            .expect("command field")
            .uint16()
            .expect("command field value")
    }

    #[test]
    fn a_command_without_a_data_set_completes_at_once() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let message = assembler
            .push(pdv_on(
                3,
                PDataValueType::Command,
                &command_bytes(C_ECHO_RQ, NO_DATA_SET),
                true,
            ))
            .expect("valid PDV")
            .expect("the message is complete");
        assert_eq!(command_field(&message), C_ECHO_RQ);
        assert!(message.data.is_empty());
        assert_eq!(message.presentation_context_id, 3);
    }

    #[test]
    fn a_data_set_in_one_fragment_completes_the_message() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assert!(assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV")
            .is_none());
        let message = assembler
            .push(pdv(PDataValueType::Data, &[1, 2, 3, 4], true))
            .expect("valid PDV")
            .expect("the message is complete");
        assert_eq!(command_field(&message), C_STORE_RQ);
        assert_eq!(message.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_data_set_is_not_complete_until_the_last_fragment() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV");
        for fragment in [[1u8, 2], [3, 4], [5, 6]] {
            assert!(
                assembler
                    .push(pdv(PDataValueType::Data, &fragment, false))
                    .expect("valid PDV")
                    .is_none(),
                "a fragment without the last-fragment flag must not complete the message"
            );
        }
        let message = assembler
            .push(pdv(PDataValueType::Data, &[7, 8], true))
            .expect("valid PDV")
            .expect("the message is complete");
        assert_eq!(message.data, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn a_command_split_across_fragments_is_joined() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_ECHO_RQ, NO_DATA_SET);
        let (first, second) = command.split_at(command.len() / 2);
        assert!(assembler
            .push(pdv(PDataValueType::Command, first, false))
            .expect("valid PDV")
            .is_none());
        let message = assembler
            .push(pdv(PDataValueType::Command, second, true))
            .expect("valid PDV")
            .expect("the message is complete");
        assert_eq!(command_field(&message), C_ECHO_RQ);
    }

    #[test]
    fn an_empty_last_data_fragment_completes_the_message() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV");
        assembler
            .push(pdv(PDataValueType::Data, &[1, 2], false))
            .expect("valid PDV");
        let message = assembler
            .push(pdv(PDataValueType::Data, &[], true))
            .expect("valid PDV")
            .expect("the message is complete");
        assert_eq!(message.data, vec![1, 2]);
    }

    #[test]
    fn the_assembler_takes_a_second_message_after_the_first() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        for payload in [[1u8, 2], [3, 4]] {
            assembler
                .push(pdv(PDataValueType::Command, &command, true))
                .expect("valid PDV");
            let message = assembler
                .push(pdv(PDataValueType::Data, &payload, true))
                .expect("valid PDV")
                .expect("the message is complete");
            assert_eq!(message.data, payload.to_vec());
        }
    }

    #[test]
    fn a_data_fragment_with_no_command_is_rejected() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let error = assembler
            .push(pdv(PDataValueType::Data, &[1, 2], true))
            .expect_err("no command was sent");
        assert!(error.to_string().contains("no command"), "{}", error);
    }

    #[test]
    fn a_data_fragment_after_a_command_without_a_data_set_is_rejected() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        assembler
            .push(pdv(
                PDataValueType::Command,
                &command_bytes(C_ECHO_RQ, NO_DATA_SET),
                true,
            ))
            .expect("valid PDV");
        assembler
            .push(pdv(PDataValueType::Data, &[1, 2], true))
            .expect_err("the command declared no data set");
    }

    #[test]
    fn a_command_during_an_incomplete_data_set_is_rejected() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV");
        assembler
            .push(pdv(PDataValueType::Data, &[1, 2], false))
            .expect("valid PDV");
        let error = assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect_err("the data set was incomplete");
        assert!(error.to_string().contains("incomplete"), "{}", error);
    }

    #[test]
    fn a_data_set_at_the_limit_is_accepted() {
        let mut assembler = assembler_with_limit(64);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assert!(command.len() <= 64, "the test command must fit the limit");
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV");
        assembler
            .push(pdv(PDataValueType::Data, &[0u8; 60], false))
            .expect("60 bytes are under the limit");
        let message = assembler
            .push(pdv(PDataValueType::Data, &[0u8; 4], true))
            .expect("64 bytes are at the limit")
            .expect("the message is complete");
        assert_eq!(message.data.len(), 64);
    }

    #[test]
    fn a_data_set_over_the_limit_is_rejected() {
        let mut assembler = assembler_with_limit(64);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("valid PDV");
        assembler
            .push(pdv(PDataValueType::Data, &[0u8; 60], false))
            .expect("60 bytes are under the limit");
        let error = assembler
            .push(pdv(PDataValueType::Data, &[0u8; 5], false))
            .expect_err("65 bytes are over the limit");
        let message = error.to_string();
        assert!(
            message.contains("data set") && message.contains("65") && message.contains("64"),
            "{}",
            message
        );
    }

    #[test]
    fn a_command_set_over_its_fixed_limit_is_rejected() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let fragment = vec![0u8; MAX_COMMAND_SET_BYTES as usize];
        assert!(assembler
            .push(pdv(PDataValueType::Command, &fragment, false))
            .expect("exactly the limit is accepted")
            .is_none());
        let error = assembler
            .push(pdv(PDataValueType::Command, &[0u8; 1], false))
            .expect_err("one byte more is over the limit");
        assert!(error.to_string().contains("command set"), "{}", error);
    }

    #[test]
    fn a_low_data_set_limit_does_not_affect_commands() {
        let mut assembler = assembler_with_limit(1);
        let command = command_bytes(C_ECHO_RQ, NO_DATA_SET);
        assert!(command.len() > 1);
        assembler
            .push(pdv(PDataValueType::Command, &command, true))
            .expect("the data set limit must not apply to the command set")
            .expect("the message is complete");
    }

    #[test]
    fn a_data_set_for_a_command_that_is_not_accepted_is_refused_before_buffering() {
        let mut assembler = MessageAssembler::new(NO_LIMIT, vec![C_FIND_RQ]);
        let error = assembler
            .push(pdv(
                PDataValueType::Command,
                &command_bytes(C_STORE_RQ, DATA_SET_PRESENT),
                true,
            ))
            .expect_err("C-STORE is not in the accepted list");
        assert!(error.to_string().contains("0x0001"), "{}", error);
    }

    #[test]
    fn a_command_that_is_not_accepted_still_passes_without_a_data_set() {
        let mut assembler = MessageAssembler::new(NO_LIMIT, Vec::new());
        let message = assembler
            .push(pdv(
                PDataValueType::Command,
                &command_bytes(C_ECHO_RQ, NO_DATA_SET),
                true,
            ))
            .expect("a command without a data set needs no entry in the list")
            .expect("the message is complete");
        assert_eq!(command_field(&message), C_ECHO_RQ);
    }

    #[test]
    fn a_data_fragment_on_another_presentation_context_is_rejected() {
        let mut assembler = assembler_with_limit(NO_LIMIT);
        let command = command_bytes(C_STORE_RQ, DATA_SET_PRESENT);
        assembler
            .push(pdv_on(1, PDataValueType::Command, &command, true))
            .expect("valid PDV");
        let error = assembler
            .push(pdv_on(3, PDataValueType::Data, &[1, 2], true))
            .expect_err("the presentation context changed");
        assert!(
            error.to_string().contains("presentation context 3"),
            "{}",
            error
        );
    }
}

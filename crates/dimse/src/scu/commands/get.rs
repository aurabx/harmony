//! C-GET command handler
//!
//! Native implementation on dicom-ul. The SCU sends C-GET-RQ and then services
//! the C-STORE sub-operations that the SCP issues on the same association. Each
//! instance is handed to the returned stream as soon as it has been received in
//! full, and its C-STORE-RSP goes back to the SCP only after the stream has
//! accepted it. A slow consumer therefore holds the SCP back instead of letting
//! instances pile up here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};

use dicom_dictionary_std::tags;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_ul::association::client::AsyncClientAssociation;
use dicom_ul::Pdu;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::common::{
    command_fields, create_command_pdata, encode_command, status, DimseMessageBuilder,
};
use crate::config::{DimseConfig, RemoteNode};
use crate::scu::{command_builder, native_connection, storage_sop_classes};
use crate::types::{DatasetStream, GetQuery, QueryLevel};
use crate::{DimseError, Result};

/// Study Root Query/Retrieve Information Model - GET
const GET_SOP_CLASS_STUDY: &str = "1.2.840.10008.5.1.4.1.2.2.3";
/// Patient Root Query/Retrieve Information Model - GET
const GET_SOP_CLASS_PATIENT: &str = "1.2.840.10008.5.1.4.1.2.1.3";

/// Instances the stream holds ahead of its consumer before the SCU stops
/// answering the SCP. Memory use is bounded by this many instances plus the
/// one being received.
const IN_FLIGHT_INSTANCES: usize = 8;

/// C-STORE status "Refused: Out of resources". Sent when the consumer of the
/// stream has gone away or an instance cannot be written to disk.
const STORE_STATUS_OUT_OF_RESOURCES: u16 = 0xA700;
/// C-STORE status "Error: Cannot understand". Sent when a received dataset
/// cannot be parsed.
const STORE_STATUS_CANNOT_UNDERSTAND: u16 = 0xC000;

static MESSAGE_ID_COUNTER: AtomicU16 = AtomicU16::new(1);

fn next_message_id() -> u16 {
    MESSAGE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// What happened to one C-STORE sub-operation.
enum StoreOutcome {
    /// The instance was handed to the stream and acknowledged to the SCP.
    Stored,
    /// The instance was refused with an error status and the C-GET continues.
    Rejected,
    /// The stream's receiver is gone, so the C-GET has to stop.
    ConsumerGone,
}

/// Handle C-GET request using the native DICOM upper layer.
///
/// With `output_dir` set, every instance is written there as a Part 10 file
/// and the stream yields `DatasetStream::File` items that the caller owns.
/// Without it, the stream yields `DatasetStream::Object` items and nothing
/// touches disk.
pub async fn handle_get(
    config: &DimseConfig,
    node: &RemoteNode,
    query: GetQuery,
    output_dir: Option<PathBuf>,
) -> Result<ReceiverStream<Result<DatasetStream>>> {
    info!(
        "Sending C-GET to {}@{}:{} (level: {})",
        node.ae_title, node.host, node.port, query.query_level,
    );

    node.validate()?;
    debug!("C-GET query parameters: {:?}", query.parameters);

    if let Some(dir) = &output_dir {
        tokio::fs::create_dir_all(dir).await.map_err(|e| {
            DimseError::operation_failed(format!(
                "Failed to create C-GET output directory {}: {}",
                dir.display(),
                e
            ))
        })?;
    }

    let (tx, rx) = mpsc::channel(IN_FLIGHT_INSTANCES);
    let config = config.clone();
    let node = node.clone();

    tokio::spawn(async move {
        if let Err(e) = perform_get(&config, &node, &query, output_dir.as_deref(), &tx).await {
            warn!(
                "C-GET from {}@{}:{} failed: {}",
                node.ae_title, node.host, node.port, e
            );
            let _ = tx.send(Err(e)).await;
        }
    });

    Ok(ReceiverStream::new(rx))
}

/// Run one C-GET to completion. Instances go to `tx`. The returned error, if
/// any, says why the retrieval stopped early.
async fn perform_get(
    config: &DimseConfig,
    node: &RemoteNode,
    query: &GetQuery,
    output_dir: Option<&Path>,
    tx: &mpsc::Sender<Result<DatasetStream>>,
) -> Result<()> {
    let retrieve_sop_class = match query.query_level {
        QueryLevel::Patient => GET_SOP_CLASS_PATIENT,
        _ => GET_SOP_CLASS_STUDY,
    };

    let mut association = native_connection::establish_retrieve_association(
        config,
        node,
        retrieve_sop_class,
        storage_sop_classes::STORAGE_SOP_CLASSES,
    )
    .await?;

    let accepted_storage = association
        .presentation_contexts()
        .iter()
        .filter(|pc| pc.abstract_syntax != retrieve_sop_class)
        .count();
    if accepted_storage == 0 {
        warn!(
            "C-GET: {} accepted no storage presentation context, so no instance can be received",
            node.ae_title
        );
    } else {
        debug!(
            "C-GET: {} storage presentation contexts accepted",
            accepted_storage
        );
    }

    let pc_id =
        match native_connection::get_presentation_context_id(&association, retrieve_sop_class) {
            Ok(id) => id,
            Err(e) => {
                native_connection::abort_association(association).await;
                return Err(e);
            }
        };

    let message_id = next_message_id();
    let identifier =
        command_builder::build_identifier_dataset(query.query_level, &query.parameters)?;
    let request = command_builder::build_command_request(
        command_fields::C_GET_RQ,
        message_id,
        true,
        retrieve_sop_class,
    );

    debug!("Sending C-GET request (message ID: {})", message_id);
    if let Err(e) = command_builder::encode_and_send_request(
        &mut association,
        request,
        Some(&identifier),
        pc_id,
    )
    .await
    {
        native_connection::abort_association(association).await;
        return Err(e);
    }

    let mut stored = 0u32;
    let mut rejected = 0u32;

    loop {
        let (command, dataset, command_pc_id) =
            match command_builder::receive_dimse_message(&mut association).await {
                Ok(message) => message,
                Err(e) => {
                    native_connection::abort_association(association).await;
                    return Err(DimseError::network(format!(
                        "Error receiving C-GET response after {} instances: {}",
                        stored, e
                    )));
                }
            };

        let command_field = command
            .element(tags::COMMAND_FIELD)
            .ok()
            .and_then(|e| e.uint16().ok())
            .unwrap_or(0);

        match command_field {
            command_fields::C_STORE_RQ => {
                match handle_store_request(
                    &mut association,
                    &command,
                    dataset,
                    command_pc_id,
                    output_dir,
                    tx,
                )
                .await
                {
                    Ok(StoreOutcome::Stored) => stored += 1,
                    Ok(StoreOutcome::Rejected) => rejected += 1,
                    Ok(StoreOutcome::ConsumerGone) => {
                        debug!(
                            "C-GET consumer went away after {} instances, aborting association",
                            stored
                        );
                        native_connection::abort_association(association).await;
                        return Ok(());
                    }
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                }
            }
            command_fields::C_GET_RSP => {
                let responded_to =
                    match command_builder::extract_message_id_being_responded_to(&command) {
                        Ok(id) => id,
                        Err(e) => {
                            native_connection::abort_association(association).await;
                            return Err(e);
                        }
                    };
                if responded_to != message_id {
                    debug!(
                        "Ignoring C-GET response for message ID {} (expected {})",
                        responded_to, message_id
                    );
                    continue;
                }

                let response_status = match command_builder::extract_status(&command) {
                    Ok(s) => s,
                    Err(e) => {
                        native_connection::abort_association(association).await;
                        return Err(e);
                    }
                };

                match response_status {
                    status::PENDING | status::PENDING_WARNING => {
                        debug!("C-GET pending: {}", sub_operation_summary(&command));
                    }
                    status::SUCCESS => {
                        info!(
                            "C-GET completed: {} instances received, {} rejected",
                            stored, rejected
                        );
                        let _ = native_connection::release_association(association).await;
                        return Ok(());
                    }
                    s if s & 0xF000 == 0xB000 => {
                        warn!(
                            "C-GET completed with warning status 0x{:04X} ({}); {} instances received",
                            s,
                            sub_operation_summary(&command),
                            stored
                        );
                        let _ = native_connection::release_association(association).await;
                        return Ok(());
                    }
                    status::CANCEL => {
                        let _ = native_connection::release_association(association).await;
                        return Err(DimseError::operation_failed(format!(
                            "C-GET cancelled by {} after {} instances",
                            node.ae_title, stored
                        )));
                    }
                    s => {
                        let _ = native_connection::release_association(association).await;
                        return Err(DimseError::operation_failed(format!(
                            "C-GET failed with status 0x{:04X} after {} instances ({})",
                            s,
                            stored,
                            sub_operation_summary(&command)
                        )));
                    }
                }
            }
            other => {
                native_connection::abort_association(association).await;
                return Err(DimseError::operation_failed(format!(
                    "Unexpected DIMSE command 0x{:04X} during C-GET",
                    other
                )));
            }
        }
    }
}

/// Service one C-STORE sub-operation: parse the instance, hand it to the
/// stream, then answer the SCP.
async fn handle_store_request(
    association: &mut AsyncClientAssociation<TcpStream>,
    command: &InMemDicomObject<StandardDataDictionary>,
    dataset: Option<Vec<u8>>,
    pc_id: u8,
    output_dir: Option<&Path>,
    tx: &mpsc::Sender<Result<DatasetStream>>,
) -> Result<StoreOutcome> {
    let store_message_id = command
        .element(tags::MESSAGE_ID)
        .map_err(|_| DimseError::parse("C-STORE-RQ has no message ID"))?
        .uint16()
        .map_err(|e| DimseError::parse(format!("Invalid C-STORE-RQ message ID: {}", e)))?;
    let sop_class_uid = command_uid(command, tags::AFFECTED_SOP_CLASS_UID);
    let sop_instance_uid = command_uid(command, tags::AFFECTED_SOP_INSTANCE_UID);

    let Some(dataset_bytes) = dataset else {
        warn!(
            "C-STORE-RQ for instance {} carried no dataset",
            sop_instance_uid
        );
        send_store_response(
            association,
            store_message_id,
            STORE_STATUS_CANNOT_UNDERSTAND,
            &sop_class_uid,
            &sop_instance_uid,
            pc_id,
        )
        .await?;
        return Ok(StoreOutcome::Rejected);
    };

    let transfer_syntax = transfer_syntax_for(association, pc_id)?;
    let object = match command_builder::parse_dataset_bytes(dataset_bytes, pc_id, association) {
        Ok(object) => object,
        Err(e) => {
            warn!(
                "Cannot parse instance {} received in C-GET: {}",
                sop_instance_uid, e
            );
            send_store_response(
                association,
                store_message_id,
                STORE_STATUS_CANNOT_UNDERSTAND,
                &sop_class_uid,
                &sop_instance_uid,
                pc_id,
            )
            .await?;
            return Ok(StoreOutcome::Rejected);
        }
    };

    let mut dataset = DatasetStream::from_object(object);
    dataset.metadata_mut().transfer_syntax = Some(transfer_syntax);

    let dataset = match output_dir {
        Some(dir) => match write_instance_file(dir, dataset).await {
            Ok(file_dataset) => file_dataset,
            Err(e) => {
                warn!(
                    "Cannot write instance {} received in C-GET to {}: {}",
                    sop_instance_uid,
                    dir.display(),
                    e
                );
                send_store_response(
                    association,
                    store_message_id,
                    STORE_STATUS_OUT_OF_RESOURCES,
                    &sop_class_uid,
                    &sop_instance_uid,
                    pc_id,
                )
                .await?;
                return Ok(StoreOutcome::Rejected);
            }
        },
        None => dataset,
    };

    if tx.send(Ok(dataset)).await.is_err() {
        let _ = send_store_response(
            association,
            store_message_id,
            STORE_STATUS_OUT_OF_RESOURCES,
            &sop_class_uid,
            &sop_instance_uid,
            pc_id,
        )
        .await;
        return Ok(StoreOutcome::ConsumerGone);
    }

    send_store_response(
        association,
        store_message_id,
        status::SUCCESS,
        &sop_class_uid,
        &sop_instance_uid,
        pc_id,
    )
    .await?;
    Ok(StoreOutcome::Stored)
}

/// Write an in-memory instance to `dir` as a Part 10 file named after its SOP
/// Instance UID and return the file-backed dataset that replaces it.
async fn write_instance_file(dir: &Path, dataset: DatasetStream) -> Result<DatasetStream> {
    let metadata = dataset.metadata().clone();
    let stem = file_stem_for(
        metadata.sop_instance_uid.as_deref().unwrap_or(""),
        &metadata.id,
    );
    let path = dir.join(format!("{}.dcm", stem));

    let bytes = dataset.to_part10_bytes()?;
    tokio::fs::write(&path, &bytes).await?;

    Ok(DatasetStream::File {
        path,
        metadata,
        delete_on_drop: false,
    })
}

/// A file stem built from a SOP Instance UID. Anything other than digits and
/// dots is dropped so that a hostile UID cannot escape the output directory.
/// Falls back to the dataset id when nothing usable is left.
fn file_stem_for(sop_instance_uid: &str, fallback: &uuid::Uuid) -> String {
    let stem: String = sop_instance_uid
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if stem.chars().any(|c| c.is_ascii_digit()) {
        stem
    } else {
        fallback.to_string()
    }
}

/// Send a C-STORE-RSP for a sub-operation of this C-GET.
async fn send_store_response(
    association: &mut AsyncClientAssociation<TcpStream>,
    message_id: u16,
    status_code: u16,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    pc_id: u8,
) -> Result<()> {
    let mut builder = DimseMessageBuilder::new()
        .command_field(command_fields::C_STORE_RSP)
        .message_id_being_responded_to(message_id)
        .status(status_code)
        .has_dataset(false);
    if !sop_class_uid.is_empty() {
        builder = builder.affected_sop_class_uid(sop_class_uid);
    }
    if !sop_instance_uid.is_empty() {
        builder = builder.affected_sop_instance_uid(sop_instance_uid);
    }

    let response_bytes = encode_command(&builder.build())?;
    association
        .send(&Pdu::PData {
            data: vec![create_command_pdata(pc_id, response_bytes, true)],
        })
        .await
        .map_err(|e| DimseError::network(format!("Failed to send C-STORE-RSP: {}", e)))
}

/// The transfer syntax negotiated for a presentation context.
fn transfer_syntax_for(
    association: &AsyncClientAssociation<TcpStream>,
    pc_id: u8,
) -> Result<String> {
    association
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == pc_id)
        .map(|pc| pc.transfer_syntax.clone())
        .ok_or_else(|| {
            DimseError::parse(format!(
                "C-STORE-RQ arrived on unknown presentation context {}",
                pc_id
            ))
        })
}

/// A UID string from a command set, empty when absent.
fn command_uid(command: &InMemDicomObject<StandardDataDictionary>, tag: dicom_core::Tag) -> String {
    command
        .element(tag)
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.trim_end_matches('\0').trim().to_string())
        .unwrap_or_default()
}

/// The sub-operation counts of a C-GET-RSP, for logging.
fn sub_operation_summary(command: &InMemDicomObject<StandardDataDictionary>) -> String {
    let count = |tag: dicom_core::Tag| {
        command
            .element(tag)
            .ok()
            .and_then(|e| e.uint16().ok())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    };
    format!(
        "remaining={} completed={} failed={} warning={}",
        count(tags::NUMBER_OF_REMAINING_SUBOPERATIONS),
        count(tags::NUMBER_OF_COMPLETED_SUBOPERATIONS),
        count(tags::NUMBER_OF_FAILED_SUBOPERATIONS),
        count(tags::NUMBER_OF_WARNING_SUBOPERATIONS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stem_keeps_digits_and_dots_only() {
        let id = uuid::Uuid::nil();
        assert_eq!(
            file_stem_for("1.2.840.113619.2.55.3", &id),
            "1.2.840.113619.2.55.3"
        );
        assert_eq!(file_stem_for("../1.2.3", &id), "..1.2.3");
        assert_eq!(file_stem_for("1.2.3\0", &id), "1.2.3");
    }

    #[test]
    fn file_stem_falls_back_to_the_dataset_id() {
        let id = uuid::Uuid::nil();
        assert_eq!(file_stem_for("", &id), id.to_string());
        assert_eq!(file_stem_for("../../", &id), id.to_string());
        assert_eq!(file_stem_for("no-digits", &id), id.to_string());
    }
}

use crate::adapters::dimse::envelope_unwrapper::{unwrap_response, UnwrappedContent};
use crate::adapters::dimse::status_mapper;
use crate::config::config::Config;
use crate::globals::{get_config, get_storage};
use crate::models::envelope::command::{Command, Operation};
use crate::models::envelope::envelope::{LocalInstanceFile, ResponseEnvelope};
use crate::models::middleware::types::policies::IpAccessRules;
use crate::models::middleware::DicomObjectNotFound;
use crate::models::protocol::{Protocol, ProtocolCtx};
use crate::pipeline::executor::{PipelineError, PipelineExecutor};
use async_trait::async_trait;
use dicom_json_tool as tool;
use dimse::error::DimseError;
use dimse::scp::PeerAdmission;
use dimse::types::{
    dataset_stream_from_vec, BoxDatasetStream, DatasetStream, DimseStatus, QueryLevel,
    RetrieveStream,
};
use dimse::Result as DimseResult;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

static CURRENT_STORE_DIR: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

// pub fn set_current_store_dir<P: Into<PathBuf>>(dir: P) {
//     let mut guard = CURRENT_STORE_DIR.lock().expect("store dir mutex");
//     *guard = Some(dir.into());
// }

fn get_current_store_dir() -> Option<PathBuf> {
    CURRENT_STORE_DIR.lock().ok().and_then(|g| g.clone())
}

/// Longest part of a receiver's failure reason that is kept in the error for
/// a failed C-STORE. The text comes from another host and ends up in the log.
const MAX_RECEIVER_REASON_CHARS: usize = 512;

/// Processing failure. PS3.7 defines it as a general failure status, so it
/// is valid for C-FIND, C-GET and C-MOVE.
const PROCESSING_FAILURE: u16 = 0x0110;

/// Unable to process. Also what SCP handlers send for an error with no
/// status code.
const UNABLE_TO_PROCESS: u16 = 0xC000;

/// The status code to send for a failed `operation`, from the status mapper's
/// answer.
///
/// PS3.4 defines 0xA701 and 0xA702 only for C-GET and C-MOVE, so a C-FIND
/// gets 0xA700 (Refused: Out of resources) in their place. 0xA801 means
/// "Move Destination unknown" and is defined only for C-MOVE. Only the SCP's
/// own destination check may send it, so a pipeline failure that maps to it
/// gets 0xC000. The C-GET and C-MOVE tables have no 0xA700, so those get
/// 0xA701 in its place. A request that matched nothing never gets here (see
/// `is_no_match_error` and `is_no_match_http`). A status that is not a
/// failure also becomes 0xC000, because an error must never reach the client
/// as a success.
fn failure_status_for_operation(operation: Operation, status: DimseStatus) -> u16 {
    let code = match status {
        DimseStatus::Failure(code) => code,
        other => {
            tracing::warn!(
                "DIMSE {} failed but the status mapper gave {:?}; answering 0x{:04X}",
                operation,
                other,
                UNABLE_TO_PROCESS
            );
            return UNABLE_TO_PROCESS;
        }
    };
    match (operation, code) {
        (_, 0xA801) => UNABLE_TO_PROCESS,
        (Operation::Query, 0xA701 | 0xA702) => 0xA700,
        (Operation::Retrieve | Operation::Move, 0xA700) => 0xA701,
        _ => code,
    }
}

/// True when `code` is a C-STORE warning: Coercion of data elements
/// (0xB000), Elements discarded (0xB006), Data set does not match SOP class
/// (0xB007), and the rest of the 0xBxxx range, plus the PS3.7 general
/// warnings 0x0001, 0x0107 and 0x0116. The instance was stored.
fn is_store_warning(code: u16) -> bool {
    matches!(code, 0x0001 | 0x0107 | 0x0116 | 0xB000..=0xBFFF)
}

/// True for the operations that PS3.4 answers with Success and zero results
/// or sub-operations when nothing matches.
fn has_empty_result(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::Query | Operation::Retrieve | Operation::Move
    )
}

/// True when a failed pipeline run says the request matched nothing.
///
/// Only the typed `DicomObjectNotFound` counts. Error text is never read for
/// this. It can hold remote status codes, ports and instance counts, so
/// reading "404" or "not found" in it would show an outage as an empty
/// result.
fn is_no_match_error(operation: Operation, error: &PipelineError) -> bool {
    has_empty_result(operation)
        && matches!(
            error,
            PipelineError::MiddlewareError(e) if e.downcast_ref::<DicomObjectNotFound>().is_some()
        )
}

/// True when the HTTP status of a whole, unstreamed response says the
/// request matched nothing.
///
/// Only C-GET and C-MOVE. The `dicom_scu` backend answers 404 for a retrieve
/// whose study is not found, and WADO-RS answers 404 for a missing study. A
/// query that matches nothing answers 2xx with an empty list (`dicom_scu`,
/// QIDO-RS), so a 404 for a C-FIND is a routing error or a denial.
fn is_no_match_http(operation: Operation, http_status: u16) -> bool {
    matches!(operation, Operation::Retrieve | Operation::Move) && matches!(http_status, 404 | 410)
}

/// How the SCP answers a failed pipeline run.
#[derive(Debug)]
enum PipelineFailure {
    /// The request matched nothing. The client gets Success and no results.
    NoMatches,
    /// The request failed. The SCP sends the error's status code and logs
    /// its detail.
    Failed(DimseError),
}

/// Decide how the SCP answers a failed pipeline run.
///
/// A failure carries the pipeline error as its detail. The SCP logs the
/// detail and sends only the status code, because the pipeline error often
/// names hosts, ports and AE titles.
fn pipeline_failure(operation: Operation, error: &PipelineError) -> PipelineFailure {
    if is_no_match_error(operation, error) {
        tracing::info!(
            "DIMSE {} matched nothing, answering with an empty result: {:?}",
            operation,
            error.to_string()
        );
        return PipelineFailure::NoMatches;
    }
    let code =
        failure_status_for_operation(operation, status_mapper::pipeline_error_to_dimse(error));
    PipelineFailure::Failed(DimseError::status(
        code,
        format!("DIMSE {} pipeline failed: {}", operation, error),
    ))
}

impl PipelineFailure {
    /// The error for an operation that has no empty result, such as C-STORE.
    /// `is_no_match_error` never gives `NoMatches` for those, so this is a
    /// guard.
    fn into_error(self) -> DimseError {
        match self {
            PipelineFailure::NoMatches => {
                DimseError::status(UNABLE_TO_PROCESS, "the pipeline found no matching object")
            }
            PipelineFailure::Failed(error) => error,
        }
    }
}

/// The failure reason in a JSON response body that has `"success": false` at
/// the top level. `None` when the body does not report a failure, including
/// a body with no `success` key.
///
/// The reason is text from another host. It is cut to a fixed length so a
/// caller can log it with `{:?}`, which escapes control characters, without
/// letting it add lines to the log.
fn body_failure_reason(body: &serde_json::Value) -> Option<String> {
    if body.get("success") != Some(&serde_json::Value::Bool(false)) {
        return None;
    }
    Some(
        body.get("error")
            .and_then(|error| error.as_str())
            .unwrap_or("no reason given")
            .chars()
            .take(MAX_RECEIVER_REASON_CHARS)
            .collect(),
    )
}

/// A DICOM Part 10 file encoded for a JSON pipeline body.
struct InlineInstance {
    /// The whole Part 10 file in standard base64.
    part10_b64: String,
    /// Transfer Syntax UID from the file meta group.
    transfer_syntax_uid: String,
}

impl InlineInstance {
    /// Encode `part10` and read its transfer syntax from the file meta group,
    /// so the stated transfer syntax always matches what a reader of the file
    /// will use.
    fn from_part10(part10: &[u8]) -> DimseResult<Self> {
        use base64::Engine;

        // The file meta group starts at the `DICM` code, after the 128-byte preamble.
        let meta_group = part10.get(128..).ok_or_else(|| {
            DimseError::operation_failed(format!(
                "stored instance is {} bytes, too short for a DICOM Part 10 file",
                part10.len()
            ))
        })?;
        let meta = dicom_object::meta::FileMetaTable::from_reader(meta_group).map_err(|e| {
            DimseError::operation_failed(format!(
                "stored instance has no readable file meta group: {}",
                e
            ))
        })?;

        Ok(Self {
            part10_b64: base64::engine::general_purpose::STANDARD.encode(part10),
            transfer_syntax_uid: meta.transfer_syntax().to_string(),
        })
    }
}

/// The adapter's on-disk copy of one received C-STORE instance.
///
/// The copy is written before the pipeline runs, so the `policies`
/// middleware (IP allow and deny lists, rate limits) only sees the request
/// after the file exists. Removing the file on drop means every way out of
/// `store()` deletes it unless `keep` was called: a policy denial, a backend
/// failure, an early error, or a success. Without this, a denied client could
/// fill the disk and leave unvetted PHI on it.
///
/// A missing file is not an error. `dimse_to_jmix` moves the file into its
/// staging folder while the pipeline runs.
struct StoredInstanceCopy {
    path: PathBuf,
    kept: bool,
}

impl StoredInstanceCopy {
    fn new(path: PathBuf) -> Self {
        Self { path, kept: false }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Leave the file on disk when the copy is dropped.
    fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for StoredInstanceCopy {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                "Failed to remove the stored C-STORE instance {}: {}",
                self.path.display(),
                e
            ),
        }
    }
}

/// DIMSE query provider that integrates with the pipeline system
///
/// This provider implements the dimse::scp::QueryProvider trait to handle
/// C-FIND, C-MOVE, and C-STORE operations by converting them to protocol
/// contexts and executing them through the PipelineExecutor.
pub struct PipelineQueryProvider {
    pipeline: String,
    endpoint: String,
}

impl PipelineQueryProvider {
    pub fn new(pipeline: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            pipeline: pipeline.into(),
            endpoint: endpoint.into(),
        }
    }

    /// Whether a peer at `ip` may open an association, judged by this
    /// provider's pipeline in `config`.
    fn admission(&self, config: &crate::config::config::Config, ip: IpAddr) -> PeerAdmission {
        match peer_admission(config, &self.pipeline, ip) {
            Ok(()) => PeerAdmission::Admit,
            Err(reason) => PeerAdmission::Reject {
                reason: format!("pipeline '{}': {}", self.pipeline, reason),
            },
        }
    }

    /// Build DICOM JSON identifier from DIMSE query parameters
    fn build_identifier_json(&self, parameters: &HashMap<String, String>) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (tag, val) in parameters.iter() {
            let vr = match tag.as_str() {
                "00100010" => "PN",
                "00100020" => "LO",
                "00080020" => "DA",
                _ => "UN",
            };
            let value = if val.is_empty() {
                serde_json::Value::Array(vec![])
            } else if vr == "PN" {
                serde_json::json!([{ "Alphabetic": val }])
            } else {
                serde_json::json!([val])
            };
            map.insert(tag.clone(), serde_json::json!({ "vr": vr, "Value": value }));
        }
        serde_json::Value::Object(map)
    }

    /// Build query metadata with match types
    fn build_query_metadata(
        &self,
        parameters: &HashMap<String, String>,
    ) -> tool::model::QueryMetadata {
        let mut out: HashMap<String, tool::model::QueryMetaEntry> = HashMap::new();
        for (tag, val) in parameters.iter() {
            let match_type = if val.is_empty() {
                "RETURN_KEY"
            } else if val.contains('*') || val.contains('?') {
                "WILDCARD"
            } else if (tag == "00080020" || tag == "00080021") && val.contains('-') {
                "RANGE"
            } else {
                "EXACT"
            };
            out.insert(
                tag.clone(),
                tool::model::QueryMetaEntry {
                    match_type: Some(match_type.into()),
                },
            );
        }
        tool::model::QueryMetadata(out)
    }

    /// Execute a DIMSE operation through the pipeline
    ///
    /// Converts DIMSE operation to protocol context, builds envelope via service,
    /// executes pipeline, and returns response envelope.
    /// `local_instance` is the adapter's copy of a received C-STORE
    /// instance. It is set only for a backend that reads it (see
    /// `store_attaches_local_instance`).
    ///
    /// Returns `None` when the request matched nothing (see `is_no_match`).
    async fn run(
        &self,
        config: &Config,
        command: Command,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
        local_instance: Option<LocalInstanceFile>,
    ) -> DimseResult<Option<ResponseEnvelope<Vec<u8>>>> {
        // Resolve endpoint service and options
        let endpoint = config.endpoints.get(&self.endpoint).ok_or_else(|| {
            DimseError::operation_failed(format!("Unknown endpoint '{}'", self.endpoint))
        })?;
        let service = endpoint
            .resolve_service()
            .map_err(|e| DimseError::operation_failed(format!("Resolve service failed: {}", e)))?;
        let options_owned: HashMap<String, serde_json::Value> =
            endpoint.options.clone().unwrap_or_default();
        let options = &options_owned;

        // Build ProtocolCtx for DIMSE with typed Command.
        // The Command is the single source of truth for the operation.
        meta.insert("protocol".into(), "dimse".into());
        let operation = command.operation;
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: Some(command),
        };

        // Let the service build the request envelope
        let mut request_envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;
        request_envelope.local_instance = local_instance;

        // Get pipeline configuration
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        match PipelineExecutor::execute(
            request_envelope,
            &self.pipeline,
            pipeline_cfg,
            config,
            &ctx,
        )
        .await
        {
            Ok(response_envelope) => Ok(Some(response_envelope)),
            Err(e) => match pipeline_failure(operation, &e) {
                PipelineFailure::NoMatches => Ok(None),
                PipelineFailure::Failed(error) => Err(error),
            },
        }
    }

    /// Execute a DIMSE operation through the pipeline with streaming support
    ///
    /// Similar to `run()` but uses `execute_streaming()` for backends that support it.
    /// Returns the datasets as a stream. Each streamed envelope is converted as
    /// it arrives, so the caller can act on the first dataset while the backend
    /// is still producing the rest.
    async fn run_streaming(
        &self,
        config: &Config,
        command: Command,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
        use futures_util::StreamExt;

        // Resolve endpoint service and options
        let endpoint = config.endpoints.get(&self.endpoint).ok_or_else(|| {
            DimseError::operation_failed(format!("Unknown endpoint '{}'", self.endpoint))
        })?;
        let service = endpoint
            .resolve_service()
            .map_err(|e| DimseError::operation_failed(format!("Resolve service failed: {}", e)))?;
        let options_owned: HashMap<String, serde_json::Value> =
            endpoint.options.clone().unwrap_or_default();
        let options = &options_owned;

        // Build ProtocolCtx for DIMSE with typed Command.
        // The Command is the single source of truth for the operation.
        meta.insert("protocol".into(), "dimse".into());
        let operation = command.operation;
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: Some(command),
        };

        // Let the service build the request envelope
        let request_envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;

        // Get pipeline configuration
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        tracing::info!(
            "Executing streaming pipeline for DIMSE operation: {}",
            operation
        );

        // Execute streaming pipeline
        let response_stream = match PipelineExecutor::execute_streaming(
            request_envelope,
            &self.pipeline,
            pipeline_cfg,
            config,
            &ctx,
        )
        .await
        {
            Ok(response_stream) => response_stream,
            Err(e) => match pipeline_failure(operation, &e) {
                PipelineFailure::NoMatches => return Ok(dataset_stream_from_vec(vec![])),
                PipelineFailure::Failed(error) => return Err(error),
            },
        };

        // Each envelope becomes zero or more datasets as it arrives. Nothing is
        // collected here.
        let datasets = response_stream
            .enumerate()
            .then(move |(index, result)| async move {
                let instance = index as u64 + 1;
                match result {
                    Ok(response_envelope) => {
                        tracing::debug!("Processing streamed instance #{}", instance);
                        match Self::response_to_datasets(response_envelope, operation, true).await {
                            Ok(datasets) => datasets.into_iter().map(Ok).collect::<Vec<_>>(),
                            Err(e) => {
                                tracing::error!(
                                    "Streamed instance #{} could not be converted: {}",
                                    instance,
                                    e
                                );
                                vec![Err(e)]
                            }
                        }
                    }
                    // The streaming executor reports a failed backend as a
                    // backend error, so a failed stream item is mapped as one.
                    // A stream that started and then failed is never an empty
                    // result, or the client would take a partial answer for a
                    // complete one.
                    Err(e) => {
                        let error = PipelineError::BackendError(format!(
                            "Stream failed on instance #{} after {} instances: {}",
                            instance, index, e
                        ));
                        vec![Err(pipeline_failure(operation, &error).into_error())]
                    }
                }
            })
            .flat_map(futures_util::stream::iter);

        Ok(Box::pin(datasets))
    }

    /// Execute a DIMSE operation through a streaming pipeline and collect a single response envelope.
    ///
    /// Some DIMSE operations (notably C-STORE) need to inspect response metadata
    /// set by outgoing middleware, but their pipeline may be backed by a
    /// streaming backend that refuses non-streaming execution. This helper
    /// mirrors `run_streaming` for setup, drives the pipeline via
    /// `PipelineExecutor::execute_streaming`, and consumes the response stream
    /// down to a single envelope. The stream must yield at least one envelope.
    async fn run_streaming_response(
        &self,
        config: &Config,
        command: Command,
        body: serde_json::Value,
        mut meta: HashMap<String, String>,
    ) -> DimseResult<ResponseEnvelope<Vec<u8>>> {
        use futures_util::StreamExt;

        // Resolve endpoint service and options
        let endpoint = config.endpoints.get(&self.endpoint).ok_or_else(|| {
            DimseError::operation_failed(format!("Unknown endpoint '{}'", self.endpoint))
        })?;
        let service = endpoint
            .resolve_service()
            .map_err(|e| DimseError::operation_failed(format!("Resolve service failed: {}", e)))?;
        let options_owned: HashMap<String, serde_json::Value> =
            endpoint.options.clone().unwrap_or_default();
        let options = &options_owned;

        meta.insert("protocol".into(), "dimse".into());
        let operation = command.operation;
        let ctx = ProtocolCtx {
            protocol: Protocol::Dimse,
            payload: serde_json::to_vec(&body).unwrap_or_default(),
            meta,
            attrs: serde_json::json!({}),
            command: Some(command),
        };

        let request_envelope = service
            .build_protocol_envelope(ctx.clone(), options)
            .await
            .map_err(|e| DimseError::operation_failed(format!("Envelope build failed: {}", e)))?;

        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        tracing::info!(
            "Executing streaming pipeline for DIMSE operation: {} (single-envelope)",
            operation
        );

        let mut response_stream = PipelineExecutor::execute_streaming(
            request_envelope,
            &self.pipeline,
            pipeline_cfg,
            config,
            &ctx,
        )
        .await
        .map_err(|e| pipeline_failure(operation, &e).into_error())?;

        // Consume the stream and keep the last successful envelope. Middleware
        // (for example dicom_to_dicomweb) may set final response metadata such
        // as dicom_status on the last envelope.
        let mut last: Option<ResponseEnvelope<Vec<u8>>> = None;
        while let Some(item) = response_stream.next().await {
            match item {
                Ok(envelope) => last = Some(envelope),
                Err(e) => {
                    let error = PipelineError::BackendError(format!(
                        "Streaming pipeline yielded error: {}",
                        e
                    ));
                    return Err(pipeline_failure(operation, &error).into_error());
                }
            }
        }

        last.ok_or_else(|| {
            DimseError::operation_failed("Streaming pipeline produced no response envelopes")
        })
    }

    /// True when the pipeline has a consumer that needs the stored instance
    /// itself: the `dicom_scu` or `dicom_scu_streaming` backend, which never
    /// opens a path it is given, or the `dicom_to_http` middleware in front of
    /// an `http_streaming_backend`, which sends the instance to another host.
    /// Every other pipeline is spared the extra copy.
    ///
    /// `dicom_to_http` in front of the `http` backend is left out on purpose.
    /// That backend does not send the headers `dicom_to_http` sets, so the
    /// receiver could not tell the request is a store.
    fn store_needs_inline_instance(
        pipeline: &crate::models::pipelines::config::Pipeline,
        config: &crate::config::config::Config,
    ) -> bool {
        let Some(backend) = pipeline
            .backends
            .first()
            .and_then(|name| config.backends.get(name))
        else {
            return false;
        };
        // `dicom` is the legacy name of `dicom_scu` (see `DicomScuServiceFactory`).
        if ["dicom_scu", "dicom", "dicom_scu_streaming"]
            .iter()
            .any(|service| backend.service.eq_ignore_ascii_case(service))
        {
            return true;
        }
        // A pipeline may name a `[middleware.<name>]` table, or a built-in
        // type with no table. The middleware builder accepts both, so this
        // must too, or the bridge would run without the instance.
        backend
            .service
            .eq_ignore_ascii_case("http_streaming_backend")
            && pipeline.middleware.left_chain().iter().any(|name| {
                let middleware_type = config
                    .middleware
                    .get(name)
                    .map_or(name.as_str(), |m| m.middleware_type.as_str());
                middleware_type.eq_ignore_ascii_case("dicom_to_http")
            })
    }

    /// True when the pipeline's backend copies the adapter's file itself: a
    /// `storage` backend with `content = "dicom_part10"`. It gets the file as
    /// `RequestEnvelope::local_instance` and streams it, so a large instance
    /// is never held in memory.
    fn store_attaches_local_instance(
        pipeline: &crate::models::pipelines::config::Pipeline,
        config: &crate::config::config::Config,
    ) -> bool {
        let Some(backend) = pipeline
            .backends
            .first()
            .and_then(|name| config.backends.get(name))
        else {
            return false;
        };
        backend.service.eq_ignore_ascii_case("storage")
            && backend
                .options
                .as_ref()
                .and_then(|options| options.get("content"))
                .and_then(|content| content.as_str())
                == Some(crate::models::services::types::storage::CONTENT_DICOM_PART10)
    }

    /// Build the pipeline body for one received C-STORE instance.
    ///
    /// The `file` key holds the path of the Part 10 copy on this host.
    /// Middleware that runs in this process (`dicom_to_dicomweb`,
    /// `dimse_to_jmix`) reads it. `inline_instance` adds the instance itself
    /// as `part10_b64` plus its `transfer_syntax_uid`.
    ///
    /// The key is not `body_b64`. The executor sends any rewritten body that
    /// has a `body_b64` key as raw bytes, which would replace the JSON that a
    /// middleware such as `dicom_flatten` writes.
    fn build_store_body(
        sop_class_uid: Option<&str>,
        dicom_json: Option<serde_json::Value>,
        temp_file: &std::path::Path,
        inline_instance: Option<InlineInstance>,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "command": {
                "message_id": 1,
                "sop_class_uid": sop_class_uid,
                "priority": "MEDIUM",
                "direction": "REQUEST"
            },
            "dataset": dicom_json,
            "file": temp_file.to_string_lossy()
        });
        if let Some(instance) = inline_instance {
            body["part10_b64"] = serde_json::Value::String(instance.part10_b64);
            body["transfer_syntax_uid"] = serde_json::Value::String(instance.transfer_syntax_uid);
        }
        body
    }

    /// Decide from the pipeline's final response whether a C-STORE reached its
    /// destination.
    ///
    /// The SCP answers the sending device from this result. A device may
    /// delete its copy of an instance once the store is acknowledged, so `Ok`
    /// must mean the instance is stored.
    ///
    /// Two things mark a failed store:
    /// - `dicom_status` in the response metadata. A backend or middleware in
    ///   this process sets it (`dicom_scu_streaming`, `dicom_to_dicomweb`).
    /// - `"success": false` in the response body. A receiving Harmony answers
    ///   HTTP 200 to a store its PACS did not take and reports the failure
    ///   only in the body, because response metadata does not cross the HTTP
    ///   hop.
    fn check_store_response(response: &ResponseEnvelope<Vec<u8>>) -> DimseResult<()> {
        // The status and body checks name the receiver's reason, so they run
        // before the bare HTTP status check.
        Self::check_store_dicom_status(response)?;
        Self::check_store_body(response)?;
        Self::check_store_http_status(response)
    }

    /// Fail the store when the pipeline's final HTTP status is not 2xx. A
    /// backend such as `http` can answer 500 or 403 with a body that has no
    /// `success` key, and that store did not happen.
    fn check_store_http_status(response: &ResponseEnvelope<Vec<u8>>) -> DimseResult<()> {
        let http_status = response.response_details.status;
        if (200..300).contains(&http_status) {
            return Ok(());
        }
        Err(DimseError::operation_failed(format!(
            "C-STORE failed: the pipeline answered HTTP status {}",
            http_status
        )))
    }

    /// Fail the store when `dicom_status` in the response metadata is not
    /// Success or a warning. The value has the form `0xC000`.
    ///
    /// C-STORE failure codes include 0xA7xx, 0xA9xx and the PS3.7 general
    /// failures such as 0x0110 and 0x0124, so every code that is not Success
    /// or a warning is a failure. A value that cannot be read is a failure
    /// too, because an acknowledged store lets the sender delete its copy.
    fn check_store_dicom_status(response: &ResponseEnvelope<Vec<u8>>) -> DimseResult<()> {
        let Some(status_str) = response.response_details.metadata.get("dicom_status") else {
            return Ok(());
        };
        let status = status_str
            .strip_prefix("0x")
            .and_then(|hex| u16::from_str_radix(hex, 16).ok());
        match status {
            Some(0x0000) => Ok(()),
            Some(code) if is_store_warning(code) => {
                tracing::warn!("C-STORE completed with warning status: 0x{:04X}", code);
                Ok(())
            }
            Some(code) => Err(DimseError::operation_failed(format!(
                "C-STORE failed with DICOM status: 0x{:04X}",
                code
            ))),
            None => Err(DimseError::operation_failed(format!(
                "C-STORE failed: unreadable DICOM status {:?}",
                status_str
            ))),
        }
    }

    /// Fail the store when the JSON response body has `"success": false` at
    /// the top level. This is how the store backends report a store that did
    /// not reach its destination. The receiver's `error` text, when it gives
    /// one, becomes part of the returned error.
    ///
    /// A body with no `success` key says nothing about the store, so it is not
    /// a failure here.
    fn check_store_body(response: &ResponseEnvelope<Vec<u8>>) -> DimseResult<()> {
        let Some(reason) = response
            .normalized_data
            .as_ref()
            .and_then(body_failure_reason)
        else {
            return Ok(());
        };
        Err(DimseError::operation_failed(format!(
            "C-STORE failed: the receiver reported that the instance was not stored: {:?}",
            reason
        )))
    }

    /// The datasets for a `run` result. A run that matched nothing has none.
    async fn run_result_to_datasets(
        response: Option<ResponseEnvelope<Vec<u8>>>,
        operation: Operation,
    ) -> DimseResult<Vec<DatasetStream>> {
        match response {
            Some(response) => Self::response_to_datasets(response, operation, false).await,
            None => Ok(vec![]),
        }
    }

    /// Convert ResponseEnvelope to Vec<DatasetStream>
    ///
    /// Parses the response data and converts it to DICOM datasets.
    /// Uses the envelope_unwrapper module to handle various response formats.
    ///
    /// A failed response becomes an error that carries the DIMSE status for
    /// `operation`. A non-2xx HTTP status is mapped by the status mapper. A
    /// whole response whose status means "matched nothing" is an empty result
    /// (see `is_no_match_http`). A `streamed` envelope never is, because
    /// earlier envelopes already reached the client. A
    /// 2xx response whose body has `"success": false` gets 0x0110, because
    /// the unwrapper would read that body as zero matches and the client
    /// would get Success.
    async fn response_to_datasets(
        response: ResponseEnvelope<Vec<u8>>,
        operation: Operation,
        streamed: bool,
    ) -> DimseResult<Vec<DatasetStream>> {
        let http_status = response.response_details.status;
        if !(200..300).contains(&http_status) {
            let status = status_mapper::http_status_to_dimse(http_status);
            if !streamed && is_no_match_http(operation, http_status) {
                tracing::info!(
                    "DIMSE {} pipeline returned HTTP status {}, answering with an empty result",
                    operation,
                    http_status
                );
                return Ok(vec![]);
            }
            return Err(DimseError::status(
                failure_status_for_operation(operation, status),
                format!(
                    "DIMSE {} pipeline returned HTTP status {}",
                    operation, http_status
                ),
            ));
        }

        // Try to parse response body as JSON
        let json_data = if let Some(normalized) = response.normalized_data {
            normalized
        } else if !response.original_data.is_empty() {
            serde_json::from_slice(&response.original_data).map_err(|e| {
                DimseError::operation_failed(format!("Failed to parse response as JSON: {}", e))
            })?
        } else {
            return Ok(vec![]);
        };

        // A response that crossed a Harmony HTTP hop, or passed middleware
        // that set its status, can be a 2xx that reports the failure only in
        // its body.
        if let Some(reason) = body_failure_reason(&json_data) {
            return Err(DimseError::status(
                PROCESSING_FAILURE,
                format!(
                    "DIMSE {} pipeline reported a failure in its response body: {:?}",
                    operation, reason
                ),
            ));
        }

        // Use envelope unwrapper to extract clean DICOM content
        match unwrap_response(&json_data) {
            UnwrappedContent::DicomJson(items) => {
                tracing::debug!(
                    "response_to_datasets: unwrapped {} DICOM JSON objects",
                    items.len()
                );
                let mut datasets = Vec::new();
                for item in items {
                    match Self::dicom_json_to_dataset(&item) {
                        Ok(dataset) => datasets.push(dataset),
                        Err(e) => {
                            tracing::warn!("Failed to convert DICOM JSON to dataset: {}", e);
                        }
                    }
                }
                Ok(datasets)
            }
            UnwrappedContent::Binary(bytes) => {
                tracing::debug!(
                    "response_to_datasets: unwrapped binary DICOM data ({} bytes)",
                    bytes.len()
                );
                let dataset = Self::binary_to_dataset(&bytes)?;
                Ok(vec![dataset])
            }
            UnwrappedContent::FolderPath(folder_path) => {
                tracing::debug!(
                    "response_to_datasets: reading DICOM files from folder: {:?}",
                    folder_path
                );
                Self::folder_to_datasets(&folder_path)
            }
            UnwrappedContent::Empty => {
                tracing::debug!("response_to_datasets: no content to convert");
                Ok(vec![])
            }
        }
    }

    /// Convert clean DICOM JSON to a DatasetStream
    ///
    /// This function expects already-cleaned DICOM JSON (8-character hex tag keys).
    /// Use envelope_unwrapper to extract clean JSON before calling this.
    fn dicom_json_to_dataset(json: &serde_json::Value) -> DimseResult<DatasetStream> {
        let dicom_obj = tool::json_value_to_identifier(json).map_err(|e| {
            tracing::error!(
                "dicom_json_to_dataset: conversion failed: {}. Keys: {:?}",
                e,
                json.as_object().map(|o| o.keys().collect::<Vec<_>>())
            );
            DimseError::operation_failed(format!("Failed to convert JSON to DICOM object: {}", e))
        })?;
        Ok(DatasetStream::from_object(dicom_obj))
    }

    /// Convert binary DICOM Part10 data to a DatasetStream
    fn binary_to_dataset(bytes: &[u8]) -> DimseResult<DatasetStream> {
        let file_obj = dicom_object::from_reader(bytes).map_err(|e| {
            DimseError::operation_failed(format!("Failed to parse DICOM from binary: {}", e))
        })?;
        Ok(DatasetStream::from_object(file_obj.into_inner()))
    }

    /// Read all DICOM files from a folder and convert to DatasetStreams
    ///
    /// This is used for C-GET/C-MOVE results where files are stored on disk.
    fn folder_to_datasets(folder_path: &std::path::Path) -> DimseResult<Vec<DatasetStream>> {
        let mut datasets = Vec::new();

        let entries = std::fs::read_dir(folder_path).map_err(|e| {
            DimseError::operation_failed(format!("Failed to read folder {:?}: {}", folder_path, e))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Check for .dcm extension or try to parse anyway
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "dcm" && ext != "dicom" {
                // Try to detect DICOM by reading magic bytes
                if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.len() < 132 || &bytes[128..132] != b"DICM" {
                        tracing::debug!("Skipping non-DICOM file: {:?}", path);
                        continue;
                    }
                } else {
                    continue;
                }
            }

            tracing::debug!("Reading DICOM file: {:?}", path);

            // Create DatasetStream::File variant pointing to the file
            // This allows the SCP to send the file directly without re-parsing
            // Note: delete_on_drop is false since we don't own these files
            let dataset = DatasetStream::from_file(path.clone(), false);
            datasets.push(dataset);
        }

        tracing::info!(
            "folder_to_datasets: loaded {} DICOM files from {:?}",
            datasets.len(),
            folder_path
        );
        Ok(datasets)
    }
}

/// Start a pipeline metadata map with the association peer's address.
///
/// The IP goes under `remote_addr` (no port) and the port under `remote_port`,
/// matching the HTTP adapter. The policies middleware reads `remote_addr` for
/// IP allow and deny rules and for rate-limit keys, so DIMSE requests without
/// it would share one "unknown" client.
fn peer_meta(peer_addr: SocketAddr) -> HashMap<String, String> {
    HashMap::from([
        ("remote_addr".to_string(), peer_addr.ip().to_string()),
        ("remote_port".to_string(), peer_addr.port().to_string()),
    ])
}

/// Compile the IP rules of every `policies` middleware on `pipeline`'s
/// incoming chain, the same chain C-FIND, C-GET, C-MOVE and C-STORE pass
/// through.
///
/// Returns each middleware name with its rules. Fails when the pipeline or a
/// policy cannot be read.
fn pipeline_ip_access_rules(
    config: &crate::config::config::Config,
    pipeline: &str,
) -> std::result::Result<Vec<(String, IpAccessRules)>, String> {
    use crate::models::middleware::types::policies::parse_config;

    let pipeline_cfg = config
        .pipelines
        .get(pipeline)
        .ok_or_else(|| format!("pipeline '{}' not found", pipeline))?;

    let no_options = HashMap::new();
    let mut all_rules = Vec::new();
    for name in pipeline_cfg.middleware.left_chain() {
        // A name with no instance block is resolved as a middleware type with
        // no options, as `build_middleware_instances_for_pipeline` does.
        let (middleware_type, options) = match config.middleware.get(&name) {
            Some(instance) => (instance.middleware_type.as_str(), &instance.options),
            None => (name.as_str(), &no_options),
        };
        if !middleware_type.eq_ignore_ascii_case("policies") {
            continue;
        }
        let rules = parse_config(options, &config.policies, &config.rules)
            .and_then(|parsed| IpAccessRules::from_config(&parsed))
            .map_err(|e| format!("middleware '{}' cannot be read: {}", name, e))?;
        all_rules.push((name, rules));
    }
    Ok(all_rules)
}

/// Whether any `policies` middleware on `pipeline`'s incoming chain has an
/// enabled `ip_allow` or `ip_deny` rule.
///
/// Fails when the pipeline or a policy cannot be read, so callers can fail
/// closed.
pub(crate) fn pipeline_has_ip_rules(
    config: &crate::config::config::Config,
    pipeline: &str,
) -> std::result::Result<bool, String> {
    Ok(pipeline_ip_access_rules(config, pipeline)?
        .iter()
        .any(|(_, rules)| !rules.is_empty()))
}

/// Decide whether a peer at `ip` may open a DIMSE association for `pipeline`.
///
/// Applies the `ip_allow` and `ip_deny` rules of every `policies` middleware
/// on the pipeline's incoming chain. Other rule types are left to the
/// requests. A pipeline with no `policies` middleware admits every peer, as
/// it admits every request. Returns the reason when the peer is refused,
/// including when the pipeline or a policy cannot be read, because such a
/// pipeline would fail every request anyway.
fn peer_admission(
    config: &crate::config::config::Config,
    pipeline: &str,
    ip: IpAddr,
) -> std::result::Result<(), String> {
    for (name, rules) in pipeline_ip_access_rules(config, pipeline)? {
        rules
            .check_peer(ip)
            .map_err(|reason| format!("middleware '{}': {}", name, reason))?;
    }
    Ok(())
}

#[async_trait]
impl dimse::scp::QueryProvider for PipelineQueryProvider {
    async fn find(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
        max_results: u32,
    ) -> DimseResult<BoxDatasetStream> {
        let mut meta = peer_meta(peer_addr);
        meta.insert("query_level".into(), format!("{}", query_level));
        meta.insert("max_results".into(), max_results.to_string());

        // Build wrapper for pipeline
        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        // Check if pipeline supports streaming
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        let operation = Operation::Query;
        let command = Command::new(operation).with_source("dimse_adapter");
        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-FIND");
            self.run_streaming(&config, command, body, meta).await
        } else {
            let response_envelope = self.run(&config, command, body, meta, None).await?;
            let datasets = Self::run_result_to_datasets(response_envelope, operation).await?;
            Ok(dataset_stream_from_vec(datasets))
        }
    }

    async fn locate(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
        let mut meta = peer_meta(peer_addr);
        meta.insert("query_level".into(), format!("{}", query_level));

        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let response_envelope = self
            .run(
                &config,
                Command::new(Operation::Move).with_source("dimse_adapter"),
                body,
                meta,
                None,
            )
            .await?;

        let datasets = Self::run_result_to_datasets(response_envelope, Operation::Move).await?;
        Ok(dataset_stream_from_vec(datasets))
    }

    async fn get(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<RetrieveStream> {
        let mut meta = peer_meta(peer_addr);
        meta.insert("query_level".into(), format!("{}", query_level));

        let cmd = tool::model::CommandMeta {
            message_id: Some(1),
            sop_class_uid: None,
            priority: Some("MEDIUM".into()),
            direction: Some("REQUEST".into()),
        };
        let identifier = self.build_identifier_json(parameters);
        let qmeta = self.build_query_metadata(parameters);
        let wrapper = tool::model::Wrapper {
            command: Some(cmd),
            identifier,
            query_metadata: Some(qmeta),
        };
        let body = serde_json::to_value(&wrapper)
            .map_err(|e| DimseError::operation_failed(format!("Wrapper serialize: {}", e)))?;

        // Check if pipeline supports streaming
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;

        let operation = Operation::Retrieve;
        let command = Command::new(operation).with_source("dimse_adapter");
        if PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config) {
            tracing::info!("Using streaming pipeline for C-GET");
            // The bridge does not know the instance count up front, so the SCP
            // sends no Pending responses for this retrieval.
            Ok(RetrieveStream::new(
                self.run_streaming(&config, command, body, meta).await?,
            ))
        } else {
            let response_envelope = self.run(&config, command, body, meta, None).await?;
            let datasets = Self::run_result_to_datasets(response_envelope, operation).await?;
            Ok(RetrieveStream::from_vec(datasets))
        }
    }

    /// Apply the pipeline's IP rules to the peer before the association is
    /// accepted, so a denied peer cannot even get a C-ECHO reply. The config
    /// is read per association, so a reload applies to new associations.
    async fn admit_peer(&self, peer_addr: SocketAddr) -> DimseResult<PeerAdmission> {
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        Ok(self.admission(&config, peer_addr.ip()))
    }

    async fn store(&self, peer_addr: SocketAddr, dataset: DatasetStream) -> DimseResult<()> {
        // Resolve storage directory using global storage backend if available, otherwise fallback
        let target_dir = if let Some(storage) = get_storage() {
            // For filesystem backend, ensure_dir_str returns the full path
            storage
                .ensure_dir_str("dimse")
                .map_err(|e| DimseError::operation_failed(format!("Storage error: {}", e)))?
        } else {
            // Fallback for legacy behavior or when no storage backend configured
            let dir = get_current_store_dir().unwrap_or_else(|| PathBuf::from("./tmp/dimse"));
            if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                return Err(DimseError::operation_failed(format!(
                    "ensure store dir: {}",
                    e
                )));
            }
            dir
        };

        // Convert first instance to DICOM JSON for pipeline metadata
        // This allows middleware and storage backends to access DICOM tags
        let dicom_json = match dataset.to_object().await {
            Ok(obj) => match tool::identifier_to_json_value(&obj) {
                Ok(json) => Some(json),
                Err(e) => {
                    tracing::warn!("Failed to convert DICOM to JSON: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to parse DICOM object: {}", e);
                None
            }
        };

        let stored_copy = StoredInstanceCopy::new(
            dataset
                .to_temp_file(&target_dir)
                .await
                .map_err(|e| DimseError::operation_failed(format!("store dataset: {}", e)))?,
        );
        let temp_file = stored_copy.path();

        // Build metadata from DICOM tags for path resolution
        let mut meta = peer_meta(peer_addr);

        // Extract key UIDs from dataset metadata for path templating
        let ds_meta = dataset.metadata();
        if let Some(ref uid) = ds_meta.study_instance_uid {
            meta.insert("StudyInstanceUID".into(), uid.clone());
        }
        if let Some(ref uid) = ds_meta.series_instance_uid {
            meta.insert("SeriesInstanceUID".into(), uid.clone());
        }
        if let Some(ref uid) = ds_meta.sop_instance_uid {
            meta.insert("SOPInstanceUID".into(), uid.clone());
        }
        if let Some(ref id) = ds_meta.patient_id {
            meta.insert("PatientID".into(), id.clone());
        }

        // Execute pipeline and check for DICOM status in response.
        // Dispatch to the streaming executor when the pipeline's backend
        // requires it. This mirrors the pattern used by `find` and `get`.
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        let pipeline_cfg = config.pipelines.get(&self.pipeline).ok_or_else(|| {
            DimseError::operation_failed(format!("Pipeline '{}' not found", self.pipeline))
        })?;
        let streaming = PipelineExecutor::is_streaming_pipeline(pipeline_cfg, &config);

        // The bytes are read back from the file just written so the inline
        // copy and the file are identical.
        let inline_instance = if Self::store_needs_inline_instance(pipeline_cfg, &config) {
            let part10 = tokio::fs::read(temp_file).await.map_err(|e| {
                DimseError::operation_failed(format!(
                    "read stored instance {}: {}",
                    temp_file.display(),
                    e
                ))
            })?;
            Some(InlineInstance::from_part10(&part10)?)
        } else {
            None
        };

        let body = Self::build_store_body(
            ds_meta.sop_class_uid.as_deref(),
            dicom_json,
            temp_file,
            inline_instance,
        );

        let command = Command::new(Operation::Store).with_source("dimse_adapter");
        let response = if streaming {
            tracing::info!("Using streaming pipeline for C-STORE");
            self.run_streaming_response(&config, command, body, meta)
                .await?
        } else {
            let local_instance = Self::store_attaches_local_instance(pipeline_cfg, &config)
                .then(|| LocalInstanceFile::new(temp_file.to_path_buf()));
            self.run(&config, command, body, meta, local_instance)
                .await?
                .ok_or_else(|| PipelineFailure::NoMatches.into_error())?
        };

        Self::check_store_response(&response)?;
        // A dicom_scp pipeline with no backend uses this copy as its store
        // (AURA-2083, tests/dicom/cstore_filesystem.rs). `pipeline_cfg` is from
        // the snapshot the pipeline ran with, so a reload during the store
        // cannot make this decision disagree with what ran.
        if pipeline_cfg.backends.is_empty() {
            stored_copy.keep();
        }
        Ok(())
    }
}

/// DIMSE operation types used as routing keys for the multiplexing provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimseOp {
    Find,
    Move,
    Get,
    Store,
}

/// A query provider that multiplexes multiple pipelines on a single SCP listener.
///
/// When several `dicom_scp` endpoints share the same `(bind_addr, port)`, each
/// endpoint typically enables a different subset of DIMSE operations (e.g., one
/// pipeline handles C-FIND while another handles C-STORE). This provider routes
/// each operation to the pipeline that registered for it.
///
/// If multiple endpoints register for the same operation, the first registration
/// wins (a warning is logged during construction).
pub struct MultiplexingQueryProvider {
    /// Maps each DIMSE operation to the pipeline provider that handles it.
    op_routes: HashMap<DimseOp, Arc<PipelineQueryProvider>>,
    /// Fallback provider used when no specific route is registered for an
    /// operation. This is the first provider added, providing backward
    /// compatibility with single-endpoint configurations.
    fallback: Arc<PipelineQueryProvider>,
    /// Every provider on this listener, in route order. Admission checks all
    /// of them, including an endpoint that enables only C-ECHO and so owns no
    /// operation route.
    all: Vec<Arc<PipelineQueryProvider>>,
}

impl MultiplexingQueryProvider {
    /// Build a multiplexing provider from a list of `(pipeline, endpoint, enabled_ops)`.
    ///
    /// `enabled_ops` is derived from the endpoint's `DimseConfig` feature flags
    /// (enable_find, enable_store, etc.). The first endpoint to claim an
    /// operation wins; duplicates are logged as warnings.
    pub fn new(routes: Vec<(String, String, Vec<DimseOp>)>) -> Self {
        assert!(
            !routes.is_empty(),
            "MultiplexingQueryProvider requires at least one route"
        );

        let mut op_routes: HashMap<DimseOp, Arc<PipelineQueryProvider>> = HashMap::new();
        let mut fallback: Option<Arc<PipelineQueryProvider>> = None;
        let mut all: Vec<Arc<PipelineQueryProvider>> = Vec::new();

        for (pipeline, endpoint, ops) in routes {
            let provider = Arc::new(PipelineQueryProvider::new(
                pipeline.clone(),
                endpoint.clone(),
            ));

            if fallback.is_none() {
                fallback = Some(Arc::clone(&provider));
            }
            all.push(Arc::clone(&provider));

            for op in ops {
                if let Some(existing) = op_routes.get(&op) {
                    tracing::warn!(
                        "DIMSE operation {:?} already registered by pipeline '{}' endpoint '{}', \
                         ignoring duplicate from pipeline '{}' endpoint '{}'",
                        op,
                        existing.pipeline,
                        existing.endpoint,
                        pipeline,
                        endpoint,
                    );
                } else {
                    tracing::debug!(
                        "DIMSE operation {:?} routed to pipeline '{}' endpoint '{}'",
                        op,
                        pipeline,
                        endpoint,
                    );
                    op_routes.insert(op, Arc::clone(&provider));
                }
            }
        }

        Self {
            op_routes,
            fallback: fallback.expect("at least one route"),
            all,
        }
    }

    /// Admit the peer when any pipeline on this listener admits it, judged
    /// against `config`.
    fn admission(&self, config: &crate::config::config::Config, ip: IpAddr) -> PeerAdmission {
        let mut reasons = Vec::new();
        for provider in &self.all {
            match provider.admission(config, ip) {
                PeerAdmission::Admit => return PeerAdmission::Admit,
                PeerAdmission::Reject { reason } => reasons.push(reason),
            }
        }
        PeerAdmission::Reject {
            reason: reasons.join("; "),
        }
    }

    /// Get the provider for a given operation, falling back to the default.
    fn provider_for(&self, op: DimseOp) -> &PipelineQueryProvider {
        self.op_routes
            .get(&op)
            .map(|p| p.as_ref())
            .unwrap_or(self.fallback.as_ref())
    }
}

#[async_trait]
impl dimse::scp::QueryProvider for MultiplexingQueryProvider {
    async fn find(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
        max_results: u32,
    ) -> DimseResult<BoxDatasetStream> {
        self.provider_for(DimseOp::Find)
            .find(peer_addr, query_level, parameters, max_results)
            .await
    }

    async fn locate(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<BoxDatasetStream> {
        self.provider_for(DimseOp::Move)
            .locate(peer_addr, query_level, parameters)
            .await
    }

    async fn get(
        &self,
        peer_addr: SocketAddr,
        query_level: QueryLevel,
        parameters: &HashMap<String, String>,
    ) -> DimseResult<RetrieveStream> {
        self.provider_for(DimseOp::Get)
            .get(peer_addr, query_level, parameters)
            .await
    }

    async fn store(&self, peer_addr: SocketAddr, dataset: DatasetStream) -> DimseResult<()> {
        self.provider_for(DimseOp::Store)
            .store(peer_addr, dataset)
            .await
    }

    /// Admit the peer when any pipeline on this listener admits it.
    ///
    /// Requiring all of them would lock a client of one pipeline out because
    /// of another pipeline's rules. Each request is still checked by the
    /// pipeline that handles its operation.
    async fn admit_peer(&self, peer_addr: SocketAddr) -> DimseResult<PeerAdmission> {
        let config =
            get_config().ok_or_else(|| DimseError::operation_failed("Global config not set"))?;
        Ok(self.admission(&config, peer_addr.ip()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================
    // Peer admission
    // ============================================

    /// Three pipelines: `office` allows only 192.168.0.0/16, `no_ten` denies
    /// 10.0.0.0/8, and `open` has no policies middleware.
    fn admission_config() -> crate::config::config::Config {
        let toml = r#"
[proxy]
id = "peer-admission-test"

[pipelines.office]
endpoints = ["scp"]
middleware = ["office_only"]

[pipelines.no_ten]
endpoints = ["scp"]
middleware = ["block_ten"]

[pipelines.open]
endpoints = ["scp"]
middleware = []

[pipelines.broken]
endpoints = ["scp"]
middleware = ["bad_policy"]

[endpoints.scp]
service = "dicom_scp"

[middleware.office_only]
type = "policies"
[middleware.office_only.options]
policies = ["office"]

[middleware.block_ten]
type = "policies"
[middleware.block_ten.options]
policies = ["no_ten"]

[middleware.bad_policy]
type = "policies"
[middleware.bad_policy.options]
policies = ["does_not_exist"]

[policies.office]
id = "office"
rules = ["office_networks"]

[policies.no_ten]
id = "no_ten"
rules = ["ten_net", "anyone"]

[rules.office_networks]
id = "office_networks"
type = "ip_allow"
[rules.office_networks.options]
ip_addresses = ["192.168.0.0/16"]

[rules.ten_net]
id = "ten_net"
type = "ip_deny"
[rules.ten_net.options]
ip_addresses = ["10.0.0.0/8"]

[rules.anyone]
id = "anyone"
type = "allow_all"
"#;
        toml::from_str(toml).expect("TOML parse error")
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn is_rejected(admission: &PeerAdmission) -> bool {
        matches!(admission, PeerAdmission::Reject { .. })
    }

    #[test]
    fn peer_admission_applies_the_pipeline_allowlist() {
        let config = admission_config();
        assert!(peer_admission(&config, "office", ip("192.168.1.20")).is_ok());
        assert!(peer_admission(&config, "office", ip("10.0.0.1")).is_err());
    }

    #[test]
    fn peer_admission_applies_ip_deny_rules() {
        let config = admission_config();
        let reason = peer_admission(&config, "no_ten", ip("10.9.9.9")).unwrap_err();
        assert!(
            reason.contains("ten_net"),
            "reason should name the rule: {}",
            reason
        );
        assert!(peer_admission(&config, "no_ten", ip("172.16.0.1")).is_ok());
    }

    #[test]
    fn peer_admission_admits_any_peer_without_a_policies_middleware() {
        let config = admission_config();
        assert!(peer_admission(&config, "open", ip("10.0.0.1")).is_ok());
    }

    #[test]
    fn peer_admission_fails_closed_for_an_unknown_pipeline() {
        let config = admission_config();
        let reason = peer_admission(&config, "missing", ip("192.168.1.20")).unwrap_err();
        assert!(reason.contains("missing"), "{}", reason);
    }

    #[test]
    fn peer_admission_fails_closed_for_an_unreadable_policy() {
        let config = admission_config();
        let reason = peer_admission(&config, "broken", ip("192.168.1.20")).unwrap_err();
        assert!(reason.contains("bad_policy"), "{}", reason);
    }

    #[test]
    fn pipeline_provider_rejection_names_the_pipeline() {
        let config = admission_config();
        let provider = PipelineQueryProvider::new("office", "scp");
        match provider.admission(&config, ip("10.0.0.1")) {
            PeerAdmission::Reject { reason } => {
                assert!(reason.contains("pipeline 'office'"), "{}", reason)
            }
            PeerAdmission::Admit => panic!("10.0.0.1 is outside the office allowlist"),
        }
    }

    /// A peer that one pipeline on the listener admits may connect. Its
    /// requests are still checked by the pipeline for each operation.
    #[test]
    fn multiplexing_provider_admits_when_any_pipeline_admits() {
        let config = admission_config();
        let provider = MultiplexingQueryProvider::new(vec![
            ("office".into(), "scp".into(), vec![DimseOp::Find]),
            ("no_ten".into(), "scp".into(), vec![DimseOp::Store]),
        ]);

        // Outside the office allowlist, but no_ten admits it.
        assert_eq!(
            provider.admission(&config, ip("172.16.0.1")),
            PeerAdmission::Admit
        );
        // Inside the office allowlist, though no_ten denies 10.0.0.0/8 only.
        assert_eq!(
            provider.admission(&config, ip("192.168.1.20")),
            PeerAdmission::Admit
        );
    }

    /// An endpoint that enables only C-ECHO owns no operation route. Its
    /// pipeline still counts at admission.
    #[test]
    fn multiplexing_provider_checks_an_echo_only_pipeline() {
        let config = admission_config();
        // `broken` owns C-FIND and rejects every peer, so only the echo-only
        // `office` pipeline can admit one.
        let provider = MultiplexingQueryProvider::new(vec![
            ("broken".into(), "scp".into(), vec![DimseOp::Find]),
            ("office".into(), "scp".into(), vec![]),
        ]);
        assert_eq!(
            provider.admission(&config, ip("192.168.1.20")),
            PeerAdmission::Admit
        );
        assert!(is_rejected(&provider.admission(&config, ip("172.16.0.1"))));
    }

    /// Only the incoming chain gates a request, so only it gates a peer.
    #[test]
    fn peer_admission_ignores_policies_on_the_outgoing_chain() {
        let mut config = admission_config();
        let toml = r#"
endpoints = ["scp"]
[middleware]
left = []
right = ["office_only"]
"#;
        let split: crate::models::pipelines::config::Pipeline =
            toml::from_str(toml).expect("split pipeline TOML");
        config.pipelines.insert("split".to_string(), split);

        assert!(peer_admission(&config, "split", ip("10.0.0.1")).is_ok());
    }

    #[test]
    fn multiplexing_provider_rejects_when_every_pipeline_rejects() {
        let config = admission_config();
        let provider = MultiplexingQueryProvider::new(vec![
            ("office".into(), "scp".into(), vec![DimseOp::Find]),
            ("no_ten".into(), "scp".into(), vec![DimseOp::Store]),
        ]);

        let admission = provider.admission(&config, ip("10.0.0.1"));
        assert!(is_rejected(&admission));
        if let PeerAdmission::Reject { reason } = admission {
            assert!(
                reason.contains("'office'") && reason.contains("'no_ten'"),
                "{}",
                reason
            );
        }
    }

    #[test]
    fn test_query_provider_creation() {
        let provider = PipelineQueryProvider::new("test_pipeline", "test_endpoint");
        assert_eq!(provider.pipeline, "test_pipeline");
        assert_eq!(provider.endpoint, "test_endpoint");
    }

    #[test]
    fn test_build_identifier_json_empty() {
        let provider = PipelineQueryProvider::new("p", "e");
        let params = HashMap::new();
        let json = provider.build_identifier_json(&params);
        assert!(json.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_build_identifier_json_patient_name() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100010".to_string(), "Doe^John".to_string());

        let json = provider.build_identifier_json(&params);
        let obj = json.as_object().unwrap();

        assert!(obj.contains_key("00100010"));
        let entry = &obj["00100010"];
        assert_eq!(entry["vr"], "PN");
        assert_eq!(entry["Value"][0]["Alphabetic"], "Doe^John");
    }

    #[test]
    fn test_build_query_metadata_exact_match() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100020".to_string(), "12345".to_string());

        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00100020").unwrap().match_type.as_deref(),
            Some("EXACT")
        );
    }

    /// The policies middleware parses `remote_addr` as a bare IP, so the port
    /// must not be part of it.
    #[test]
    fn test_peer_meta_records_ipv4_without_port() {
        let meta = peer_meta("203.0.113.42:51234".parse().unwrap());
        assert_eq!(
            meta.get("remote_addr").map(String::as_str),
            Some("203.0.113.42")
        );
        assert_eq!(meta.get("remote_port").map(String::as_str), Some("51234"));
    }

    #[test]
    fn test_peer_meta_records_ipv6_without_brackets() {
        let meta = peer_meta("[2001:db8::1]:51234".parse().unwrap());
        assert_eq!(
            meta.get("remote_addr").map(String::as_str),
            Some("2001:db8::1")
        );
        assert_eq!(meta.get("remote_port").map(String::as_str), Some("51234"));
    }

    #[test]
    fn test_build_query_metadata_wildcard() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00100010".to_string(), "Doe*".to_string());

        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00100010").unwrap().match_type.as_deref(),
            Some("WILDCARD")
        );
    }

    #[test]
    fn test_build_query_metadata_return_key() {
        let provider = PipelineQueryProvider::new("p", "e");
        let mut params = HashMap::new();
        params.insert("00080020".to_string(), "".to_string());

        let meta = provider.build_query_metadata(&params);
        assert_eq!(
            meta.0.get("00080020").unwrap().match_type.as_deref(),
            Some("RETURN_KEY")
        );
    }

    // Note: test_store_dir_functions removed - uses global static that conflicts with parallel test execution

    #[test]
    fn test_run_streaming_command_carries_correct_operation() {
        // Verify that the Command built at each call site has the expected operation,
        // mirroring what run_streaming() receives in practice.
        let find_cmd = Command::new(Operation::Query).with_source("dimse_adapter");
        assert_eq!(find_cmd.operation, Operation::Query);
        assert_eq!(find_cmd.source.as_deref(), Some("dimse_adapter"));

        let get_cmd = Command::new(Operation::Retrieve).with_source("dimse_adapter");
        assert_eq!(get_cmd.operation, Operation::Retrieve);
        assert_eq!(get_cmd.source.as_deref(), Some("dimse_adapter"));
    }

    // ── MultiplexingQueryProvider tests ──────────────────────────────────────

    #[test]
    fn test_multiplexing_provider_routes_to_correct_pipeline() {
        let routes = vec![
            (
                "find_pipeline".into(),
                "find_ep".into(),
                vec![DimseOp::Find],
            ),
            (
                "store_pipeline".into(),
                "store_ep".into(),
                vec![DimseOp::Store],
            ),
        ];
        let mux = MultiplexingQueryProvider::new(routes);

        // Find should route to find_pipeline
        let find_provider = mux.provider_for(DimseOp::Find);
        assert_eq!(find_provider.pipeline, "find_pipeline");
        assert_eq!(find_provider.endpoint, "find_ep");

        // Store should route to store_pipeline
        let store_provider = mux.provider_for(DimseOp::Store);
        assert_eq!(store_provider.pipeline, "store_pipeline");
        assert_eq!(store_provider.endpoint, "store_ep");
    }

    #[test]
    fn test_multiplexing_provider_fallback_for_unregistered_op() {
        let routes = vec![(
            "only_pipeline".into(),
            "only_ep".into(),
            vec![DimseOp::Find],
        )];
        let mux = MultiplexingQueryProvider::new(routes);

        // Get is not registered, should fall back to the first provider
        let get_provider = mux.provider_for(DimseOp::Get);
        assert_eq!(get_provider.pipeline, "only_pipeline");
    }

    #[test]
    fn test_multiplexing_provider_first_registration_wins() {
        let routes = vec![
            (
                "first_pipeline".into(),
                "first_ep".into(),
                vec![DimseOp::Find],
            ),
            (
                "second_pipeline".into(),
                "second_ep".into(),
                vec![DimseOp::Find], // duplicate
            ),
        ];
        let mux = MultiplexingQueryProvider::new(routes);

        let find_provider = mux.provider_for(DimseOp::Find);
        assert_eq!(find_provider.pipeline, "first_pipeline");
    }

    #[test]
    fn test_multiplexing_provider_single_pipeline_all_ops() {
        let routes = vec![(
            "all_in_one".into(),
            "ep".into(),
            vec![DimseOp::Find, DimseOp::Store, DimseOp::Get, DimseOp::Move],
        )];
        let mux = MultiplexingQueryProvider::new(routes);

        for op in [DimseOp::Find, DimseOp::Store, DimseOp::Get, DimseOp::Move] {
            assert_eq!(mux.provider_for(op).pipeline, "all_in_one");
        }
    }

    // ── C-STORE pipeline body ────────────────────────────────────────────────

    fn sample_instance_path() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm")
    }

    #[test]
    fn test_inline_instance_states_the_transfer_syntax_of_the_file() {
        use base64::Engine;

        let path = sample_instance_path();
        let part10 = std::fs::read(&path).expect("read sample instance");
        let source = dicom_object::open_file(&path).expect("open sample instance");

        let instance = InlineInstance::from_part10(&part10).expect("inline instance");

        assert_eq!(
            instance.transfer_syntax_uid,
            source.meta().transfer_syntax()
        );
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&instance.part10_b64)
            .expect("part10_b64 is base64");
        assert_eq!(decoded, part10);
    }

    #[test]
    fn test_inline_instance_reads_a_transfer_syntax_that_is_not_the_default() {
        // Explicit VR Little Endian is what `to_part10_bytes` writes when no
        // transfer syntax is recorded, so it cannot show the value was read.
        let implicit_vr_le = "1.2.840.10008.1.2";
        let source = dicom_object::open_file(sample_instance_path()).expect("open sample instance");
        let mut dataset = DatasetStream::from_object(source.into_inner());
        dataset.metadata_mut().transfer_syntax = Some(implicit_vr_le.to_string());
        let part10 = dataset.to_part10_bytes().expect("encode Part 10");

        let instance = InlineInstance::from_part10(&part10).expect("inline instance");

        assert_eq!(instance.transfer_syntax_uid, implicit_vr_le);
    }

    #[test]
    fn test_inline_instance_rejects_bytes_that_are_not_part10() {
        for bytes in [vec![0u8; 16], vec![0u8; 200]] {
            let err = InlineInstance::from_part10(&bytes)
                .err()
                .expect("bytes without a file meta group are rejected");
            assert!(
                err.to_string().contains("stored instance"),
                "unexpected error: {}",
                err
            );
        }
    }

    /// How the pipeline under test refers to its one middleware.
    enum MiddlewareRef<'a> {
        None,
        /// `middleware = ["mw"]` with a `[middleware.mw]` table of this type.
        Table(&'a str),
        /// `middleware = ["<type>"]` with no table.
        BareType(&'a str),
    }

    fn inline_config(
        backend_service: &str,
        middleware: MiddlewareRef,
    ) -> crate::config::config::Config {
        let (middleware_list, middleware_block) = match middleware {
            MiddlewareRef::Table(kind) => (
                "\"mw\"".to_string(),
                format!("[middleware.mw]\ntype = \"{}\"\n", kind),
            ),
            MiddlewareRef::BareType(kind) => (format!("\"{}\"", kind), String::new()),
            MiddlewareRef::None => (String::new(), String::new()),
        };
        let backend_options = match backend_service {
            "dicom_scu" | "dicom" | "dicom_scu_streaming" => {
                "aet = \"PACS\"\nhost = \"127.0.0.1\"\nport = 4242"
            }
            _ => "base_url = \"http://127.0.0.1:9\"",
        };
        let toml = format!(
            r#"
[proxy]
id = "store-inline-test"

[network.default]
enable_wireguard = false
interface = "wg0"

[network.default.tcp_config]
bind_address = "127.0.0.1"
bind_port = 11112

[pipelines.store]
networks = ["default"]
endpoints = ["scp"]
backends = ["out"]
middleware = [{middleware_list}]

[endpoints.scp]
service = "dicom_scp"
[endpoints.scp.options]
local_aet = "HARMONY"
enable_store = true

[backends.out]
service = "{backend_service}"
[backends.out.options]
{backend_options}

{middleware_block}

[services.dicom_scp]
module = ""

[services.{backend_service}]
module = ""
"#
        );
        toml::from_str(&toml).expect("TOML parse error")
    }

    #[test]
    fn test_instance_goes_inline_only_where_a_consumer_needs_it() {
        use MiddlewareRef::{BareType, None, Table};
        let needs = |backend: &str, middleware: MiddlewareRef| {
            let config = inline_config(backend, middleware);
            let pipeline = config.pipelines.get("store").expect("pipeline present");
            PipelineQueryProvider::store_needs_inline_instance(pipeline, &config)
        };

        assert!(
            needs("dicom_scu", None),
            "single node: the backend takes the instance"
        );
        assert!(needs("dicom", None), "the legacy name of dicom_scu");
        assert!(
            needs("dicom_scu_streaming", None),
            "single node: the backend takes the instance"
        );
        assert!(
            needs("http_streaming_backend", Table("dicom_to_http")),
            "streaming bridge"
        );
        assert!(
            needs("http_streaming_backend", BareType("dicom_to_http")),
            "streaming bridge that names the built-in type with no table"
        );
        assert!(
            !needs("http", Table("dicom_to_http")),
            "the http backend cannot carry the bridge headers"
        );
        assert!(
            !needs("http_streaming_backend", Table("dicom_flatten")),
            "no bridge middleware"
        );
        assert!(!needs("http_streaming_backend", None));
        assert!(
            !needs("http", Table("dicom_to_dicomweb")),
            "reads the local file"
        );
        assert!(!needs("http", None));
    }

    #[test]
    fn test_part10_storage_backend_gets_the_local_file_not_an_inline_copy() {
        let needs = |content: Option<&str>| {
            let mut config = inline_config("storage", MiddlewareRef::None);
            let mut options = HashMap::new();
            options.insert("root".to_string(), serde_json::json!("./tmp/archive"));
            if let Some(content) = content {
                options.insert("content".to_string(), serde_json::json!(content));
            }
            config
                .backends
                .get_mut("out")
                .expect("backend present")
                .options = Some(options);
            let pipeline = config.pipelines.get("store").expect("pipeline present");
            (
                PipelineQueryProvider::store_attaches_local_instance(pipeline, &config),
                PipelineQueryProvider::store_needs_inline_instance(pipeline, &config),
            )
        };

        assert_eq!(
            needs(Some("dicom_part10")),
            (true, false),
            "the backend streams the file"
        );
        assert_eq!(
            needs(None),
            (false, false),
            "the backend writes the request body"
        );
    }

    #[test]
    fn test_store_body_carries_the_instance_only_when_inline() {
        let temp_file = std::path::Path::new("/var/harmony/dimse/instance.dcm");
        let dataset = serde_json::json!({ "00100020": { "vr": "LO", "Value": ["PID1"] } });

        let local = PipelineQueryProvider::build_store_body(
            Some("1.2.840.10008.5.1.4.1.1.2"),
            Some(dataset.clone()),
            temp_file,
            None,
        );
        assert_eq!(local["file"], "/var/harmony/dimse/instance.dcm");
        assert_eq!(local["dataset"], dataset);
        assert_eq!(
            local["command"]["sop_class_uid"],
            "1.2.840.10008.5.1.4.1.1.2"
        );
        assert!(local.get("part10_b64").is_none());
        assert!(local.get("body_b64").is_none());
        assert!(local.get("transfer_syntax_uid").is_none());

        let inline = PipelineQueryProvider::build_store_body(
            Some("1.2.840.10008.5.1.4.1.1.2"),
            Some(dataset),
            temp_file,
            Some(InlineInstance {
                part10_b64: "UGFydDEw".to_string(),
                transfer_syntax_uid: "1.2.840.10008.1.2.1".to_string(),
            }),
        );
        assert_eq!(inline["part10_b64"], "UGFydDEw");
        // `body_b64` would make the executor replace any rewritten JSON body.
        assert!(inline.get("body_b64").is_none());
        assert_eq!(inline["transfer_syntax_uid"], "1.2.840.10008.1.2.1");
        assert_eq!(inline["file"], "/var/harmony/dimse/instance.dcm");
    }

    // ── Streaming bridge hand-off ────────────────────────────────────────────
    //
    // Harmony B turns each retrieved DICOM file into a ResponseEnvelope and
    // writes the stream as NDJSON. Harmony A reads the NDJSON and converts each
    // envelope back into a dataset for the SCP caller. This test runs a real
    // sample instance through all of those stages in process.

    fn bridge_request_details() -> crate::models::envelope::envelope::RequestDetails {
        crate::models::envelope::envelope::RequestDetails {
            method: "POST".to_string(),
            uri: "/stream".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata: HashMap::new(),
            content_metadata: None,
        }
    }

    fn element_text(object: &dicom_object::InMemDicomObject, name: &str) -> String {
        object
            .element_by_name(name)
            .unwrap_or_else(|e| panic!("{} missing: {}", name, e))
            .to_str()
            .unwrap_or_else(|e| panic!("{} is not text: {}", name, e))
            .trim_end_matches(|c: char| c == '\0' || c == ' ')
            .to_string()
    }

    #[tokio::test]
    async fn test_streaming_bridge_round_trip_preserves_instance() {
        use crate::models::services::streaming::StreamMetadata;
        use crate::models::services::types::dicom_scu_streaming::DicomScuStreamingBackend;
        use crate::models::services::types::http_streaming::{HttpStreamingEndpoint, StreamFormat};
        use crate::models::services::types::http_streaming_backend::HttpStreamingBackend;
        use bytes::Bytes;
        use futures_util::StreamExt;

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let source_path = manifest_dir.join("samples/dicom/study_1/series_1/CT.1.1.dcm");

        // The producer deletes each file after reading it, so give it a copy.
        let tmp_root = manifest_dir.join("tmp");
        std::fs::create_dir_all(&tmp_root).expect("create ./tmp");
        let work_dir = tempfile::tempdir_in(&tmp_root).expect("create work dir");
        let copy_path = work_dir.path().join("instance.dcm");
        std::fs::copy(&source_path, &copy_path).expect("copy sample instance");

        // Harmony B: retrieved file → ResponseEnvelope
        let retrieved: std::pin::Pin<
            Box<dyn futures_util::Stream<Item = Result<DatasetStream, DimseError>> + Send>,
        > = Box::pin(futures_util::stream::iter(vec![Ok(
            DatasetStream::from_file(copy_path, false),
        )]));
        let envelopes = DicomScuStreamingBackend::convert_dimse_stream_to_envelopes(
            retrieved,
            bridge_request_details(),
            StreamMetadata::new("dimse-get"),
            None,
        );

        // Harmony B: envelopes → NDJSON response body
        let writer = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: None,
            enable_compression: None,
        };
        let response = writer
            .build_streaming_response(Box::pin(envelopes), StreamFormat::Ndjson, &HashMap::new())
            .expect("NDJSON response should build");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read NDJSON body");

        // Harmony A: NDJSON body → ResponseEnvelope. The body arrives in small
        // chunks, as an HTTP client receives it, so the line spans chunk boundaries.
        let chunks: Vec<Result<Bytes, reqwest::Error>> = body
            .chunks(256)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        let parsed: Vec<_> = HttpStreamingBackend::parse_ndjson_stream(
            futures_util::stream::iter(chunks),
            bridge_request_details(),
            None,
        )
        .collect()
        .await;
        assert_eq!(parsed.len(), 1, "one instance in, one envelope out");
        let envelope = parsed
            .into_iter()
            .next()
            .unwrap()
            .expect("NDJSON line should parse");

        // Harmony A SCP: envelope → dataset
        let datasets =
            PipelineQueryProvider::response_to_datasets(envelope, Operation::Retrieve, false)
                .await
                .expect("envelope should convert to a dataset");
        assert_eq!(datasets.len(), 1, "one envelope in, one dataset out");
        let received = datasets[0].to_object().await.expect("dataset should parse");

        let source = dicom_object::open_file(&source_path).expect("open sample instance");
        for name in [
            "SOPInstanceUID",
            "StudyInstanceUID",
            "SeriesInstanceUID",
            "SOPClassUID",
        ] {
            assert_eq!(
                element_text(&received, name),
                element_text(&source, name),
                "{} changed in the round trip",
                name
            );
        }
        let source_pixels = source
            .element_by_name("PixelData")
            .expect("source pixel data")
            .to_bytes()
            .expect("source pixel bytes");
        let received_pixels = received
            .element_by_name("PixelData")
            .expect("received pixel data")
            .to_bytes()
            .expect("received pixel bytes");
        assert_eq!(
            received_pixels, source_pixels,
            "pixel data changed in the round trip"
        );
    }

    /// A backend response after the executor's JSON round trip, as
    /// `response_to_datasets` receives it.
    fn backend_response_after_round_trip(
        status: u16,
        content_type: Option<&str>,
        body: &[u8],
    ) -> ResponseEnvelope<Vec<u8>> {
        let headers = content_type
            .map(|ct| HashMap::from([("content-type".to_string(), ct.to_string())]))
            .unwrap_or_default();
        ResponseEnvelope::from_backend_with_command(
            bridge_request_details(),
            status,
            headers,
            body.to_vec(),
            None,
            None,
        )
        .to_json()
        .expect("to_json failed")
        .to_bytes()
        .expect("to_bytes failed")
    }

    #[tokio::test]
    async fn response_to_datasets_fails_when_a_200_response_is_html() {
        // A login or error page served with 200 is not "no results".
        let envelope = backend_response_after_round_trip(
            200,
            Some("text/html"),
            b"<html><body>Please sign in</body></html>",
        );
        let error =
            match PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
                .await
            {
                Ok(datasets) => panic!("expected an error, got {} datasets", datasets.len()),
                Err(error) => error,
            };
        assert!(
            error
                .to_string()
                .contains("Failed to parse response as JSON"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn response_to_datasets_fails_when_a_json_labelled_body_is_not_json() {
        let envelope = backend_response_after_round_trip(
            200,
            Some("application/json"),
            b"<html>502 Bad Gateway</html>",
        );
        assert!(
            PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn response_to_datasets_returns_nothing_for_an_empty_200_response() {
        let envelope = backend_response_after_round_trip(200, Some("text/plain"), b"");
        let datasets =
            PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
                .await
                .expect("an empty body is an empty result");
        assert!(datasets.is_empty());
    }

    #[tokio::test]
    async fn response_to_datasets_reads_json_that_has_no_content_type() {
        let envelope = backend_response_after_round_trip(
            200,
            None,
            br#"[{"00100020":{"vr":"LO","Value":["PATIENT1"]}}]"#,
        );
        let datasets =
            PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
                .await
                .expect("JSON without a content type is still JSON");
        assert_eq!(datasets.len(), 1);
    }

    /// The status code a failed `response_to_datasets` carries for
    /// `operation`.
    async fn failure_status(envelope: ResponseEnvelope<Vec<u8>>, operation: Operation) -> u16 {
        match PipelineQueryProvider::response_to_datasets(envelope, operation, false).await {
            Ok(datasets) => panic!("expected a failure, got {} datasets", datasets.len()),
            Err(error) => error
                .status_code()
                .unwrap_or_else(|| panic!("the error carries no status code: {error}")),
        }
    }

    #[tokio::test]
    async fn response_to_datasets_fails_on_a_200_body_with_success_false() {
        // The unwrapper reads this body as zero matches, which the client
        // would take as Success.
        let body = br#"{"operation":"find","success":false,"error":"C-FIND failed"}"#;
        for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
            let envelope = backend_response_after_round_trip(200, Some("application/json"), body);
            assert_eq!(
                failure_status(envelope, operation).await,
                0x0110,
                "{operation}"
            );
        }
    }

    #[tokio::test]
    async fn response_to_datasets_keeps_the_body_reason_for_the_log() {
        let body = br#"{"success":false,"error":"connect to 10.0.0.5:104\nforged line"}"#;
        let envelope = backend_response_after_round_trip(200, Some("application/json"), body);
        let error = PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
            .await
            .err()
            .expect("a failure");
        assert_eq!(error.status_code(), Some(0x0110));
        let detail = error.to_string();
        assert!(detail.contains("10.0.0.5:104"), "{detail}");
        assert!(
            !detail.contains('\n'),
            "the reason must be escaped: {detail}"
        );
    }

    #[tokio::test]
    async fn response_to_datasets_accepts_a_body_with_success_true_or_no_success_key() {
        for body in [
            br#"{"success":true,"matches":[]}"#.as_slice(),
            br#"{"matches":[]}"#.as_slice(),
        ] {
            let envelope = backend_response_after_round_trip(200, Some("application/json"), body);
            let datasets =
                PipelineQueryProvider::response_to_datasets(envelope, Operation::Query, false)
                    .await
                    .expect("not a failure");
            assert!(datasets.is_empty());
        }
    }

    #[tokio::test]
    async fn response_to_datasets_maps_the_http_status_for_the_operation() {
        let body = br#"{"success":false,"error":"C-FIND failed"}"#;
        let cases = [
            (500, Operation::Query, 0x0110),
            (500, Operation::Retrieve, 0x0110),
            (503, Operation::Query, 0xA700),
            (503, Operation::Retrieve, 0xA701),
            (503, Operation::Move, 0xA701),
            (429, Operation::Query, 0xA700),
            (429, Operation::Retrieve, 0xA702),
            (413, Operation::Query, 0xA700),
            (413, Operation::Retrieve, 0xA701),
            (413, Operation::Move, 0xA701),
            // A query that matches nothing answers 2xx, so a 404 is a
            // routing error or a denial.
            (404, Operation::Query, 0xC000),
            (410, Operation::Query, 0xC000),
        ];
        for (http_status, operation, expected) in cases {
            let envelope =
                backend_response_after_round_trip(http_status, Some("application/json"), body);
            assert_eq!(
                failure_status(envelope, operation).await,
                expected,
                "HTTP {http_status} for {operation}"
            );
        }
    }

    #[test]
    fn test_find_gets_a700_in_place_of_the_retrieve_only_codes() {
        for code in [0xA701, 0xA702] {
            assert_eq!(
                failure_status_for_operation(Operation::Query, DimseStatus::Failure(code)),
                0xA700
            );
            for operation in [Operation::Retrieve, Operation::Move] {
                assert_eq!(
                    failure_status_for_operation(operation, DimseStatus::Failure(code)),
                    code
                );
            }
        }
    }

    #[test]
    fn test_move_destination_unknown_is_never_sent_for_a_pipeline_failure() {
        for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
            assert_eq!(
                failure_status_for_operation(operation, DimseStatus::Failure(0xA801)),
                0xC000
            );
        }
    }

    #[test]
    fn test_retrieves_get_a701_in_place_of_the_find_only_code() {
        for operation in [Operation::Retrieve, Operation::Move] {
            assert_eq!(
                failure_status_for_operation(operation, DimseStatus::Failure(0xA700)),
                0xA701
            );
        }
        assert_eq!(
            failure_status_for_operation(Operation::Query, DimseStatus::Failure(0xA700)),
            0xA700
        );
    }

    #[test]
    fn test_other_failure_codes_pass_through() {
        for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
            for code in [0x0110, 0x0124, 0xA900, 0xC000] {
                assert_eq!(
                    failure_status_for_operation(operation, DimseStatus::Failure(code)),
                    code
                );
            }
        }
    }

    #[test]
    fn test_a_non_failure_status_never_reaches_the_client_for_an_error() {
        for status in [
            DimseStatus::Success,
            DimseStatus::Pending,
            DimseStatus::Cancel,
            DimseStatus::Warning(0xB000),
        ] {
            assert_eq!(
                failure_status_for_operation(Operation::Query, status),
                0xC000
            );
        }
    }

    #[tokio::test]
    async fn response_to_datasets_gives_an_empty_retrieve_result_when_nothing_matched() {
        // PS3.4: a retrieve that matches nothing is Success with zero
        // sub-operations. dicom_scu answers 404 for a study it cannot find.
        let body = br#"{"operation":"move","success":false,"error":"Study not found"}"#;
        for http_status in [404, 410] {
            for operation in [Operation::Retrieve, Operation::Move] {
                let envelope =
                    backend_response_after_round_trip(http_status, Some("application/json"), body);
                let datasets =
                    PipelineQueryProvider::response_to_datasets(envelope, operation, false)
                        .await
                        .unwrap_or_else(|e| panic!("HTTP {http_status} for {operation}: {e}"));
                assert!(datasets.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn response_to_datasets_fails_on_a_streamed_404() {
        // Earlier envelopes already reached the client, so a later 404
        // cannot make the answer complete.
        for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
            let envelope = backend_response_after_round_trip(404, Some("application/json"), b"{}");
            let error = PipelineQueryProvider::response_to_datasets(envelope, operation, true)
                .await
                .err()
                .unwrap_or_else(|| panic!("a streamed 404 for {operation} must fail"));
            assert_eq!(error.status_code(), Some(0xC000), "{operation}");
        }
    }

    #[tokio::test]
    async fn run_result_with_no_matches_has_no_datasets() {
        let datasets = PipelineQueryProvider::run_result_to_datasets(None, Operation::Query)
            .await
            .expect("no matches is not a failure");
        assert!(datasets.is_empty());
    }

    #[test]
    fn test_object_not_found_is_an_empty_result_for_find_get_and_move() {
        let error = PipelineError::MiddlewareError(Box::new(DicomObjectNotFound(
            "C-GET matched no instances".to_string(),
        )));
        for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
            assert!(
                matches!(
                    pipeline_failure(operation, &error),
                    PipelineFailure::NoMatches
                ),
                "{operation}"
            );
        }
    }

    #[test]
    fn test_error_text_never_makes_an_empty_result() {
        // Text can hold instance counts, ports and remote status codes. None
        // of it may turn a failure into Success.
        let errors = [
            PipelineError::BackendError(
                "Stream failed on instance #405 after 404 instances: reset".to_string(),
            ),
            PipelineError::BackendError("Backend returned 404".to_string()),
            PipelineError::BackendError(
                "Backend request failed: http://pacs:4042/: connection refused".to_string(),
            ),
            PipelineError::BackendError(
                "DIMSE C-FIND failed: C-FIND failed with status: 0xC404".to_string(),
            ),
            PipelineError::ServiceError("Resource not found".to_string()),
            PipelineError::MiddlewareError(Box::new(std::io::Error::other(
                "Path not found: /studies",
            ))),
        ];
        for error in &errors {
            for operation in [Operation::Query, Operation::Retrieve, Operation::Move] {
                let PipelineFailure::Failed(failure) = pipeline_failure(operation, error) else {
                    panic!("{operation}: {error} became an empty result");
                };
                let code = failure.status_code().expect("a status code");
                assert_ne!(code, 0xA801, "{operation}: {error}");
            }
        }
    }

    #[test]
    fn test_object_not_found_is_still_a_failure_for_a_store() {
        let error = PipelineError::MiddlewareError(Box::new(DicomObjectNotFound(
            "no instance".to_string(),
        )));
        assert!(!is_no_match_error(Operation::Store, &error));
        match pipeline_failure(Operation::Store, &error) {
            PipelineFailure::Failed(error) => assert_eq!(error.status_code(), Some(0xC000)),
            PipelineFailure::NoMatches => panic!("a store has no empty result"),
        }
    }

    #[test]
    fn test_store_fails_on_a_non_2xx_http_status() {
        for http_status in [403, 500, 502] {
            let mut response = store_response(None, None);
            response.response_details.status = http_status;
            assert!(
                PipelineQueryProvider::check_store_response(&response).is_err(),
                "HTTP {http_status}"
            );
        }
    }

    #[test]
    fn test_store_fails_on_any_dicom_status_that_is_not_success_or_a_warning() {
        for status in [
            "0xA700", "0xA900", "0x0110", "0x0124", "0xC000", "0xFF00", "junk",
        ] {
            assert!(
                PipelineQueryProvider::check_store_response(&store_response(Some(status), None))
                    .is_err(),
                "{status}"
            );
        }
        for status in ["0x0000", "0xB000", "0xB006", "0xB007", "0x0107"] {
            assert!(
                PipelineQueryProvider::check_store_response(&store_response(Some(status), None))
                    .is_ok(),
                "{status}"
            );
        }
    }

    /// The status code of a failed `pipeline_failure` answer.
    fn failed_status(failure: PipelineFailure) -> u16 {
        match failure {
            PipelineFailure::Failed(error) => error
                .status_code()
                .unwrap_or_else(|| panic!("the error carries no status code: {error}")),
            PipelineFailure::NoMatches => panic!("expected a failure, got no matches"),
        }
    }

    #[test]
    fn test_pipeline_failure_carries_the_mapped_status() {
        let error =
            PipelineError::MiddlewareError(Box::new(crate::models::middleware::UpstreamFailure {
                operation: "find".to_string(),
                detail: "connection refused to 10.0.0.5:104".to_string(),
            }));
        let PipelineFailure::Failed(find) = pipeline_failure(Operation::Query, &error) else {
            panic!("an upstream failure is a failure");
        };
        assert_eq!(find.status_code(), Some(0xA700));
        assert_eq!(
            failed_status(pipeline_failure(Operation::Retrieve, &error)),
            0xA701
        );
        let generic = PipelineError::ConfigError("no backend".to_string());
        assert_eq!(
            failed_status(pipeline_failure(Operation::Move, &generic)),
            0x0110
        );
    }

    /// The final response of a store pipeline, as `store` sees it.
    fn store_response(
        dicom_status: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> ResponseEnvelope<Vec<u8>> {
        let original_data = body
            .as_ref()
            .map(|json| serde_json::to_vec(json).expect("serialize body"))
            .unwrap_or_default();
        let mut response = ResponseEnvelope::from_backend_with_command(
            bridge_request_details(),
            200,
            HashMap::new(),
            original_data,
            None,
            None,
        );
        response.normalized_data = body;
        if let Some(status) = dicom_status {
            response
                .response_details
                .metadata
                .insert("dicom_status".to_string(), status.to_string());
        }
        response
    }

    #[test]
    fn test_store_fails_when_the_receiver_reports_success_false() {
        // What a receiving Harmony sends back over HTTP 200 when its PACS is
        // down. No dicom_status crosses the HTTP hop.
        let response = store_response(
            None,
            Some(serde_json::json!({
                "operation": "store",
                "success": false,
                "error": "C-STORE to PACS@10.0.0.5:4242 failed: could not connect to server"
            })),
        );

        let error = PipelineQueryProvider::check_store_response(&response)
            .expect_err("success:false must fail the store");
        assert!(
            error.to_string().contains("could not connect to server"),
            "the error should carry the receiver's reason: {}",
            error
        );
    }

    // The whole path of a failed store on the bridge, without the network:
    // the receiving Harmony's own failure response, written as NDJSON, parsed
    // by the sending Harmony and judged by `store`. If the receiver's
    // response changes shape, this test fails.
    #[tokio::test]
    async fn test_failed_store_on_the_receiving_harmony_fails_the_store_on_the_sender() {
        use crate::models::services::types::dicom_scu_streaming::DicomScuStreamingBackend;
        use crate::models::services::types::http_streaming::{HttpStreamingEndpoint, StreamFormat};
        use crate::models::services::types::http_streaming_backend::HttpStreamingBackend;
        use bytes::Bytes;
        use futures_util::StreamExt;

        // Harmony B: the PACS did not take the instance.
        let not_stored = DicomScuStreamingBackend::build_store_response(
            &bridge_request_details(),
            &None,
            &dimse::config::RemoteNode::new("PACS", "10.0.0.5", 4242),
            false,
            Some("C-STORE to PACS@10.0.0.5:4242 failed: could not connect to server".to_string()),
            None,
            Some("1.2.3".to_string()),
        );

        // Harmony B: envelope → NDJSON response body
        let writer = HttpStreamingEndpoint {
            default_format: None,
            buffer_size: None,
            enable_compression: None,
        };
        let response = writer
            .build_streaming_response(
                Box::pin(futures_util::stream::iter(vec![Ok(not_stored)])),
                StreamFormat::Ndjson,
                &HashMap::new(),
            )
            .expect("NDJSON response should build");
        assert_eq!(
            response.status(),
            200,
            "the receiver answers 200 for a failed store"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read NDJSON body");

        // Harmony A: NDJSON body → the last envelope, which `store` judges
        let chunks: Vec<Result<Bytes, reqwest::Error>> = vec![Ok(body)];
        let last = HttpStreamingBackend::parse_ndjson_stream(
            futures_util::stream::iter(chunks),
            bridge_request_details(),
            None,
        )
        .collect::<Vec<_>>()
        .await
        .pop()
        .expect("one line in, one envelope out")
        .expect("NDJSON line should parse");
        assert!(
            !last.response_details.metadata.contains_key("dicom_status"),
            "dicom_status does not cross the HTTP hop, so the body must carry the result"
        );

        let error = PipelineQueryProvider::check_store_response(&last)
            .expect_err("a store the receiver could not complete must fail on the sender");
        assert!(
            error.to_string().contains("could not connect to server"),
            "the error should carry the receiver's reason: {}",
            error
        );
    }

    #[test]
    fn test_store_succeeds_when_nothing_reports_a_failure() {
        let stored = serde_json::json!({ "operation": "store", "success": true });
        // A receiver that is not Harmony, such as a STOW-RS server, has no
        // `success` key in its answer.
        let stow_rs = serde_json::json!({ "00081199": { "vr": "SQ", "Value": [] } });

        for (dicom_status, body) in [
            (None, Some(stored.clone())),
            (Some("0x0000"), Some(stored)),
            (Some("0x0000"), Some(stow_rs.clone())),
            (None, Some(stow_rs)),
            (None, None),
        ] {
            let response = store_response(dicom_status, body.clone());
            assert!(
                PipelineQueryProvider::check_store_response(&response).is_ok(),
                "dicom_status {:?} with body {:?} must not fail the store",
                dicom_status,
                body
            );
        }
    }

    #[test]
    fn test_store_fails_on_a_failure_dicom_status_whatever_the_body_says() {
        let response = store_response(
            Some("0xC000"),
            Some(serde_json::json!({ "operation": "store", "success": true })),
        );

        let error = PipelineQueryProvider::check_store_response(&response)
            .expect_err("dicom_status 0xC000 must fail the store");
        assert!(
            error.to_string().contains("0xC000"),
            "unexpected error: {}",
            error
        );
    }

    #[test]
    fn test_store_fails_on_success_false_even_with_a_success_dicom_status() {
        let response = store_response(
            Some("0x0000"),
            Some(serde_json::json!({ "operation": "store", "success": false })),
        );

        PipelineQueryProvider::check_store_response(&response)
            .expect_err("success:false must fail the store");
    }

    #[test]
    fn test_store_error_keeps_the_receiver_reason_on_one_bounded_line() {
        let reason = format!("first line\nforged log line {}", "x".repeat(4096));
        let response = store_response(
            None,
            Some(serde_json::json!({ "success": false, "error": reason })),
        );

        let error = PipelineQueryProvider::check_store_response(&response)
            .expect_err("success:false must fail the store")
            .to_string();
        assert!(
            !error.contains('\n'),
            "the reason must not add a line: {}",
            error
        );
        assert!(
            error.len() < MAX_RECEIVER_REASON_CHARS + 256,
            "the reason must be cut, got {} bytes",
            error.len()
        );
    }

    fn stored_copy_work_dir() -> tempfile::TempDir {
        let tmp_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp");
        std::fs::create_dir_all(&tmp_root).expect("create ./tmp");
        tempfile::tempdir_in(&tmp_root).expect("create work dir")
    }

    #[test]
    fn test_stored_instance_copy_removes_the_file_on_drop() {
        let work_dir = stored_copy_work_dir();
        let path = work_dir.path().join("instance.dcm");
        std::fs::write(&path, b"instance").expect("write instance");

        drop(StoredInstanceCopy::new(path.clone()));

        assert!(
            !path.exists(),
            "the copy must be removed: {}",
            path.display()
        );
    }

    #[test]
    fn test_stored_instance_copy_keeps_the_file_when_kept() {
        let work_dir = stored_copy_work_dir();
        let path = work_dir.path().join("kept.dcm");
        std::fs::write(&path, b"instance").expect("write instance");

        StoredInstanceCopy::new(path.clone()).keep();

        assert!(path.exists(), "a kept copy must stay: {}", path.display());
    }

    #[test]
    fn test_stored_instance_copy_ignores_a_file_already_moved() {
        let work_dir = stored_copy_work_dir();
        let path = work_dir.path().join("moved.dcm");

        drop(StoredInstanceCopy::new(path.clone()));

        assert!(!path.exists());
    }
}

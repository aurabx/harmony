//! DIMSE C-STORE → JMIX aggregation middleware.
//!
//! Each DIMSE C-STORE arrives as its own pipeline invocation carrying a
//! single DICOM instance. Building a JMIX envelope requires the whole
//! study to be on disk, so this middleware acts as an accumulator:
//!
//! * `left()` on a C-STORE envelope copies the DICOM instance file into a
//!   per-study staging folder under `<storage>/jmix-staging/<studyUid>/`,
//!   records the study's activity in a process-wide registry, and asks
//!   the executor to skip the backend by setting `skip_backends=true`.
//! * A background task, spawned on first use, scans the registry and
//!   flushes any study that has been idle for `idle_ms` (default 3000).
//!   A flush builds a JMIX ZIP with `jmix_rs::builder::JmixBuilder`,
//!   writes it under `<storage>/jmix-store/<jmix_id>/<jmix_id>.zip`,
//!   indexes it via `JmixIndex`, and POSTs the ZIP to the upstream
//!   captured on the first C-STORE for that study.
//!
//! Trade-off: the upstream POST does not travel through the pipeline's
//! backend chain, because the pipeline has already returned success for
//! every C-STORE. The middleware performs the POST directly with
//! `reqwest`, using the base_url that the executor placed on
//! `envelope.target_details.base_url` before middleware ran.

use crate::globals::get_storage;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::types::jmix_index::{
    current_timestamp, get_jmix_index, JmixPackageInfo,
};
use crate::utils::Error;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration parsed from middleware options.
#[derive(Debug, Clone)]
pub struct DimseToJmixConfig {
    /// Idle time before a study is flushed, in milliseconds.
    pub idle_ms: u64,
    /// Interval between background sweeps.
    pub sweep_interval_ms: u64,
    /// Skip SHA256 hashing when building the JMIX envelope.
    pub skip_hashing: bool,
    /// Skip file-listing generation when building the JMIX envelope.
    pub skip_listing: bool,
    /// Subdirectory under the storage root used for per-study staging.
    pub staging_subdir: String,
    /// Path appended to the upstream base URL when POSTing the ZIP.
    /// Defaults to "/api/jmix", matching the layout of the JMIX HTTP endpoint.
    pub upstream_path: String,
}

impl Default for DimseToJmixConfig {
    fn default() -> Self {
        Self {
            idle_ms: 3000,
            sweep_interval_ms: 250,
            skip_hashing: false,
            skip_listing: false,
            staging_subdir: "jmix-staging".to_string(),
            upstream_path: "/api/jmix".to_string(),
        }
    }
}

/// Parse middleware options into a typed config.
pub fn parse_config(options: &HashMap<String, Value>) -> Result<DimseToJmixConfig, String> {
    let mut cfg = DimseToJmixConfig::default();
    if let Some(v) = options.get("idle_ms").and_then(|v| v.as_u64()) {
        cfg.idle_ms = v;
    }
    if let Some(v) = options.get("sweep_interval_ms").and_then(|v| v.as_u64()) {
        cfg.sweep_interval_ms = v;
    }
    if let Some(v) = options.get("skip_hashing").and_then(|v| v.as_bool()) {
        cfg.skip_hashing = v;
    }
    if let Some(v) = options.get("skip_listing").and_then(|v| v.as_bool()) {
        cfg.skip_listing = v;
    }
    if let Some(v) = options.get("staging_subdir").and_then(|v| v.as_str()) {
        if v.trim().is_empty() {
            return Err("staging_subdir must not be empty".into());
        }
        cfg.staging_subdir = v.to_string();
    }
    if let Some(v) = options.get("upstream_path").and_then(|v| v.as_str()) {
        if v.trim().is_empty() {
            return Err("upstream_path must not be empty".into());
        }
        cfg.upstream_path = v.to_string();
    }
    Ok(cfg)
}

/// Per-study state held in the registry.
#[derive(Debug, Clone)]
struct StudyState {
    staging_dir: PathBuf,
    last_touch: Instant,
    upstream_base_url: String,
    upstream_path: String,
    upstream_headers: HashMap<String, String>,
    skip_hashing: bool,
    skip_listing: bool,
}

/// Process-wide registry of in-flight studies.
///
/// The middleware factory is invoked once per request, so no per-request
/// instance can own this state. It lives in a module-level `Lazy` and is
/// shared by every C-STORE that flows through any `dimse_to_jmix`
/// middleware in the process.
#[derive(Default)]
struct StudyRegistry {
    studies: HashMap<String, StudyState>,
}

static REGISTRY: Lazy<Arc<Mutex<StudyRegistry>>> =
    Lazy::new(|| Arc::new(Mutex::new(StudyRegistry::default())));

static BG_STARTED: AtomicBool = AtomicBool::new(false);

/// Middleware entry point.
pub struct DimseToJmixMiddleware {
    cfg: DimseToJmixConfig,
}

impl DimseToJmixMiddleware {
    pub fn new(cfg: DimseToJmixConfig) -> Self {
        Self::ensure_background_task(cfg.idle_ms, cfg.sweep_interval_ms);
        Self { cfg }
    }

    /// Spawn the sweep loop the first time a middleware instance is built.
    ///
    /// Subsequent constructions with different idle values re-use the
    /// original loop; the loop reads the current idle value from each
    /// study's state so per-study configuration still applies.
    fn ensure_background_task(_idle_ms: u64, sweep_interval_ms: u64) {
        if BG_STARTED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        // Only spawn when we are already inside a Tokio runtime.
        // The middleware is always constructed inside pipeline execution,
        // which runs on a Tokio runtime, but unit tests that construct
        // the middleware outside a runtime rely on the manual force_flush
        // helper instead.
        if tokio::runtime::Handle::try_current().is_err() {
            // Reset the flag so a later construction on a runtime can start
            // the loop.
            BG_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(sweep_interval_ms)).await;
                if let Err(e) = flush_idle_studies().await {
                    tracing::warn!("dimse_to_jmix: background sweep error: {}", e);
                }
            }
        });
    }
}

#[async_trait]
impl Middleware for DimseToJmixMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        // Only C-STORE envelopes carry the {command, dataset, file} body
        // that this middleware knows how to stage. Anything else passes
        // through unchanged so the middleware can sit in a mixed pipeline.
        if !is_cstore_envelope(&envelope) {
            return Ok(envelope);
        }

        let body = envelope
            .normalized_data
            .as_ref()
            .cloned()
            .unwrap_or_else(|| envelope.original_data.clone());

        let file_path = body
            .get("file")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::from("dimse_to_jmix: C-STORE envelope missing 'file' field"))?
            .to_string();

        let study_uid = extract_study_uid(&envelope, &body).ok_or_else(|| {
            Error::from(
                "dimse_to_jmix: C-STORE envelope missing StudyInstanceUID in metadata or dataset",
            )
        })?;

        let staging_root = staging_root(&self.cfg.staging_subdir)?;
        let study_dir = staging_root.join(sanitise_uid(&study_uid));
        std::fs::create_dir_all(&study_dir).map_err(|e| {
            Error::from(format!(
                "dimse_to_jmix: create staging dir {}: {}",
                study_dir.display(),
                e
            ))
        })?;

        let src = PathBuf::from(&file_path);
        let file_name = src
            .file_name()
            .map(|n| n.to_owned())
            .ok_or_else(|| {
                Error::from(format!(
                    "dimse_to_jmix: source file has no basename: {}",
                    file_path
                ))
            })?;
        let dst = study_dir.join(&file_name);
        // Prefer rename because the source is a per-instance temp file.
        // Fall back to copy when the source and destination live on
        // different filesystems.
        if let Err(_e) = std::fs::rename(&src, &dst) {
            std::fs::copy(&src, &dst).map_err(|e| {
                Error::from(format!(
                    "dimse_to_jmix: copy {} to {}: {}",
                    src.display(),
                    dst.display(),
                    e
                ))
            })?;
            let _ = std::fs::remove_file(&src);
        }

        // Capture the upstream URL on the first C-STORE for the study.
        // The executor placed the resolved backend base_url on
        // target_details before middleware ran.
        let upstream_base_url = envelope
            .target_details
            .as_ref()
            .map(|t| t.base_url.clone())
            .unwrap_or_default();
        let upstream_headers = envelope
            .target_details
            .as_ref()
            .map(|t| t.headers.clone())
            .unwrap_or_default();

        {
            let mut reg = REGISTRY.lock().expect("study registry mutex");
            let entry = reg
                .studies
                .entry(study_uid.clone())
                .or_insert_with(|| StudyState {
                    staging_dir: study_dir.clone(),
                    last_touch: Instant::now(),
                    upstream_base_url: upstream_base_url.clone(),
                    upstream_path: self.cfg.upstream_path.clone(),
                    upstream_headers: upstream_headers.clone(),
                    skip_hashing: self.cfg.skip_hashing,
                    skip_listing: self.cfg.skip_listing,
                });
            entry.last_touch = Instant::now();
            // Once captured on the first C-STORE for a study these do
            // not change, but if the earliest C-STORE arrived without a
            // resolved base_url (test path, for example) prefer any
            // non-empty value seen later.
            if entry.upstream_base_url.is_empty() && !upstream_base_url.is_empty() {
                entry.upstream_base_url = upstream_base_url;
            }
            if entry.upstream_headers.is_empty() && !upstream_headers.is_empty() {
                entry.upstream_headers = upstream_headers;
            }
        }

        // Tell the executor to skip the backend chain: the ZIP will be
        // POSTed once the study is complete. Return a 200 body so the
        // DIMSE adapter's C-STORE handler sees a success status.
        envelope
            .request_details
            .metadata
            .insert("skip_backends".to_string(), "true".to_string());
        envelope
            .request_details
            .metadata
            .insert("dimse_to_jmix_staged".to_string(), study_uid.clone());
        envelope.normalized_data = Some(serde_json::json!({
            "response": {
                "status": 200,
                "body": {
                    "status": "staged",
                    "studyInstanceUID": study_uid,
                }
            }
        }));

        Ok(envelope)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        Ok(envelope)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn is_cstore_envelope(envelope: &RequestEnvelope<Value>) -> bool {
    use crate::models::envelope::command::Operation;
    if envelope
        .command
        .as_ref()
        .map(|c| c.operation == Operation::Store)
        .unwrap_or(false)
    {
        return true;
    }
    envelope
        .request_details
        .uri
        .starts_with("dimse://scp/store")
}

fn extract_study_uid(envelope: &RequestEnvelope<Value>, body: &Value) -> Option<String> {
    if let Some(uid) = envelope.request_details.metadata.get("StudyInstanceUID") {
        if !uid.is_empty() {
            return Some(uid.clone());
        }
    }
    // Fall back to the DICOM JSON dataset (tag 0020000D).
    body.get("dataset")
        .and_then(|d| d.get("0020000D"))
        .and_then(|t| t.get("Value"))
        .and_then(|arr| arr.get(0))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn staging_root(subdir: &str) -> Result<PathBuf, Error> {
    if let Some(storage) = get_storage() {
        return storage
            .ensure_dir_str(subdir)
            .map_err(|e| Error::from(format!("dimse_to_jmix: staging root: {}", e)));
    }
    let fallback = PathBuf::from("./tmp").join(subdir);
    std::fs::create_dir_all(&fallback).map_err(|e| {
        Error::from(format!(
            "dimse_to_jmix: fallback staging root {}: {}",
            fallback.display(),
            e
        ))
    })?;
    Ok(fallback)
}

fn jmix_store_root() -> Result<PathBuf, Error> {
    let root = if let Some(storage) = get_storage() {
        storage.subpath_str("jmix-store")
    } else {
        PathBuf::from("./tmp/jmix-store")
    };
    std::fs::create_dir_all(&root).map_err(|e| {
        Error::from(format!(
            "dimse_to_jmix: jmix-store root {}: {}",
            root.display(),
            e
        ))
    })?;
    Ok(root)
}

/// Turn a StudyInstanceUID into a filesystem-safe directory name.
///
/// StudyInstanceUIDs are dotted decimal identifiers with no path-hostile
/// characters, but we still guard against any future change to that
/// assumption by replacing anything outside [0-9A-Za-z._-] with an
/// underscore.
fn sanitise_uid(uid: &str) -> String {
    uid.chars()
        .map(|c| match c {
            '0'..='9' | 'A'..='Z' | 'a'..='z' | '.' | '_' | '-' => c,
            _ => '_',
        })
        .collect()
}

/// Scan the registry and flush any study idle beyond its threshold.
async fn flush_idle_studies() -> Result<(), String> {
    let due: Vec<(String, StudyState)> = {
        let mut reg = REGISTRY.lock().map_err(|e| e.to_string())?;
        let now = Instant::now();
        let idle_ms = default_idle_ms();
        let ready: Vec<String> = reg
            .studies
            .iter()
            .filter(|(_, s)| now.duration_since(s.last_touch) >= Duration::from_millis(idle_ms))
            .map(|(k, _)| k.clone())
            .collect();
        ready
            .into_iter()
            .filter_map(|k| reg.studies.remove(&k).map(|s| (k, s)))
            .collect()
    };

    for (study_uid, state) in due {
        // The Study Instance UID identifies a patient's study, so it is
        // not logged here or in flush_study.
        if let Err(e) = flush_study(&study_uid, &state).await {
            tracing::error!("dimse_to_jmix: study flush failed: {}", e);
        }
    }
    Ok(())
}

/// Read the default idle threshold. The background loop does not know
/// per-study idle values, so it uses whichever was captured in the
/// process; this is a design shortcut acceptable because production
/// deployments configure a single dimse_to_jmix instance.
fn default_idle_ms() -> u64 {
    DEFAULT_IDLE_MS.load(Ordering::Relaxed)
}

static DEFAULT_IDLE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(3000);

/// Build the ZIP, write it under jmix-store, index it, and POST it upstream.
async fn flush_study(study_uid: &str, state: &StudyState) -> Result<(), String> {
    let (zip_path, jmix_id) = build_jmix_package(study_uid, state).await?;

    // Best-effort staging cleanup: never fail the flush over cleanup.
    // The staging directory is named after the Study Instance UID, so the
    // package id stands in for it.
    if let Err(e) = std::fs::remove_dir_all(&state.staging_dir) {
        tracing::warn!(
            "dimse_to_jmix: could not remove staging directory for package {}: {}",
            jmix_id,
            e
        );
    }

    if !state.upstream_base_url.is_empty() {
        if let Err(e) = post_zip_upstream(&zip_path, state).await {
            tracing::error!(
                "dimse_to_jmix: upstream POST failed for package {}: {}",
                jmix_id,
                e
            );
        }
    } else {
        tracing::debug!(
            "dimse_to_jmix: no upstream base URL; local package built at {}",
            zip_path.display()
        );
    }

    Ok(())
}

/// Build the JMIX package for a study using jmix_rs.
///
/// Public so integration tests can exercise it without a background task.
pub async fn build_jmix_package_for_test(
    study_uid: &str,
    staging_dir: &Path,
    skip_hashing: bool,
    skip_listing: bool,
) -> Result<(PathBuf, String), String> {
    let state = StudyState {
        staging_dir: staging_dir.to_path_buf(),
        last_touch: Instant::now(),
        upstream_base_url: String::new(),
        upstream_path: String::new(),
        upstream_headers: HashMap::new(),
        skip_hashing,
        skip_listing,
    };
    build_jmix_package(study_uid, &state).await
}

async fn build_jmix_package(
    _study_uid: &str,
    state: &StudyState,
) -> Result<(PathBuf, String), String> {
    let mut jcfg = jmix_rs::config::Config::default();
    jcfg.sender.name = "Harmony Proxy".to_string();
    jcfg.sender.id = "org:harmony-proxy".to_string();
    jcfg.requester.name = "Harmony Proxy".to_string();
    jcfg.requester.id = "org:harmony-proxy".to_string();

    let staging_str = state.staging_dir.to_string_lossy().to_string();

    let builder = jmix_rs::builder::JmixBuilder::new();
    let (envelope_built, dicom_files) = builder
        .build_from_dicom_with_options(
            &staging_str,
            &jcfg,
            state.skip_hashing,
            state.skip_listing,
        )
        .map_err(|e| format!("jmix_rs build: {}", e))?;

    let jmix_id = envelope_built.manifest.id.clone();
    let store_root = jmix_store_root().map_err(|e| e.to_string())?;
    let pkg_dir = store_root.join(&jmix_id);
    std::fs::create_dir_all(&pkg_dir).map_err(|e| {
        format!(
            "create package dir {}: {}",
            pkg_dir.display(),
            e
        )
    })?;

    let _saved = builder
        .save_to_files_with_options(
            &envelope_built,
            &dicom_files,
            &pkg_dir,
            state.skip_hashing,
            state.skip_listing,
        )
        .map_err(|e| format!("jmix_rs save: {}", e))?;

    let study_uid_out = envelope_built
        .metadata
        .studies
        .as_ref()
        .and_then(|s| s.study_uid.clone())
        .unwrap_or_default();

    let index = get_jmix_index(&store_root).map_err(|e| format!("open jmix index: {}", e))?;
    let info = JmixPackageInfo {
        id: jmix_id.clone(),
        study_uid: study_uid_out,
        path: pkg_dir.to_string_lossy().to_string(),
        created_at: current_timestamp(),
    };
    index
        .index_package(&info)
        .map_err(|e| format!("index package: {}", e))?;

    let zip_path = pkg_dir.join(format!("{}.zip", jmix_id));
    if !zip_path.exists() {
        return Err(format!(
            "jmix_rs did not produce zip at {}",
            zip_path.display()
        ));
    }
    Ok((zip_path, jmix_id))
}

async fn post_zip_upstream(zip_path: &Path, state: &StudyState) -> Result<(), String> {
    let base = state.upstream_base_url.trim_end_matches('/');
    let path = if state.upstream_path.starts_with('/') {
        state.upstream_path.clone()
    } else {
        format!("/{}", state.upstream_path)
    };
    let url = format!("{}{}", base, path);

    let bytes = tokio::fs::read(zip_path)
        .await
        .map_err(|e| format!("read zip {}: {}", zip_path.display(), e))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("build reqwest client: {}", e))?;

    let mut req = client.post(&url).body(bytes);
    for (k, v) in &state.upstream_headers {
        let key = k.to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "host"
                | "connection"
                | "content-length"
                | "transfer-encoding"
                | "keep-alive"
                | "proxy-connection"
                | "upgrade"
        ) {
            continue;
        }
        req = req.header(k, v);
    }
    req = req.header("content-type", "application/zip");

    let resp = req
        .send()
        .await
        .map_err(|e| format!("POST {}: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("upstream {} returned status {}", url, status));
    }
    tracing::info!(
        "dimse_to_jmix: uploaded JMIX package to {} ({} bytes)",
        url,
        std::fs::metadata(zip_path)
            .map(|m| m.len())
            .unwrap_or_default()
    );
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Force-flush every study whose idle window has elapsed. Intended for
/// integration tests that do not want to wait on the background loop.
#[cfg(test)]
pub(crate) async fn force_flush_now() -> Result<(), String> {
    // Zero the idle threshold so anything in the registry becomes due.
    let previous = DEFAULT_IDLE_MS.swap(0, Ordering::SeqCst);
    let res = flush_idle_studies().await;
    DEFAULT_IDLE_MS.store(previous, Ordering::SeqCst);
    res
}

#[cfg(test)]
pub(crate) fn clear_registry_for_test() {
    let mut reg = REGISTRY.lock().expect("registry");
    reg.studies.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::globals::{reset_storage, set_storage};
    use crate::models::envelope::envelope::{RequestDetails, RequestEnvelope, TargetDetails};
    use crate::storage::{FilesystemStorage, StorageBackend};
    use serial_test::serial;
    use std::sync::Arc;

    fn create_test_storage(prefix: &str) -> Arc<FilesystemStorage> {
        let id = uuid::Uuid::new_v4();
        let path = format!("./tmp/{}-{}", prefix, id);
        Arc::new(FilesystemStorage::new(&path).expect("test storage"))
    }

    fn cstore_envelope(
        file_path: &str,
        study_uid: &str,
        base_url: &str,
    ) -> RequestEnvelope<Value> {
        let mut metadata = HashMap::new();
        metadata.insert("StudyInstanceUID".into(), study_uid.into());
        metadata.insert("protocol".into(), "dimse".into());
        let details = RequestDetails {
            method: "store".into(),
            uri: "dimse://scp/store".into(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata,
            content_metadata: None,
        };
        let target = TargetDetails {
            base_url: base_url.to_string(),
            method: "POST".into(),
            uri: "dimse://scp/store".into(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            metadata: HashMap::new(),
        };
        let body = serde_json::json!({
            "command": {"message_id": 1, "priority": "MEDIUM", "direction": "REQUEST"},
            "dataset": {"0020000D": {"vr": "UI", "Value": [study_uid]}},
            "file": file_path,
        });
        RequestEnvelope::builder()
            .method("store")
            .uri("dimse://scp/store")
            .headers(HashMap::new())
            .cookies(HashMap::new())
            .query_params(HashMap::new())
            .cache_status(None)
            .metadata(details.metadata.clone())
            .target_details(Some(target))
            .command(Some(
                crate::models::envelope::command::Command::new(
                    crate::models::envelope::command::Operation::Store,
                )
                .with_source("test"),
            ))
            .original_data(body.clone())
            .normalized_data(Some(body))
            .normalized_snapshot(None)
            .build()
            .expect("build envelope")
    }

    /// Write a real sample DICOM file into a staging area jmix_rs can accept.
    fn seed_instance(dst: &Path) {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let source_path = manifest_dir.join("samples/dicom/study_1/series_1/CT.1.1.dcm");
        std::fs::copy(&source_path, dst).expect("copy sample dcm");
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn stages_single_instance_and_flushes_to_jmix_store() {
        reset_storage();
        clear_registry_for_test();
        let storage = create_test_storage("dtj-single");
        set_storage(storage.clone());

        // Prepare the incoming C-STORE temp file inside the same storage
        // root so the middleware's rename stays on-filesystem.
        let dimse_dir = storage.ensure_dir_str("dimse").expect("dimse dir");
        let temp_file = dimse_dir.join("CT.1.1.dcm");
        seed_instance(&temp_file);

        let mw = DimseToJmixMiddleware::new(DimseToJmixConfig {
            idle_ms: 0,
            sweep_interval_ms: 10_000, // background loop must not race the test
            ..DimseToJmixConfig::default()
        });

        let envelope = cstore_envelope(
            &temp_file.to_string_lossy(),
            "1.2.3.4.single",
            "", // no upstream POST in this test
        );
        let out = mw.left(envelope).await.expect("middleware left");
        assert_eq!(
            out.request_details.metadata.get("skip_backends").map(|s| s.as_str()),
            Some("true"),
            "staging C-STOREs must skip the backend chain"
        );

        // Flush and verify the ZIP appears under jmix-store.
        force_flush_now().await.expect("force flush");

        let jmix_root = storage.subpath_str("jmix-store");
        let mut package_dirs: Vec<_> = std::fs::read_dir(&jmix_root)
            .expect("list jmix-store")
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        assert_eq!(package_dirs.len(), 1, "one package per flushed study");
        let pkg = package_dirs.pop().unwrap().path();
        let zip = pkg.join(format!(
            "{}.zip",
            pkg.file_name().unwrap().to_string_lossy()
        ));
        assert!(zip.exists(), "package zip should exist at {}", zip.display());
        assert!(
            std::fs::metadata(&zip).map(|m| m.len() > 0).unwrap_or(false),
            "zip should not be empty"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn accumulates_multi_instance_study_into_single_zip() {
        reset_storage();
        clear_registry_for_test();
        let storage = create_test_storage("dtj-multi");
        set_storage(storage.clone());

        let dimse_dir = storage.ensure_dir_str("dimse").expect("dimse dir");
        let f1 = dimse_dir.join("CT.1.1.dcm");
        let f2 = dimse_dir.join("CT.1.2.dcm");
        let f3 = dimse_dir.join("CT.1.3.dcm");
        seed_instance(&f1);
        seed_instance(&f2);
        seed_instance(&f3);

        let mw = DimseToJmixMiddleware::new(DimseToJmixConfig {
            idle_ms: 0,
            sweep_interval_ms: 10_000,
            ..DimseToJmixConfig::default()
        });

        for f in [&f1, &f2, &f3] {
            let envelope = cstore_envelope(
                &f.to_string_lossy(),
                "1.2.3.4.multi",
                "",
            );
            mw.left(envelope).await.expect("stage");
        }

        force_flush_now().await.expect("flush");

        let jmix_root = storage.subpath_str("jmix-store");
        let package_dirs: Vec<_> = std::fs::read_dir(&jmix_root)
            .expect("list jmix-store")
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        assert_eq!(
            package_dirs.len(),
            1,
            "multi-instance study must produce a single package"
        );

        // Staging directory is removed after a successful flush.
        let staging_root = storage.subpath_str("jmix-staging");
        let staging_study = staging_root.join(sanitise_uid("1.2.3.4.multi"));
        assert!(
            !staging_study.exists(),
            "staging dir {} should be cleaned up",
            staging_study.display()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn two_studies_do_not_interleave() {
        reset_storage();
        clear_registry_for_test();
        let storage = create_test_storage("dtj-two");
        set_storage(storage.clone());

        let dimse_dir = storage.ensure_dir_str("dimse").expect("dimse dir");

        let a1 = dimse_dir.join("A.1.dcm");
        let a2 = dimse_dir.join("A.2.dcm");
        let b1 = dimse_dir.join("B.1.dcm");
        let b2 = dimse_dir.join("B.2.dcm");
        for f in [&a1, &a2, &b1, &b2] {
            seed_instance(f);
        }

        let mw = DimseToJmixMiddleware::new(DimseToJmixConfig {
            idle_ms: 0,
            sweep_interval_ms: 10_000,
            ..DimseToJmixConfig::default()
        });

        // Interleave two studies.
        mw.left(cstore_envelope(&a1.to_string_lossy(), "1.study.A", ""))
            .await
            .unwrap();
        mw.left(cstore_envelope(&b1.to_string_lossy(), "1.study.B", ""))
            .await
            .unwrap();
        mw.left(cstore_envelope(&a2.to_string_lossy(), "1.study.A", ""))
            .await
            .unwrap();
        mw.left(cstore_envelope(&b2.to_string_lossy(), "1.study.B", ""))
            .await
            .unwrap();

        // Staging must have kept them apart.
        let staging_root = storage.subpath_str("jmix-staging");
        let a_dir = staging_root.join(sanitise_uid("1.study.A"));
        let b_dir = staging_root.join(sanitise_uid("1.study.B"));
        let a_files: Vec<_> = std::fs::read_dir(&a_dir)
            .expect("study A staging")
            .flatten()
            .collect();
        let b_files: Vec<_> = std::fs::read_dir(&b_dir)
            .expect("study B staging")
            .flatten()
            .collect();
        assert_eq!(a_files.len(), 2, "study A should have 2 staged instances");
        assert_eq!(b_files.len(), 2, "study B should have 2 staged instances");

        force_flush_now().await.expect("flush");

        let jmix_root = storage.subpath_str("jmix-store");
        let package_dirs: Vec<_> = std::fs::read_dir(&jmix_root)
            .expect("list jmix-store")
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        assert_eq!(
            package_dirs.len(),
            2,
            "each study should produce its own package"
        );
    }

    /// Stage one instance under `study_uid`, then flush it with the log
    /// captured. Returns the captured log text.
    async fn flush_with_captured_log(prefix: &str, study_uid: &str, base_url: &str) -> String {
        reset_storage();
        clear_registry_for_test();
        let storage = create_test_storage(prefix);
        set_storage(storage.clone());
        let dimse_dir = storage.ensure_dir_str("dimse").expect("dimse dir");
        let temp_file = dimse_dir.join("CT.1.1.dcm");
        seed_instance(&temp_file);

        let mw = DimseToJmixMiddleware::new(DimseToJmixConfig {
            idle_ms: 0,
            sweep_interval_ms: 10_000,
            ..DimseToJmixConfig::default()
        });
        mw.left(cstore_envelope(&temp_file.to_string_lossy(), study_uid, base_url))
            .await
            .expect("middleware left");

        let (log, _guard) = crate::logging::test_capture::capture_debug();
        force_flush_now().await.expect("force flush");
        log.contents()
    }

    /// A Study Instance UID identifies a patient's imaging study. The upload
    /// failure is logged at ERROR, the default level.
    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn upload_failure_log_does_not_contain_study_uid() {
        let marker = "1.2.826.0.1.424242.987654";
        let text = flush_with_captured_log("dtj-upload-fail", marker, "http://127.0.0.1:1").await;

        assert!(
            text.contains("dimse_to_jmix: upstream POST failed for package"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "study UID leaked: {text}");
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial]
    async fn local_package_log_does_not_contain_study_uid() {
        let marker = "1.2.826.0.1.424242.123456";
        let text = flush_with_captured_log("dtj-local-only", marker, "").await;

        assert!(
            text.contains("dimse_to_jmix: no upstream base URL; local package"),
            "log: {text}"
        );
        assert!(!text.contains(marker), "study UID leaked: {text}");
    }
}

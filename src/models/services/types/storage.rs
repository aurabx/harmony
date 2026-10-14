use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use chrono::Utc;
use futures_util::StreamExt;
use object_store::local::LocalFileSystem;
use object_store::{ObjectStore, PutMode, PutOptions, path::Path as ObjectStorePath};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use super::dicom_store_body::part10_from_store_body;

/// The `content` value that writes the DICOM Part 10 instance of a C-STORE.
pub(crate) const CONTENT_DICOM_PART10: &str = "dicom_part10";

/// What a write puts in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteContent {
    /// The request body, unchanged. This is the default.
    Body,
    /// The DICOM Part 10 instance in the body. A `dicom_scp` endpoint's body
    /// is JSON that describes the instance, so writing it as-is would store
    /// the JSON, not the image.
    DicomPart10,
}

impl WriteContent {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None => Ok(Self::Body),
            Some(CONTENT_DICOM_PART10) => Ok(Self::DicomPart10),
            Some(other) => Err(format!(
                "unknown content '{}'; the only accepted value is '{}'",
                other, CONTENT_DICOM_PART10
            )),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct StorageBackend {
    pub root: Option<String>,
    pub write_pattern: Option<String>,
    pub read_pattern: Option<String>,
    /// `dicom_part10` writes the instance of a C-STORE instead of the body.
    pub content: Option<String>,
    // S3 configuration
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub endpoint: Option<String>,
}

impl StorageBackend {
    /// Create the appropriate ObjectStore implementation based on configuration
    fn get_store(&self, options: &HashMap<String, Value>) -> Result<Arc<dyn ObjectStore>, Error> {
        let root = options
            .get("root")
            .and_then(|v| v.as_str())
            .or(self.root.as_deref())
            .unwrap_or("./tmp/disk");

        if root.starts_with("s3://") {
            use object_store::aws::AmazonS3Builder;
            
            // Parse bucket from s3://bucket/prefix or options
            let bucket = if let Some(rest) = root.strip_prefix("s3://") {
                rest.split('/').next().unwrap_or("").to_string()
            } else {
                self.bucket.clone().unwrap_or_default()
            };

            let region = options.get("region").and_then(|v| v.as_str())
                .or(self.region.as_deref()).unwrap_or("us-east-1");
            
            let access_key = options.get("access_key_id").and_then(|v| v.as_str())
                .or(self.access_key_id.as_deref());
                
            let secret_key = options.get("secret_access_key").and_then(|v| v.as_str())
                .or(self.secret_access_key.as_deref());

            let endpoint = options.get("endpoint").and_then(|v| v.as_str())
                .or(self.endpoint.as_deref());

            let mut builder = AmazonS3Builder::new()
                .with_region(region)
                .with_bucket_name(bucket);

            if let (Some(k), Some(s)) = (access_key, secret_key) {
                builder = builder.with_access_key_id(k).with_secret_access_key(s);
            }
            
            if let Some(ep) = endpoint {
                builder = builder.with_endpoint(ep);
            }

            let store = builder.build().map_err(|e| Error::from(format!("Failed to build S3 store: {}", e)))?;
            Ok(Arc::new(store))
        } else {
            // Local filesystem
            // Ensure directory exists if it's a local path
            let path = std::path::Path::new(root);
            if !path.exists() {
                std::fs::create_dir_all(path).map_err(|e| Error::from(e.to_string()))?;
            }
            let store = LocalFileSystem::new_with_prefix(path).map_err(|e| Error::from(e.to_string()))?;
            Ok(Arc::new(store))
        }
    }

    fn resolve_path(
        &self,
        pattern: &str,
        envelope: &RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<ObjectStorePath, Error> {
        let mut path_str = pattern.to_string();

        // 1. Context replacements from metadata
        for (key, value) in &envelope.request_details.metadata {
            path_str = path_str.replace(&format!("{{{}}}", key), value);
        }

        // 2. Replacements from normalized_data (JMIX fields)
        if let Some(nd) = &envelope.normalized_data {
            if let Some(obj) = nd.as_object() {
                 for (key, value) in obj {
                    if let Some(s) = value.as_str() {
                        path_str = path_str.replace(&format!("{{{}}}", key), s);
                    }
                 }
            }
        }

        // 3. Special tokens
        if path_str.contains("{uuid}") {
             path_str = path_str.replace("{uuid}", &Uuid::new_v4().to_string());
        }
        if path_str.contains("{timestamp}") {
            path_str = path_str.replace("{timestamp}", &Utc::now().to_rfc3339());
        }
        
        // ObjectStore paths must not be absolute (no leading /) and handle URL encoding safely
        let clean_path = path_str.trim_start_matches('/');
        Ok(ObjectStorePath::parse(clean_path).map_err(|e| Error::from(format!("Invalid path: {}", e)))?)
    }

    /// Read the `content` option. An option value overrides the struct field.
    fn write_content(&self, options: &HashMap<String, Value>) -> Result<WriteContent, String> {
        let value = match options.get("content") {
            Some(Value::String(s)) => Some(s.as_str()),
            Some(other) => return Err(format!("content must be a string, got {}", other)),
            None => self.content.as_deref(),
        };
        WriteContent::parse(value)
    }

    /// The bytes a write puts in the file, chosen by `content`.
    ///
    /// In `dicom_part10` mode a body with no Part 10 instance fails the write.
    /// A `dicom_scp` endpoint then answers the sending device with a failure
    /// status, so the device keeps its copy. The `file` path in the body is
    /// never read.
    fn write_bytes(&self, options: &HashMap<String, Value>, body: &[u8]) -> Result<Vec<u8>, Error> {
        match self.write_content(options).map_err(Error::from)? {
            WriteContent::Body => Ok(body.to_vec()),
            WriteContent::DicomPart10 => part10_from_store_body(body)
                .map(|part10| part10.into_owned())
                .map_err(|e| {
                    Error::from(format!(
                        "storage backend with content = '{}': {}",
                        CONTENT_DICOM_PART10, e
                    ))
                }),
        }
    }

    /// Resolve `write_pattern` to the path of the file a write creates.
    ///
    /// Each `{name}` is replaced once. `{uuid}` and `{timestamp}` are
    /// generated. Any other value comes from request metadata, then
    /// normalized data.
    /// A value is never scanned for further placeholders.
    ///
    /// Values come from the client: a DICOM header, a JWT claim. So a value
    /// fills part of one path segment and no more. A value that is empty, is
    /// `.` or `..`, or holds `/` or `\` fails the write. A placeholder with no
    /// value also fails it. Left in the path as text, it would send every such
    /// request to the same file.
    ///
    /// Error messages name the placeholder but not its value, which may be
    /// patient data.
    fn resolve_write_path(
        &self,
        pattern: &str,
        envelope: &RequestEnvelope<Vec<u8>>,
    ) -> Result<ObjectStorePath, Error> {
        let mut resolved = String::with_capacity(pattern.len());
        let mut rest = pattern;
        while let Some(start) = rest.find('{') {
            resolved.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('}').ok_or_else(|| {
                Error::from(format!(
                    "write_pattern '{}' has a '{{' with no closing '}}'",
                    pattern
                ))
            })?;
            let name = &after[..end];
            let value = Self::placeholder_value(name, envelope).ok_or_else(|| {
                Error::from(format!(
                    "write_pattern placeholder '{{{}}}' has no value in the request",
                    name
                ))
            })?;
            Self::check_segment_value(name, &value)?;
            resolved.push_str(&value);
            rest = &after[end + 1..];
        }
        resolved.push_str(rest);

        let clean_path = resolved.trim_start_matches('/');
        ObjectStorePath::parse(clean_path)
            .map_err(|e| {
                tracing::debug!("Invalid storage write path: {}", e);
                Error::from("the resolved write path has a segment that is not allowed")
            })
    }

    /// Write a DICOM instance without replacing a different one.
    ///
    /// A device may resend an instance it already sent, so identical bytes
    /// at the path count as stored. Different bytes fail the write: a peer
    /// must not replace an archived instance by reusing its UIDs.
    async fn put_dicom_instance(
        store: &dyn ObjectStore,
        path: &ObjectStorePath,
        bytes: bytes::Bytes,
    ) -> Result<(), Error> {
        let create = PutOptions {
            mode: PutMode::Create,
            ..Default::default()
        };
        match store.put_opts(path, bytes.clone().into(), create).await {
            Ok(_) => Ok(()),
            Err(object_store::Error::AlreadyExists { .. }) => {
                let existing = store
                    .get(path)
                    .await
                    .map_err(Self::write_error)?
                    .bytes()
                    .await
                    .map_err(Self::write_error)?;
                if existing == bytes {
                    Ok(())
                } else {
                    Err(Error::from(
                        "a different instance is already stored at the write path",
                    ))
                }
            }
            Err(e) => Err(Self::write_error(e)),
        }
    }

    /// Describe a store error without its message. object_store messages
    /// hold the full path, which may hold patient data.
    fn write_error(e: object_store::Error) -> Error {
        tracing::debug!("Storage write failed: {}", e);
        let kind = match e {
            object_store::Error::NotFound { .. } => "not found",
            object_store::Error::AlreadyExists { .. } => "already exists",
            object_store::Error::PermissionDenied { .. } => "permission denied",
            object_store::Error::Unauthenticated { .. } => "unauthenticated",
            object_store::Error::NotImplemented => "not supported by this store",
            _ => "store error",
        };
        Error::from(format!("Failed to write to storage: {}", kind))
    }

    /// `uuid` and `timestamp` are reserved. Request data cannot set them, so
    /// a client cannot choose the name of a file the pattern makes unique.
    fn placeholder_value(name: &str, envelope: &RequestEnvelope<Vec<u8>>) -> Option<String> {
        match name {
            "uuid" => return Some(Uuid::new_v4().to_string()),
            "timestamp" => return Some(Utc::now().to_rfc3339()),
            _ => {}
        }
        if let Some(value) = envelope.request_details.metadata.get(name) {
            return Some(value.clone());
        }
        if let Some(value) = envelope
            .normalized_data
            .as_ref()
            .and_then(|nd| nd.get(name))
            .and_then(Value::as_str)
        {
            return Some(value.to_string());
        }
        None
    }

    fn check_segment_value(name: &str, value: &str) -> Result<(), Error> {
        let reason = if value.is_empty() {
            "is empty"
        } else if value == "." || value == ".." {
            "is a relative path segment"
        } else if value.contains(['/', '\\']) {
            "contains a path separator"
        } else {
            return Ok(());
        };
        Err(Error::from(format!(
            "write_pattern placeholder '{{{}}}' {}",
            name, reason
        )))
    }
}

#[async_trait]
impl ServiceType for StorageBackend {
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        self.write_content(options)
            .map(|_| ())
            .map_err(|reason| ConfigError::InvalidBackend {
                name: "storage".to_string(),
                reason,
            })
    }

    fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<crate::router::route_config::RouteConfig> {
        vec![]
    }
}

#[async_trait]
impl ServiceHandler<Value> for StorageBackend {
    type ReqBody = Value;

    async fn endpoint_incoming_request(
        &self,
        _envelope: RequestEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
        Err(Error::from("StorageBackend cannot be used as an endpoint"))
    }

    async fn backend_outgoing_request(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
        use crate::models::envelope::command::Operation;

        let method = envelope.request_details.method.to_uppercase();
        let store = self.get_store(options)?;

        // Determine if this is a read or write operation using Command (preferred) or HTTP method (fallback)
        let is_read = match envelope.resolve_operation() {
            Some(Operation::Query) | Some(Operation::Retrieve) => true,
            Some(Operation::Store) | Some(Operation::Update) | Some(Operation::Patch) => false,
            Some(Operation::Delete) => false, // treat as write-side for now
            _ => method == "GET",             // fallback to HTTP method
        };
        
        if is_read {
             // Read operation
             let pattern = options
                .get("read_pattern")
                .and_then(|v| v.as_str())
                .or(self.read_pattern.as_deref())
                .unwrap_or("{storedPath}");

             // Check if 'path' query param is provided (e.g. disk://root?path=...)
             // The envelope.target_details.uri might contain it if we parsed it?
             // Actually, envelope.request_details.query_params might have it if not consumed.
             // But we rely on metadata injection or pattern resolution.
             // If pattern is just "{storedPath}", we expect "storedPath" in metadata.
             
             let path = self.resolve_path(pattern, &envelope, options)?;
             
             match store.get(&path).await {
                 Ok(result) => {
                     let stream = result.into_stream();
                     // Collect stream into bytes
                     let bytes = stream.map(|chunk| chunk.unwrap_or_default()) // simplified error handling
                        .collect::<Vec<_>>()
                        .await
                        .concat(); // TODO: handle Bytes properly

                     // Try to guess content type
                     let mut headers = HashMap::new();
                     if let Some(ext) = path.extension() {
                         let ct = match ext {
                             "json" => "application/json",
                             "xml" => "application/xml",
                             "dcm" => "application/dicom",
                             _ => "application/octet-stream",
                         };
                         headers.insert("content-type".to_string(), ct.to_string());
                     }

                     Ok(ResponseEnvelope::from_backend_with_command(
                         envelope.request_details.clone(),
                         200,
                         headers,
                         bytes,
                         None,
                         envelope.command.clone(),
                     ))
                 },
                 Err(object_store::Error::NotFound { .. }) => {
                     Ok(ResponseEnvelope::from_backend_with_command(
                         envelope.request_details.clone(),
                         404,
                         HashMap::new(),
                         b"File not found".to_vec(),
                         None,
                         envelope.command.clone(),
                     ))
                 },
                 Err(e) => Err(Error::from(format!("Storage error: {}", e)))
             }

        } else {
            // Write operation (POST, PUT, etc)
             let pattern = options
                .get("write_pattern")
                .and_then(|v| v.as_str())
                .or(self.write_pattern.as_deref())
                .ok_or(Error::from("Missing write_pattern for storage backend"))?;

             let bytes = self.write_bytes(options, &envelope.original_data)?;
             let path = self.resolve_write_path(pattern, &envelope)?;

             if self.write_content(options).map_err(Error::from)? == WriteContent::DicomPart10 {
                 Self::put_dicom_instance(store.as_ref(), &path, bytes::Bytes::from(bytes)).await?;
             } else {
                 store.put(&path, bytes.into()).await.map_err(Self::write_error)?;
             }
             
             // Return location
             let mut headers = HashMap::new();
             let root = options
                .get("root")
                .and_then(|v| v.as_str())
                .or(self.root.as_deref())
                .unwrap_or("./tmp/disk");
             
             // Location: scheme://root/path
             // If root is s3://bucket, path is key.
             // If root is ./tmp/disk, path is relative to it?
             // LocalFileSystem from object_store treats prefix as root. So path is relative.
             // We construct location as root + / + path
             let separator = if root.ends_with('/') { "" } else { "/" };
             let location = format!("{}{}{}", root, separator, path);
             
             headers.insert("Location".to_string(), location.clone());
             
             // Response body
             let body_json = serde_json::json!({
                 "location": location,
                 "path": path.to_string(),
                 "status": "stored"
             });
             
             let body = serde_json::to_vec(&body_json).unwrap();
             headers.insert("content-type".to_string(), "application/json".to_string());

             Ok(ResponseEnvelope::from_backend_with_command(
                 envelope.request_details.clone(),
                 201, // Created
                 headers,
                 body,
                 None,
                 envelope.command.clone(),
             ))
        }
    }

    async fn endpoint_outgoing_response(
        &self,
        envelope: ResponseEnvelope<Vec<u8>>,
        _options: &HashMap<String, Value>,
    ) -> Result<Response, Error> {
         let status = http::StatusCode::from_u16(envelope.response_details.status)
            .unwrap_or(http::StatusCode::OK);

        let mut builder = Response::builder().status(status);

        for (k, v) in &envelope.response_details.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let body = if !envelope.original_data.is_empty() {
            Body::from(envelope.original_data)
        } else {
            Body::empty()
        };

        builder
            .body(body)
            .map_err(|_| Error::from("Failed to construct Disk HTTP response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;
    use tempfile::tempdir;

    fn create_test_envelope() -> RequestEnvelope<Vec<u8>> {
        RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/test")
            .original_data(vec![])
            .build()
            .unwrap()
    }

    #[test]
    fn test_resolve_path_simple() {
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String("/tmp".to_string()));
        let envelope = create_test_envelope();

        let path = backend.resolve_path("test.txt", &envelope, &options).unwrap();
        assert_eq!(path.as_ref(), "test.txt");
    }

    #[test]
    fn test_resolve_path_metadata() {
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String("/tmp".to_string()));
        let mut envelope = create_test_envelope();
        envelope.request_details.metadata.insert("tenant".to_string(), "acme".to_string());

        let path = backend.resolve_path("{tenant}/file.txt", &envelope, &options).unwrap();
        assert_eq!(path.as_ref(), "acme/file.txt");
    }

    #[test]
    fn test_resolve_path_normalized_data() {
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String("/tmp".to_string()));
        let mut envelope = create_test_envelope();
        envelope.normalized_data = Some(serde_json::json!({
            "PatientID": "12345"
        }));

        let path = backend.resolve_path("{PatientID}.dcm", &envelope, &options).unwrap();
        assert_eq!(path.as_ref(), "12345.dcm");
    }

    #[test]
    fn test_resolve_path_uuid_timestamp() {
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String("/tmp".to_string()));
        let envelope = create_test_envelope();

        let path = backend.resolve_path("{uuid}_{timestamp}.txt", &envelope, &options).unwrap();
        let path_str = path.to_string();
        // Simple check for UUID structure (not exhaustive regex)
        assert!(!path_str.contains("{uuid}"));
        assert!(!path_str.contains("{timestamp}"));
    }

    #[tokio::test]
    async fn test_write_read_cycle() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String(root.clone()));
        options.insert("write_pattern".to_string(), Value::String("{filename}".to_string()));
        options.insert("read_pattern".to_string(), Value::String("{filename}".to_string()));

        // Write
        let mut write_env = create_test_envelope();
        write_env.request_details.method = "POST".to_string();
        write_env.request_details.metadata.insert("filename".to_string(), "test.txt".to_string());
        write_env.original_data = b"Hello World".to_vec();

        let write_resp = backend.backend_outgoing_request(write_env, &options).await.unwrap();
        assert_eq!(write_resp.response_details.status, 201);
        
        // Check file exists (LocalFileSystem should put it relative to root)
        let file_path = dir.path().join("test.txt");
        assert!(file_path.exists());
        assert_eq!(std::fs::read(&file_path).unwrap(), b"Hello World");

        // Read
        let mut read_env = create_test_envelope();
        read_env.request_details.method = "GET".to_string();
        read_env.request_details.metadata.insert("filename".to_string(), "test.txt".to_string());

        let read_resp = backend.backend_outgoing_request(read_env, &options).await.unwrap();
        assert_eq!(read_resp.response_details.status, 200);
        assert_eq!(read_resp.original_data, b"Hello World");
    }
    
    #[tokio::test]
    async fn test_read_not_found() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        
        let backend = StorageBackend::default();
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String(root));
        options.insert("read_pattern".to_string(), Value::String("missing.txt".to_string()));

        let mut read_env = create_test_envelope();
        read_env.request_details.method = "GET".to_string();

        let read_resp = backend.backend_outgoing_request(read_env, &options).await.unwrap();
        assert_eq!(read_resp.response_details.status, 404);
    }

    fn fake_part10() -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes.extend_from_slice(b"DICM");
        bytes.extend_from_slice(&[1u8; 64]);
        bytes
    }

    fn part10_options(root: &str) -> HashMap<String, Value> {
        let mut options = HashMap::new();
        options.insert("root".to_string(), Value::String(root.to_string()));
        options.insert("content".to_string(), Value::String("dicom_part10".to_string()));
        options.insert(
            "write_pattern".to_string(),
            Value::String("{StudyInstanceUID}/{SOPInstanceUID}.dcm".to_string()),
        );
        options
    }

    fn store_envelope(body: Value) -> RequestEnvelope<Vec<u8>> {
        let mut envelope = create_test_envelope();
        envelope.request_details.method = "POST".to_string();
        let metadata = &mut envelope.request_details.metadata;
        metadata.insert("StudyInstanceUID".to_string(), "1.2.3".to_string());
        metadata.insert("SOPInstanceUID".to_string(), "1.2.3.4".to_string());
        envelope.original_data = serde_json::to_vec(&body).unwrap();
        envelope
    }

    #[tokio::test]
    async fn test_dicom_part10_writes_the_inline_instance() {
        use base64::Engine;
        let dir = tempdir().unwrap();
        let options = part10_options(&dir.path().to_string_lossy());
        let part10 = fake_part10();
        let envelope = store_envelope(serde_json::json!({
            "dataset": {},
            "file": "/somewhere/else.dcm",
            "part10_b64": base64::engine::general_purpose::STANDARD.encode(&part10),
        }));

        let response = StorageBackend::default()
            .backend_outgoing_request(envelope, &options)
            .await
            .unwrap();

        assert_eq!(response.response_details.status, 201);
        let written = std::fs::read(dir.path().join("1.2.3").join("1.2.3.4.dcm")).unwrap();
        assert_eq!(written, part10);
    }

    #[tokio::test]
    async fn test_dicom_part10_rejects_a_body_that_only_names_a_file() {
        let dir = tempdir().unwrap();
        let options = part10_options(&dir.path().to_string_lossy());
        let envelope = store_envelope(serde_json::json!({
            "dataset": {},
            "file": "/etc/passwd",
        }));

        let err = StorageBackend::default()
            .backend_outgoing_request(envelope, &options)
            .await
            .err()
            .expect("a body with no instance fails the write");

        assert!(err.to_string().contains("'file'"), "unexpected error: {}", err);
        assert!(!dir.path().join("1.2.3").exists(), "nothing is written");
    }

    #[test]
    fn test_validate_rejects_an_unknown_content() {
        let mut options = HashMap::new();
        options.insert("content".to_string(), Value::String("dicom".to_string()));

        let err = StorageBackend::default().validate(&options).err();

        assert!(
            matches!(err, Some(ConfigError::InvalidBackend { ref reason, .. }) if reason.contains("dicom_part10")),
            "unexpected result: {:?}",
            err
        );
        assert!(StorageBackend::default().validate(&HashMap::new()).is_ok());
    }

    fn write_path_with(key: &str, value: &str, pattern: &str) -> Result<ObjectStorePath, Error> {
        let mut envelope = create_test_envelope();
        envelope.request_details.metadata.insert(key.to_string(), value.to_string());
        StorageBackend::default().resolve_write_path(pattern, &envelope)
    }

    #[test]
    fn test_write_path_fills_placeholders_once() {
        let path = write_path_with("PatientID", "{uuid}", "{PatientID}.dcm").unwrap();
        assert_eq!(path.as_ref(), "{uuid}.dcm", "a value is not scanned for placeholders");

        let path = write_path_with("tenant", "acme", "/{tenant}/a-{uuid}.bin").unwrap();
        assert!(path.as_ref().starts_with("acme/a-"));
        assert!(!path.as_ref().contains('{'));
    }

    #[test]
    fn test_write_path_rejects_values_that_leave_their_segment() {
        for (value, reason) in [
            ("../../etc", "path separator"),
            ("a/b", "path separator"),
            ("a\\b", "path separator"),
            ("..", "relative path segment"),
            (".", "relative path segment"),
            ("", "is empty"),
        ] {
            let err = write_path_with("PatientID", value, "{PatientID}/x.dcm")
                .err()
                .unwrap_or_else(|| panic!("value {:?} must be rejected", value));
            assert!(err.to_string().contains(reason), "{:?}: unexpected error: {}", value, err);
        }
    }

    #[test]
    fn test_write_path_rejects_a_placeholder_with_no_value() {
        let err = write_path_with("tenant", "acme", "{SOPInstanceUID}.dcm").err().unwrap();
        assert!(err.to_string().contains("{SOPInstanceUID}"), "unexpected error: {}", err);

        let err = write_path_with("tenant", "acme", "{tenant.dcm").err().unwrap();
        assert!(err.to_string().contains("no closing"), "unexpected error: {}", err);
    }

    #[tokio::test]
    async fn test_dicom_part10_keeps_an_identical_resend_and_refuses_a_different_one() {
        use base64::Engine;
        let dir = tempdir().unwrap();
        let options = part10_options(&dir.path().to_string_lossy());
        let body = |part10: &[u8]| {
            store_envelope(serde_json::json!({
                "part10_b64": base64::engine::general_purpose::STANDARD.encode(part10),
            }))
        };
        let backend = StorageBackend::default();
        let original = fake_part10();

        backend.backend_outgoing_request(body(&original), &options).await.unwrap();
        backend
            .backend_outgoing_request(body(&original), &options)
            .await
            .expect("an identical resend counts as stored");

        let mut other = fake_part10();
        other.push(9);
        let err = backend
            .backend_outgoing_request(body(&other), &options)
            .await
            .err()
            .expect("different bytes at the same path are refused");
        assert!(err.to_string().contains("already stored"), "unexpected error: {}", err);
        let stored = std::fs::read(dir.path().join("1.2.3").join("1.2.3.4.dcm")).unwrap();
        assert_eq!(stored, original);
    }

    #[test]
    fn test_write_path_uuid_cannot_be_set_by_the_request() {
        let mut envelope = create_test_envelope();
        envelope.request_details.metadata.insert("uuid".to_string(), "victim".to_string());
        envelope.normalized_data = Some(serde_json::json!({ "uuid": "victim" }));

        let path = StorageBackend::default()
            .resolve_write_path("files/{uuid}.bin", &envelope)
            .unwrap();

        assert_ne!(path.as_ref(), "files/victim.bin");
        assert!(Uuid::parse_str(path.as_ref().trim_start_matches("files/").trim_end_matches(".bin")).is_ok());
    }
}

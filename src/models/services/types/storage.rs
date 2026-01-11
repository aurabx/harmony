use crate::config::config::ConfigError;
use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::services::services::{ServiceHandler, ServiceType};
use crate::utils::Error;
use async_trait::async_trait;
use axum::{body::Body, response::Response};
use chrono::Utc;
use futures_util::StreamExt;
use object_store::local::LocalFileSystem;
use object_store::{ObjectStore, path::Path as ObjectStorePath};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize, Clone, Default)]
pub struct StorageBackend {
    pub root: Option<String>,
    pub write_pattern: Option<String>,
    pub read_pattern: Option<String>,
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
}

#[async_trait]
impl ServiceType for StorageBackend {
    fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
        Ok(())
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
        let method = envelope.request_details.method.to_uppercase();
        let store = self.get_store(options)?;
        
        if method == "GET" {
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

                     Ok(ResponseEnvelope::from_backend(
                         envelope.request_details.clone(),
                         200,
                         headers,
                         bytes,
                         None
                     ))
                 },
                 Err(object_store::Error::NotFound { .. }) => {
                     Ok(ResponseEnvelope::from_backend(
                         envelope.request_details.clone(),
                         404,
                         HashMap::new(),
                         b"File not found".to_vec(),
                         None
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

             let path = self.resolve_path(pattern, &envelope, options)?;
             
             store.put(&path, envelope.original_data.clone().into()).await
                .map_err(|e| Error::from(format!("Failed to write to storage: {}", e)))?;
             
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

             Ok(ResponseEnvelope::from_backend(
                 envelope.request_details.clone(),
                 201, // Created
                 headers,
                 body,
                 None
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
}

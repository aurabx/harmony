//! Streaming service support
//!
//! This module provides traits and types for services that support streaming responses,
//! allowing backends to yield multiple ResponseEnvelopes instead of a single response.
//!
//! ## Use Case
//!
//! Streaming is useful for operations that produce multiple data items over time, such as:
//! - DICOM C-GET operations that retrieve multiple instances
//! - Database query results with many rows
//! - File system directory listings
//!
//! ## Architecture
//!
//! ```text
//! Backend Stream → Middleware (per-envelope) → Endpoint Stream → Client
//! ```
//!
//! Each envelope in the stream passes through the full middleware chain independently,
//! enabling per-item transforms, authentication checks, and filtering.

use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::utils::Error;
use async_trait::async_trait;
use futures_util::Stream;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;

/// Trait for backends that support streaming responses
///
/// Streaming backends yield a stream of ResponseEnvelopes instead of a single response.
/// Each envelope represents one logical item (e.g., a DICOM instance, a database row).
///
/// # Example
///
/// ```ignore
/// struct DicomScuStreamingBackend { /* ... */ }
///
/// #[async_trait]
/// impl StreamingBackend for DicomScuStreamingBackend {
///     async fn backend_outgoing_stream(
///         &self,
///         envelope: RequestEnvelope<Vec<u8>>,
///         options: &HashMap<String, Value>,
///     ) -> Result<ResponseStream, Error> {
///         // Perform DICOM C-GET operation
///         let mut stream = scu.get_request(&remote_node, query, None).await?;
///         
///         // Convert DIMSE stream to ResponseEnvelope stream
///         let envelope_stream = stream.map(move |result| {
///             match result {
///                 Ok(instance) => {
///                     // Create ResponseEnvelope for this instance
///                     Ok(ResponseEnvelope::from_backend(/* ... */))
///                 }
///                 Err(e) => Err(Error::from(e)),
///             }
///         });
///         
///         Ok(Box::pin(envelope_stream))
///     }
/// }
/// ```
#[async_trait]
pub trait StreamingBackend: Send + Sync {
    /// Execute a backend request and return a stream of responses
    ///
    /// # Arguments
    ///
    /// * `envelope` - The request envelope
    /// * `options` - Backend configuration options
    ///
    /// # Returns
    ///
    /// A stream of ResponseEnvelopes, where each envelope represents one logical item.
    /// The stream should complete normally when all items have been yielded, or produce
    /// an error if the operation fails.
    ///
    /// # Error Handling
    ///
    /// - Stream initialization errors should be returned as `Err(Error)`
    /// - Per-item errors should be yielded as `Err(Error)` in the stream
    /// - The stream should terminate after yielding an error
    async fn backend_outgoing_stream(
        &self,
        envelope: RequestEnvelope<Vec<u8>>,
        options: &HashMap<String, Value>,
    ) -> Result<ResponseStream, Error>;
}

/// Type alias for response envelope streams
pub type ResponseStream =
    Pin<Box<dyn Stream<Item = Result<ResponseEnvelope<Vec<u8>>, Error>> + Send>>;

/// Metadata about a streaming operation
///
/// This can be included in the first ResponseEnvelope's metadata to provide
/// information about the stream to downstream components.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamMetadata {
    /// Unique identifier for this stream session
    pub stream_id: String,

    /// Expected number of items in the stream (if known)
    pub expected_items: Option<usize>,

    /// Total expected size in bytes (if known)
    pub expected_bytes: Option<u64>,

    /// Stream operation type (e.g., "dimse-get", "dimse-find", "query-results")
    pub operation: String,

    /// Additional operation-specific metadata
    pub metadata: HashMap<String, String>,
}

impl StreamMetadata {
    /// Create a new StreamMetadata instance
    pub fn new(operation: impl Into<String>) -> Self {
        Self {
            stream_id: uuid::Uuid::new_v4().to_string(),
            expected_items: None,
            expected_bytes: None,
            operation: operation.into(),
            metadata: HashMap::new(),
        }
    }

    /// Set the expected number of items
    pub fn with_expected_items(mut self, count: usize) -> Self {
        self.expected_items = Some(count);
        self
    }

    /// Set the expected total size in bytes
    pub fn with_expected_bytes(mut self, bytes: u64) -> Self {
        self.expected_bytes = Some(bytes);
        self
    }

    /// Add operation-specific metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_metadata_creation() {
        let metadata = StreamMetadata::new("test-operation")
            .with_expected_items(10)
            .with_expected_bytes(1024)
            .with_metadata("key", "value");

        assert_eq!(metadata.operation, "test-operation");
        assert_eq!(metadata.expected_items, Some(10));
        assert_eq!(metadata.expected_bytes, Some(1024));
        assert_eq!(metadata.metadata.get("key"), Some(&"value".to_string()));
    }
}

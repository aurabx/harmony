/// Common path handling utilities for service implementations
use crate::models::envelope::envelope::RequestEnvelope;

/// Extract the path WITHOUT query parameters from request metadata
///
/// This helper function provides consistent path extraction across all service types.
/// It prefers `path` (without query string), as the query parameters should be 
/// managed via the `query_params` hash in RequestDetails/TargetDetails.
///
/// # Arguments
/// * `envelope` - The request envelope containing metadata
///
/// # Returns
/// A string containing the path without query string, with a leading slash if not already present
///
/// # Example
/// ```ignore
/// // For a request to /fhir/Patient?_count=5 with endpoint prefix "/fhir"
/// // Returns: "/Patient" (query params are in envelope.request_details.query_params)
/// use harmony::models::services::path_utils::extract_path;
/// let path = extract_path(&envelope);
/// ```
pub fn extract_path(envelope: &RequestEnvelope<Vec<u8>>) -> String {
    envelope
        .request_details
        .metadata
        .get("path")
        .map(|p| {
            if p.starts_with('/') {
                p.clone()
            } else {
                format!("/{}", p)
            }
        })
        .unwrap_or_else(|| {
            // Fallback: extract path from URI (strip query string if present)
            let uri = &envelope.request_details.uri;
            uri.split('?').next().unwrap_or(uri).to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestEnvelope;
    use std::collections::HashMap;

    #[test]
    fn test_extract_path() {
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "/Patient".to_string());

        let envelope = RequestEnvelope::builder()
            .method("GET".to_string())
            .uri("/fhir/Patient?_count=5".to_string())
            .metadata(metadata)
            .original_data(vec![])
            .build()
            .unwrap();

        let result = extract_path(&envelope);
        assert_eq!(result, "/Patient");
    }

    #[test]
    fn test_extract_path_adds_leading_slash() {
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), "Patient".to_string());

        let envelope = RequestEnvelope::builder()
            .method("GET".to_string())
            .uri("/fhir/Patient".to_string())
            .metadata(metadata)
            .original_data(vec![])
            .build()
            .unwrap();

        let result = extract_path(&envelope);
        assert_eq!(result, "/Patient");
    }

    #[test]
    fn test_extract_path_fallback_to_uri() {
        let metadata = HashMap::new();

        let envelope = RequestEnvelope::builder()
            .method("GET".to_string())
            .uri("/fhir/Patient?_count=5".to_string())
            .metadata(metadata)
            .original_data(vec![])
            .build()
            .unwrap();

        let result = extract_path(&envelope);
        // Should strip query string from URI
        assert_eq!(result, "/fhir/Patient");
    }

    #[test]
    fn test_extract_path_uri_without_query() {
        let metadata = HashMap::new();

        let envelope = RequestEnvelope::builder()
            .method("GET".to_string())
            .uri("/api/users".to_string())
            .metadata(metadata)
            .original_data(vec![])
            .build()
            .unwrap();

        let result = extract_path(&envelope);
        assert_eq!(result, "/api/users");
    }
}

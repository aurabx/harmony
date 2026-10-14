/// Common path handling utilities for service implementations
use crate::models::envelope::envelope::RequestEnvelope;
use std::collections::HashMap;

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
/// ```text
/// For a request to /fhir/Patient?_count=5 with endpoint prefix "/fhir"
/// Returns: "/Patient" (query params are in envelope.request_details.query_params)
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

/// Build the request target an HTTP/3 backend sends: `base_path`, then the
/// request URI, then the query string.
///
/// The HTTP/3 client takes host and port separately, so unlike the reqwest
/// path it cannot be handed `TargetDetails::full_url`. This keeps the same
/// joining rules: `base_path` gets a leading slash and no trailing slash, and
/// exactly one slash separates it from the URI.
///
/// # Arguments
/// * `base_path` - The connection's optional `base_path`
/// * `uri` - The path of the request, without a query string
/// * `query_params` - Query parameters to append, URL-encoded
pub fn http3_request_path(
    base_path: Option<&str>,
    uri: &str,
    query_params: &HashMap<String, Vec<String>>,
) -> String {
    let prefix = base_path
        .map(|p| p.trim().trim_matches('/'))
        .filter(|p| !p.is_empty())
        .map(|p| format!("/{}", p))
        .unwrap_or_default();
    let suffix = uri.trim_start_matches('/');

    let mut path = if suffix.is_empty() {
        if prefix.is_empty() {
            "/".to_string()
        } else {
            prefix
        }
    } else {
        format!("{}/{}", prefix, suffix)
    };

    let query_string = query_params
        .iter()
        .flat_map(|(key, values)| {
            values.iter().map(move |value| {
                format!(
                    "{}={}",
                    urlencoding::encode(key),
                    urlencoding::encode(value)
                )
            })
        })
        .collect::<Vec<_>>()
        .join("&");
    if !query_string.is_empty() {
        path.push('?');
        path.push_str(&query_string);
    }
    path
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

    #[test]
    fn http3_path_joins_base_path_and_uri() {
        let none = HashMap::new();
        assert_eq!(
            http3_request_path(Some("/api"), "/data.json", &none),
            "/api/data.json"
        );
        assert_eq!(
            http3_request_path(Some("api/"), "data.json", &none),
            "/api/data.json"
        );
        assert_eq!(http3_request_path(Some("/v1/"), "//x", &none), "/v1/x");
        assert_eq!(http3_request_path(None, "/data.json", &none), "/data.json");
        assert_eq!(
            http3_request_path(Some(""), "/data.json", &none),
            "/data.json"
        );
        assert_eq!(
            http3_request_path(Some("/"), "/data.json", &none),
            "/data.json"
        );
    }

    #[test]
    fn http3_path_with_empty_uri_is_base_path_or_root() {
        let none = HashMap::new();
        assert_eq!(http3_request_path(Some("/api"), "", &none), "/api");
        assert_eq!(http3_request_path(Some("/api"), "/", &none), "/api");
        assert_eq!(http3_request_path(None, "", &none), "/");
    }

    #[test]
    fn http3_path_appends_encoded_query() {
        let mut query = HashMap::new();
        query.insert(
            "name".to_string(),
            vec!["a b".to_string(), "c&d".to_string()],
        );
        let path = http3_request_path(Some("/api"), "/p", &query);
        assert!(path.starts_with("/api/p?"), "{}", path);
        assert!(path.contains("name=a%20b"), "{}", path);
        assert!(path.contains("name=c%26d"), "{}", path);
    }
}

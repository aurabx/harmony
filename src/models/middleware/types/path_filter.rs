use crate::models::envelope::envelope::RequestEnvelope;
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use matchit::Router;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct PathFilterConfig {
    /// List of path patterns to allow using matchit syntax
    pub rules: Vec<String>,
}

pub struct PathFilterMiddleware {
    router: Router<()>,
}

impl PathFilterMiddleware {
    pub fn new(config: PathFilterConfig) -> Result<Self, String> {
        let mut router = Router::new();
        
        if config.rules.is_empty() {
            return Err("PathFilter requires at least one rule".to_string());
        }
        
        for rule in &config.rules {
            tracing::trace!("Loading path filter rule: {}", rule);
            if let Err(e) = router.insert(rule, ()) {
                return Err(format!("Failed to insert path filter rule '{}': {}", rule, e));
            }
        }
        
        tracing::info!("PathFilter initialized with {} rules", config.rules.len());
        Ok(Self { router })
    }
}

#[async_trait]
impl Middleware for PathFilterMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Get the subpath from request metadata
        let subpath = envelope
            .request_details
            .metadata
            .get("path")
            .cloned()
            .unwrap_or_default();
        
        // Normalize path: ensure leading slash, use "/" if empty
        let normalized_path = if subpath.is_empty() {
            "/".to_string()
        } else if !subpath.starts_with('/') {
            format!("/{}", subpath)
        } else {
            subpath.clone()
        };
        
        // Trim trailing slash except for root
        let path_to_match = if normalized_path != "/" && normalized_path.ends_with('/') {
            normalized_path.trim_end_matches('/').to_string()
        } else {
            normalized_path
        };
        
        tracing::debug!("PathFilter evaluating path: {}", path_to_match);
        
        // Try to match the path
        if let Ok(_) = self.router.at(&path_to_match) {
            tracing::debug!("PathFilter: path '{}' matched, allowing request", path_to_match);
            Ok(envelope)
        } else {
            tracing::warn!("PathFilter: path '{}' rejected - no matching rule", path_to_match);
            
            // Set skip_backends flag and 404 response
            envelope.request_details.metadata.insert("skip_backends".to_string(), "true".to_string());
            envelope.normalized_data = Some(serde_json::json!({
                "response": {
                    "status": 404,
                    "body": ""
                }
            }));
            
            Ok(envelope)
        }
    }

    async fn right(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
    ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
        // Path filtering only applies on the left (incoming requests)
        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(
    options: &HashMap<String, Value>,
) -> Result<PathFilterConfig, String> {
    let rules: Vec<String> = options
        .get("rules")
        .and_then(|v| v.as_array())
        .ok_or("Missing required 'rules' array in path_filter middleware config")?
        .iter()
        .map(|v| v.as_str().ok_or("All rules must be strings"))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    if rules.is_empty() {
        return Err("PathFilter requires at least one rule".to_string());
    }

    Ok(PathFilterConfig { rules })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestDetails;
    use std::collections::HashMap;

    fn create_test_envelope(path: &str) -> RequestEnvelope<serde_json::Value> {
        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), path.to_string());
        
        let request_details = RequestDetails {
            method: "GET".to_string(),
            uri: "/test".to_string(),
            headers: HashMap::new(),
            cookies: HashMap::new(),
            query_params: HashMap::new(),
            cache_status: None,
            metadata,
        };

        RequestEnvelope {
            request_details,
            original_data: serde_json::Value::Null,
            normalized_data: Some(serde_json::Value::Null),
            normalized_snapshot: None,
        }
    }

    #[tokio::test]
    async fn test_matches_exact_route_passes() {
        let config = PathFilterConfig {
            rules: vec!["/ImagingStudy".to_string()],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();
        
        let envelope = create_test_envelope("ImagingStudy");
        let result = middleware.left(envelope).await.unwrap();
        
        // Should not set skip_backends
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
        // Should not modify normalized_data to include response
        assert!(!result.normalized_data.as_ref().unwrap().get("response").is_some());
    }

    #[tokio::test]
    async fn test_non_matching_returns_404_and_skips_backends() {
        let config = PathFilterConfig {
            rules: vec!["/ImagingStudy".to_string()],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();
        
        let envelope = create_test_envelope("ImagingStudy/series");
        let result = middleware.left(envelope).await.unwrap();
        
        // Should set skip_backends
        assert_eq!(result.request_details.metadata.get("skip_backends"), Some(&"true".to_string()));
        
        // Should set 404 response
        let response = result.normalized_data.as_ref().unwrap().get("response").unwrap();
        assert_eq!(response.get("status").unwrap().as_u64().unwrap(), 404);
        assert_eq!(response.get("body").unwrap().as_str().unwrap(), "");
    }

    #[tokio::test]
    async fn test_trailing_slash_handling() {
        let config = PathFilterConfig {
            rules: vec!["/ImagingStudy".to_string()],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();
        
        // Test that "ImagingStudy/" matches "/ImagingStudy"
        let envelope = create_test_envelope("ImagingStudy/");
        let result = middleware.left(envelope).await.unwrap();
        
        // Should not set skip_backends (should match)
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_empty_path_becomes_root() {
        let config = PathFilterConfig {
            rules: vec!["/".to_string()],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();
        
        let envelope = create_test_envelope("");
        let result = middleware.left(envelope).await.unwrap();
        
        // Should not set skip_backends (should match root)
        assert!(!result.request_details.metadata.contains_key("skip_backends"));
    }

    #[test]
    fn test_parse_config() {
        let mut options = HashMap::new();
        options.insert("rules".to_string(), serde_json::json!(["/ImagingStudy", "/Patient"]));
        
        let config = parse_config(&options).unwrap();
        assert_eq!(config.rules, vec!["/ImagingStudy", "/Patient"]);
    }

    #[test]
    fn test_parse_config_missing_rules() {
        let options = HashMap::new();
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required 'rules'"));
    }

    #[test]
    fn test_parse_config_empty_rules() {
        let mut options = HashMap::new();
        options.insert("rules".to_string(), serde_json::json!([]));
        
        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires at least one rule"));
    }
}
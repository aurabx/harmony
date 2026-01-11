use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::models::middleware::PathDenied;
use crate::utils::Error;
use async_trait::async_trait;
use matchit::Router;
use serde_json::Value;
use std::collections::HashMap;

/// Represents a path filter rule with either allow or deny action
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathFilterRule {
    /// Allow requests matching this pattern
    Allow(String),
    /// Deny requests matching this pattern
    Deny(String),
}

impl PathFilterRule {
    /// Returns the pattern regardless of rule type
    pub fn pattern(&self) -> &str {
        match self {
            PathFilterRule::Allow(pattern) => pattern,
            PathFilterRule::Deny(pattern) => pattern,
        }
    }

    /// Returns true if this is an allow rule
    pub fn is_allow(&self) -> bool {
        matches!(self, PathFilterRule::Allow(_))
    }

    /// Returns true if this is a deny rule
    pub fn is_deny(&self) -> bool {
        matches!(self, PathFilterRule::Deny(_))
    }
}

#[derive(Debug, Clone)]
pub struct PathFilterConfig {
    /// List of allow/deny rules with matchit patterns
    /// Rules are evaluated in order (first match wins)
    pub rules: Vec<PathFilterRule>,
}

pub struct PathFilterMiddleware {
    routers: Vec<(PathFilterRule, Router<()>)>,
}

impl PathFilterMiddleware {
    pub fn new(config: PathFilterConfig) -> Result<Self, String> {
        if config.rules.is_empty() {
            return Err("PathFilter requires at least one rule".to_string());
        }

        // Build a router for each rule to maintain order and rule type
        let mut routers = Vec::new();
        for rule in &config.rules {
            let mut router = Router::new();
            let pattern = rule.pattern();

            tracing::trace!(
                "Loading path filter rule: {:?} {}",
                if rule.is_allow() { "allow" } else { "deny" },
                pattern
            );

            if let Err(e) = router.insert(pattern, ()) {
                return Err(format!(
                    "Failed to insert path filter rule '{}': {}",
                    pattern, e
                ));
            }
            routers.push((rule.clone(), router));
        }

        tracing::info!("PathFilter initialized with {} rules", config.rules.len());
        Ok(Self { routers })
    }
}

#[async_trait]
impl Middleware for PathFilterMiddleware {
    async fn left(
        &self,
        envelope: RequestEnvelope<serde_json::Value>,
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

        // First-match-wins: iterate through rules in order
        for (rule, router) in &self.routers {
            if router.at(&path_to_match).is_ok() {
                match rule {
                    PathFilterRule::Allow(_) => {
                        tracing::debug!(
                            "PathFilter: path '{}' matched allow rule '{}', allowing request",
                            path_to_match,
                            rule.pattern()
                        );
                        return Ok(envelope);
                    }
                    PathFilterRule::Deny(_) => {
                        tracing::warn!(
                            "PathFilter: path '{}' matched deny rule '{}', rejecting request",
                            path_to_match,
                            rule.pattern()
                        );
                        return Err(Box::new(PathDenied(path_to_match)) as Error);
                    }
                }
            }
        }

        // No rules matched - implicit deny
        tracing::warn!(
            "PathFilter: path '{}' rejected - no matching rule (implicit deny)",
            path_to_match
        );

        Err(Box::new(PathDenied(path_to_match)) as Error)
    }

    async fn right(
        &self,
        envelope: ResponseEnvelope<serde_json::Value>,
    ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
        // Path filtering only applies on the left (incoming requests)
        Ok(envelope)
    }
}

/// Parse configuration from HashMap for middleware registry
pub fn parse_config(options: &HashMap<String, Value>) -> Result<PathFilterConfig, String> {
    let rules_array = options
        .get("rules")
        .and_then(|v| v.as_array())
        .ok_or("Missing required 'rules' array in path_filter middleware config")?;

    let mut rules = Vec::new();

    for (idx, rule_value) in rules_array.iter().enumerate() {
        let rule_obj = rule_value.as_object().ok_or_else(|| {
            format!(
                "Rule at index {} must be an object with 'allow' or 'deny' key",
                idx
            )
        })?;

        // Check for "allow" key
        if let Some(allow_val) = rule_obj.get("allow") {
            let pattern = allow_val
                .as_str()
                .ok_or_else(|| format!("Rule at index {}: 'allow' value must be a string", idx))?;

            if pattern.trim().is_empty() {
                return Err(format!(
                    "Rule at index {}: 'allow' pattern cannot be empty",
                    idx
                ));
            }

            // Check that "deny" is not also present
            if rule_obj.contains_key("deny") {
                return Err(format!(
                    "Rule at index {}: cannot have both 'allow' and 'deny' keys",
                    idx
                ));
            }

            rules.push(PathFilterRule::Allow(pattern.to_string()));
        }
        // Check for "deny" key
        else if let Some(deny_val) = rule_obj.get("deny") {
            let pattern = deny_val
                .as_str()
                .ok_or_else(|| format!("Rule at index {}: 'deny' value must be a string", idx))?;

            if pattern.trim().is_empty() {
                return Err(format!(
                    "Rule at index {}: 'deny' pattern cannot be empty",
                    idx
                ));
            }

            rules.push(PathFilterRule::Deny(pattern.to_string()));
        } else {
            return Err(format!(
                "Rule at index {}: must have either 'allow' or 'deny' key",
                idx
            ));
        }
    }

    if rules.is_empty() {
        return Err("PathFilter requires at least one rule".to_string());
    }

    Ok(PathFilterConfig { rules })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::envelope::envelope::RequestEnvelopeBuilder;
    use std::collections::HashMap;

    fn create_test_envelope(path: &str) -> RequestEnvelope<serde_json::Value> {
        RequestEnvelopeBuilder::new()
            .method("GET")
            .uri("/test")
            .metadata_entry("path", path)
            .original_data(serde_json::Value::Null)
            .normalized_data(Some(serde_json::Value::Null))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_matches_exact_route_passes() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/ImagingStudy".to_string()),
                PathFilterRule::Deny("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        let envelope = create_test_envelope("ImagingStudy");
        let result = middleware.left(envelope).await.unwrap();

        // Should not set skip_backends
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
        // Should not modify normalized_data to include response
        assert!(!result
            .normalized_data
            .as_ref()
            .unwrap()
            .get("response")
            .is_some());
    }

    #[tokio::test]
    async fn test_non_matching_returns_path_denied_error() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/ImagingStudy".to_string()),
                PathFilterRule::Deny("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        let envelope = create_test_envelope("ImagingStudy/series");
        let result = middleware.left(envelope).await;

        // Should return a PathDenied error for the normalized path
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/ImagingStudy/series");
    }

    #[tokio::test]
    async fn test_trailing_slash_handling() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/ImagingStudy".to_string()),
                PathFilterRule::Deny("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // Test that "ImagingStudy/" matches "/ImagingStudy"
        let envelope = create_test_envelope("ImagingStudy/");
        let result = middleware.left(envelope).await.unwrap();

        // Should not set skip_backends (should match)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_empty_path_becomes_root() {
        let config = PathFilterConfig {
            rules: vec![PathFilterRule::Allow("/".to_string())],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        let envelope = create_test_envelope("");
        let result = middleware.left(envelope).await.unwrap();

        // Should not set skip_backends (should match root)
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[test]
    fn test_parse_config() {
        let mut options = HashMap::new();
        options.insert(
            "rules".to_string(),
            serde_json::json!([
                { "allow": "/ImagingStudy" },
                { "allow": "/Patient" },
                { "deny": "/{*rest}" }
            ]),
        );

        let config = parse_config(&options).unwrap();
        assert_eq!(config.rules.len(), 3);
        assert!(matches!(config.rules[0], PathFilterRule::Allow(_)));
        assert_eq!(config.rules[0].pattern(), "/ImagingStudy");
        assert!(matches!(config.rules[1], PathFilterRule::Allow(_)));
        assert_eq!(config.rules[1].pattern(), "/Patient");
        assert!(matches!(config.rules[2], PathFilterRule::Deny(_)));
        assert_eq!(config.rules[2].pattern(), "/{*rest}");
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

    #[test]
    fn test_parse_config_invalid_rule_format() {
        let mut options = HashMap::new();
        options.insert(
            "rules".to_string(),
            serde_json::json!(["/invalid"]), // Should be object, not string
        );

        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be an object"));
    }

    #[test]
    fn test_parse_config_both_allow_and_deny() {
        let mut options = HashMap::new();
        options.insert(
            "rules".to_string(),
            serde_json::json!([{ "allow": "/test", "deny": "/test" }]),
        );

        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot have both"));
    }

    #[test]
    fn test_parse_config_neither_allow_nor_deny() {
        let mut options = HashMap::new();
        options.insert("rules".to_string(), serde_json::json!([{ "foo": "/test" }]));

        let result = parse_config(&options);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("must have either 'allow' or 'deny' key"));
    }

    // New tests for deny rules and first-match-wins behavior

    #[tokio::test]
    async fn test_deny_rule_blocks_request() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Deny("/admin".to_string()),
                PathFilterRule::Allow("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        let envelope = create_test_envelope("admin");
        let result = middleware.left(envelope).await;

        // Should return a PathDenied error for the normalized path
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/admin");
    }

    #[tokio::test]
    async fn test_first_match_wins_allow_then_deny() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/api/public/{*path}".to_string()),
                PathFilterRule::Deny("/api/{*path}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // First allow rule should match
        let envelope = create_test_envelope("api/public/data");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Second deny rule should match and return PathDenied
        let envelope = create_test_envelope("api/private/data");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/api/private/data");
    }

    #[tokio::test]
    async fn test_first_match_wins_deny_then_allow() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Deny("/admin/{*path}".to_string()),
                PathFilterRule::Allow("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // First deny rule should match and return PathDenied
        let envelope = create_test_envelope("admin/users");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/admin/users");

        // Second allow rule should match
        let envelope = create_test_envelope("api/data");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_implicit_deny_when_no_match() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/health".to_string()),
                PathFilterRule::Allow("/api/public".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // Allowed path should pass
        let envelope = create_test_envelope("health");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Unmatched path should be denied (implicit deny via PathDenied)
        let envelope = create_test_envelope("api/private");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/api/private");
    }

    #[tokio::test]
    async fn test_deny_all_with_specific_allows() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/health".to_string()),
                PathFilterRule::Allow("/api/public/{*path}".to_string()),
                PathFilterRule::Deny("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // Allowed paths should pass
        let envelope = create_test_envelope("health");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        let envelope = create_test_envelope("api/public/test");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Other paths should be denied by catch-all via PathDenied
        let envelope = create_test_envelope("api/private");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/api/private");
    }

    #[tokio::test]
    async fn test_wildcard_patterns_in_deny_rules() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Deny("/internal/{*path}".to_string()),
                PathFilterRule::Allow("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // Internal paths should be denied via PathDenied
        let envelope = create_test_envelope("internal/admin");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/internal/admin");

        // Other paths should be allowed
        let envelope = create_test_envelope("api/data");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));
    }

    #[tokio::test]
    async fn test_parameter_patterns_work_for_both_allow_and_deny() {
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Allow("/users/{id}".to_string()),
                PathFilterRule::Deny("/users/{*path}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        // Parameter pattern should match and allow
        let envelope = create_test_envelope("users/123");
        let result = middleware.left(envelope).await.unwrap();
        assert!(!result
            .request_details
            .metadata
            .contains_key("skip_backends"));

        // Deny with parameter pattern
        let config = PathFilterConfig {
            rules: vec![
                PathFilterRule::Deny("/admin/{path}".to_string()),
                PathFilterRule::Allow("/{*rest}".to_string()),
            ],
        };
        let middleware = PathFilterMiddleware::new(config).unwrap();

        let envelope = create_test_envelope("admin/users");
        let result = middleware.left(envelope).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let path_err = err
            .downcast::<PathDenied>()
            .expect("expected PathDenied error");
        assert_eq!(path_err.0, "/admin/users");
    }
}

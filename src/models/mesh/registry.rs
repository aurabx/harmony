//! Mesh Registry - URL-based routing for mesh ingress definitions.
//!
//! The MeshRegistry provides priority routing for mesh requests. When a request's
//! URL (scheme + host + path) matches an ingress URL, the request is routed to
//! the ingress's endpoint instead of following normal routing rules.

use super::config::{Mesh, MeshIngress};
use crate::config::config::Config;
use std::collections::HashMap;
use url::Url;

/// Context attached to requests that match a mesh ingress.
#[derive(Debug, Clone)]
pub struct MeshContext {
    /// Name of the matched mesh
    pub mesh_name: String,
    /// The mesh configuration
    pub mesh: Mesh,
    /// Name of the matched ingress
    pub ingress_name: String,
    /// The ingress configuration
    pub ingress: MeshIngress,
    /// The pipeline that contains the ingress endpoint
    pub pipeline_name: String,
}

/// Result of a mesh routing lookup.
#[derive(Debug, Clone)]
pub struct MeshRouteMatch {
    /// The endpoint to route to
    pub endpoint_name: String,
    /// The pipeline containing the endpoint
    pub pipeline_name: String,
    /// Full mesh context for downstream use
    pub context: MeshContext,
}

/// Registry for URL-to-ingress routing.
///
/// Built from config at startup, provides O(n) URL matching where n is the
/// number of ingress URLs (could be optimized with a trie if needed).
pub struct MeshRegistry {
    /// Map of parsed ingress URLs to (ingress_name, optional mesh_name)
    url_index: Vec<(UrlPattern, String, Option<String>)>,
    /// Reference to ingress definitions
    ingress: HashMap<String, MeshIngress>,
    /// Reference to mesh definitions  
    mesh: HashMap<String, Mesh>,
}

/// Parsed URL pattern for matching.
#[derive(Debug, Clone)]
struct UrlPattern {
    scheme: Option<String>,
    host: String,
    port: Option<u16>,
    path_prefix: String,
}

impl UrlPattern {
    fn from_url_str(url_str: &str) -> Option<Self> {
        let url = Url::parse(url_str).ok()?;
        Some(Self {
            scheme: Some(url.scheme().to_string()),
            host: url.host_str()?.to_string(),
            port: url.port(),
            path_prefix: url.path().to_string(),
        })
    }

    /// Check if a request matches this pattern.
    fn matches(&self, scheme: &str, host: &str, port: Option<u16>, path: &str) -> bool {
        // Scheme must match if specified
        if let Some(ref s) = self.scheme {
            if s != scheme {
                return false;
            }
        }

        // Host must match exactly
        if self.host != host {
            return false;
        }

        // Port must match if specified in pattern
        if let Some(pattern_port) = self.port {
            if port != Some(pattern_port) {
                return false;
            }
        }

        // Path must start with the pattern's path prefix
        path.starts_with(&self.path_prefix)
    }
}

impl MeshRegistry {
    /// Build a new MeshRegistry from configuration.
    pub fn from_config(config: &Config) -> Self {
        let mut url_index = Vec::new();

        // Build URL index from all ingress definitions (mesh membership optional)
        for (ingress_name, ingress) in &config.ingress {
            if !ingress.enabled {
                continue;
            }

            // Find which mesh this ingress belongs to (optional)
            let mesh_name = config
                .mesh
                .iter()
                .find(|(_, m)| m.enabled && m.ingress.contains(ingress_name))
                .map(|(name, _)| name.clone());

            // Parse and index each URL
            for url_str in &ingress.urls {
                match UrlPattern::from_url_str(url_str) {
                    Some(pattern) => {
                        tracing::debug!(
                            "Indexed ingress URL '{}' -> ingress '{}' (mesh {:?})",
                            url_str,
                            ingress_name,
                            mesh_name
                        );
                        url_index.push((pattern, ingress_name.clone(), mesh_name.clone()));
                    }
                    None => {
                        tracing::warn!(
                            "Failed to parse ingress URL '{}' for ingress '{}'",
                            url_str,
                            ingress_name
                        );
                    }
                }
            }
        }

        if !url_index.is_empty() {
            tracing::info!(
                "MeshRegistry initialized with {} URL patterns",
                url_index.len()
            );
        }

        Self {
            url_index,
            ingress: config.ingress.clone(),
            mesh: config.mesh.clone(),
        }
    }

    /// Check if a request URL matches any ingress.
    ///
    /// Returns the routing information if matched, None otherwise.
    /// Ingress can work without mesh membership for simple URL→pipeline binding.
    pub fn resolve(&self, scheme: &str, host: &str, port: Option<u16>, path: &str) -> Option<MeshRouteMatch> {
        self.resolve_with_config(scheme, host, port, path, None)
    }

    /// Check if a request URL matches any ingress, using Config for endpoint fallback.
    pub fn resolve_with_config(
        &self,
        scheme: &str,
        host: &str,
        port: Option<u16>,
        path: &str,
        config: Option<&Config>,
    ) -> Option<MeshRouteMatch> {
        for (pattern, ingress_name, mesh_name_opt) in &self.url_index {
            if pattern.matches(scheme, host, port, path) {
                let ingress = self.ingress.get(ingress_name)?;

                // Pipeline comes directly from ingress.pipeline
                let pipeline_name = &ingress.pipeline;

                // Resolve effective endpoint (override or first in pipeline)
                let endpoint_name = if let Some(ref ep) = ingress.endpoint {
                    ep.clone()
                } else {
                    // Fallback to first endpoint in pipeline
                    config
                        .and_then(|c| c.pipelines.get(pipeline_name))
                        .and_then(|p| p.endpoints.first())
                        .cloned()?
                };

                // Get mesh context if ingress belongs to a mesh
                let mesh = mesh_name_opt
                    .as_ref()
                    .and_then(|mn| self.mesh.get(mn));

                tracing::debug!(
                    "Ingress route match: {}://{}{}  -> ingress '{}' -> endpoint '{}' (pipeline '{}')",
                    scheme,
                    host,
                    path,
                    ingress_name,
                    endpoint_name,
                    pipeline_name
                );

                return Some(MeshRouteMatch {
                    endpoint_name,
                    pipeline_name: pipeline_name.clone(),
                    context: MeshContext {
                        mesh_name: mesh_name_opt.clone().unwrap_or_default(),
                        mesh: mesh.cloned().unwrap_or_default(),
                        ingress_name: ingress_name.clone(),
                        ingress: ingress.clone(),
                        pipeline_name: pipeline_name.clone(),
                    },
                });
            }
        }
        None
    }

    /// Check if the registry has any URL patterns.
    pub fn is_empty(&self) -> bool {
        self.url_index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::mesh::config::{MeshProtocol, MeshProvider};

    fn make_test_config() -> Config {
        let mut config = Config::default();

        // Add an endpoint
        config.endpoints.insert(
            "fhir_endpoint".to_string(),
            crate::models::endpoints::endpoint::Endpoint {
                service: "http".to_string(),
                options: None,
                peer_ref: None,
                connection: None,
                authentication: None,
            },
        );

        // Add a pipeline referencing the endpoint
        config.pipelines.insert(
            "fhir_pipeline".to_string(),
            crate::models::pipelines::config::Pipeline {
                endpoints: vec!["fhir_endpoint".to_string()],
                ..Default::default()
            },
        );

        // Add an ingress
        config.ingress.insert(
            "fhir_ingress".to_string(),
            MeshIngress {
                ingress_type: MeshProtocol::Http,
                pipeline: "fhir_pipeline".to_string(),
                endpoint: Some("fhir_endpoint".to_string()),
                urls: vec![
                    "https://fhir.example.com/r4".to_string(),
                    "https://fhir-backup.example.com/r4".to_string(),
                ],
                enabled: true,
                ..Default::default()
            },
        );

        // Add a mesh referencing the ingress
        config.mesh.insert(
            "healthcare".to_string(),
            Mesh {
                mesh_type: MeshProtocol::Http,
                provider: MeshProvider::Local,
                ingress: vec!["fhir_ingress".to_string()],
                egress: vec!["placeholder".to_string()],
                enabled: true,
                ..Default::default()
            },
        );

        config
    }

    #[test]
    fn test_registry_build() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        assert!(!registry.is_empty());
        assert_eq!(registry.url_index.len(), 2);
    }

    #[test]
    fn test_exact_url_match() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        let result = registry.resolve("https", "fhir.example.com", None, "/r4");
        assert!(result.is_some());

        let route = result.unwrap();
        assert_eq!(route.endpoint_name, "fhir_endpoint");
        assert_eq!(route.pipeline_name, "fhir_pipeline");
        assert_eq!(route.context.mesh_name, "healthcare");
        assert_eq!(route.context.ingress_name, "fhir_ingress");
    }

    #[test]
    fn test_path_prefix_match() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        // Should match paths that start with /r4
        let result = registry.resolve("https", "fhir.example.com", None, "/r4/Patient/123");
        assert!(result.is_some());
    }

    #[test]
    fn test_no_match_wrong_host() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        let result = registry.resolve("https", "wrong.example.com", None, "/r4");
        assert!(result.is_none());
    }

    #[test]
    fn test_no_match_wrong_scheme() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        let result = registry.resolve("http", "fhir.example.com", None, "/r4");
        assert!(result.is_none());
    }

    #[test]
    fn test_no_match_wrong_path() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        let result = registry.resolve("https", "fhir.example.com", None, "/api/other");
        assert!(result.is_none());
    }

    #[test]
    fn test_backup_url_match() {
        let config = make_test_config();
        let registry = MeshRegistry::from_config(&config);

        let result = registry.resolve("https", "fhir-backup.example.com", None, "/r4/Patient");
        assert!(result.is_some());
        assert_eq!(result.unwrap().endpoint_name, "fhir_endpoint");
    }

    #[test]
    fn test_disabled_ingress_not_indexed() {
        let mut config = make_test_config();
        config.ingress.get_mut("fhir_ingress").unwrap().enabled = false;

        let registry = MeshRegistry::from_config(&config);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_disabled_mesh_allows_ingress_indexing() {
        // Ingresses now work independently of meshes.
        // A disabled mesh shouldn't prevent the ingress from being indexed.
        let mut config = make_test_config();
        config.mesh.get_mut("healthcare").unwrap().enabled = false;

        let registry = MeshRegistry::from_config(&config);
        // Ingress should still be indexed even without an active mesh
        assert!(!registry.is_empty());

        // The route should match, but mesh context will be empty
        let result = registry.resolve("https", "fhir.example.com", None, "/r4");
        assert!(result.is_some());
        let route = result.unwrap();
        assert_eq!(route.context.mesh_name, ""); // No mesh since it's disabled
    }
}

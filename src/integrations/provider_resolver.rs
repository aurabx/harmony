//! Provider resolution service for resolving remote resource references.
//!
//! This service handles runtime resolution of provider-based resource references
//! by calling the appropriate provider API endpoints.

use crate::config::provider_config::ProviderConfig;
use crate::config::resource_reference::{LookupBy, ParsedReference};
use runbeam_sdk::runbeam_api::types::ResolvedResource;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Error type for provider resolution
#[derive(Debug, thiserror::Error)]
pub enum ResolutionError {
    #[error("Provider '{0}' not found")]
    ProviderNotFound(String),

    #[error("Provider '{0}' has no API URL configured")]
    NoApiUrl(String),

    #[error("Invalid reference: {0}")]
    InvalidReference(String),

    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),
}

/// Service for resolving remote resource references via provider APIs.
pub struct ProviderResolver {
    /// HTTP client for making API requests
    client: reqwest::Client,
    /// Provider configurations (keyed by provider name)
    providers: Arc<RwLock<HashMap<String, ProviderConfig>>>,
    /// Machine token for authenticating with providers
    token: Arc<RwLock<Option<String>>>,
}

impl ProviderResolver {
    /// Create a new provider resolver
    pub fn new(providers: HashMap<String, ProviderConfig>) -> Self {
        Self {
            client: reqwest::Client::new(),
            providers: Arc::new(RwLock::new(providers)),
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the authentication token for provider API calls
    pub async fn set_token(&self, token: String) {
        let mut token_guard = self.token.write().await;
        *token_guard = Some(token);
    }

    /// Update the provider configurations
    pub async fn update_providers(&self, providers: HashMap<String, ProviderConfig>) {
        let mut providers_guard = self.providers.write().await;
        *providers_guard = providers;
    }

    /// Resolve a resource reference string
    pub async fn resolve(&self, reference: &str) -> Result<ResolvedResource, ResolutionError> {
        let parsed = ParsedReference::parse(reference)
            .map_err(|e| ResolutionError::InvalidReference(e))?;

        self.resolve_parsed(&parsed).await
    }

    /// Resolve a parsed reference
    pub async fn resolve_parsed(
        &self,
        reference: &ParsedReference,
    ) -> Result<ResolvedResource, ResolutionError> {
        // Local references should be resolved from config, not via API
        if reference.is_local() {
            return Err(ResolutionError::InvalidReference(
                "Local references should be resolved from config".to_string(),
            ));
        }

        // Get the provider configuration
        let providers = self.providers.read().await;
        let provider = providers
            .get(&reference.provider)
            .ok_or_else(|| ResolutionError::ProviderNotFound(reference.provider.clone()))?;

        let api_url = provider
            .api
            .as_ref()
            .ok_or_else(|| ResolutionError::NoApiUrl(reference.provider.clone()))?;

        // Build the reference string for the API call
        let ref_string = self.build_reference_string(reference);

        // Call the provider API
        self.call_provider_api(api_url, &ref_string).await
    }

    /// Build the reference string for the API call
    fn build_reference_string(&self, reference: &ParsedReference) -> String {
        let mut parts = vec![reference.provider.clone()];

        if let Some(ref team) = reference.team {
            parts.push(team.clone());
        }

        if let Some(ref resource_type) = reference.resource_type {
            parts.push(resource_type.clone());
        }

        match &reference.lookup {
            LookupBy::Id(id) => {
                parts.push("id".to_string());
                parts.push(id.clone());
            }
            LookupBy::Name(name) => {
                parts.push("name".to_string());
                parts.push(name.clone());
            }
        }

        parts.join(".")
    }

    /// Call the provider API to resolve a reference
    async fn call_provider_api(
        &self,
        api_url: &str,
        reference: &str,
    ) -> Result<ResolvedResource, ResolutionError> {
        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| ResolutionError::ApiError("No authentication token set".to_string()))?;

        let url = format!(
            "{}/api/harmony/resources/resolve?ref={}",
            api_url.trim_end_matches('/'),
            urlencoding::encode(reference)
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ResolutionError::ApiError(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        // Parse the response
        let response_body: serde_json::Value = response.json().await?;

        // Extract the data field
        let data = response_body
            .get("data")
            .ok_or_else(|| ResolutionError::ApiError("Missing 'data' field in response".to_string()))?;

        // Parse into ResolvedResource
        let resolved: ResolvedResource = serde_json::from_value(data.clone())
            .map_err(|e| ResolutionError::ApiError(format!("Failed to parse response: {}", e)))?;

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_reference_string_full_path() {
        let resolver = ProviderResolver::new(HashMap::new());

        let reference = ParsedReference {
            provider: "runbeam".to_string(),
            team: Some("acme".to_string()),
            resource_type: Some("ingress".to_string()),
            lookup: LookupBy::Name("patient_api".to_string()),
        };

        let ref_string = resolver.build_reference_string(&reference);
        assert_eq!(ref_string, "runbeam.acme.ingress.name.patient_api");
    }

    #[test]
    fn test_build_reference_string_provider_id() {
        let resolver = ProviderResolver::new(HashMap::new());

        let reference = ParsedReference {
            provider: "runbeam".to_string(),
            team: None,
            resource_type: None,
            lookup: LookupBy::Id("01JGXYZ123ABC".to_string()),
        };

        let ref_string = resolver.build_reference_string(&reference);
        assert_eq!(ref_string, "runbeam.id.01JGXYZ123ABC");
    }

    #[tokio::test]
    async fn test_resolve_local_fails() {
        let resolver = ProviderResolver::new(HashMap::new());

        let result = resolver.resolve("my_ingress").await;
        assert!(matches!(result, Err(ResolutionError::InvalidReference(_))));
    }

    #[tokio::test]
    async fn test_resolve_unknown_provider() {
        let resolver = ProviderResolver::new(HashMap::new());

        let result = resolver.resolve("unknown.id.01JGXYZ123ABC").await;
        assert!(matches!(result, Err(ResolutionError::ProviderNotFound(_))));
    }

    #[tokio::test]
    async fn test_resolve_no_api_url() {
        let mut providers = HashMap::new();
        providers.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: None,
                poll_interval_secs: 30,
            },
        );

        let resolver = ProviderResolver::new(providers);

        let result = resolver.resolve("runbeam.id.01JGXYZ123ABC").await;
        assert!(matches!(result, Err(ResolutionError::NoApiUrl(_))));
    }
}

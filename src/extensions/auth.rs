//! Pluggable authentication provider trait.
//!
//! This module defines the `AuthProvider` trait which allows commercial
//! editions to implement custom authentication mechanisms like:
//! - SAML 2.0
//! - OIDC/OAuth2
//! - LDAP/Active Directory
//! - Custom API key validation

use crate::models::middleware::middleware::Middleware;
use serde_json::Value;
use std::collections::HashMap;

/// Trait for pluggable authentication providers.
///
/// Implement this trait to add custom authentication mechanisms that can
/// be configured via the Harmony configuration file and used as middleware.
///
/// # Configuration
///
/// Auth providers are referenced in middleware configuration:
///
/// ```toml
/// [middleware.my_saml_auth]
/// type = "saml"  # Matches AuthProvider::name()
/// options = { idp_url = "https://idp.example.com", entity_id = "harmony" }
/// ```
///
/// # Example Implementation
///
/// ```rust,ignore
/// use harmony::extensions::AuthProvider;
/// use harmony::models::middleware::Middleware;
///
/// struct SamlProvider;
///
/// impl AuthProvider for SamlProvider {
///     fn name(&self) -> &str {
///         "saml"
///     }
///
///     fn validate_config(&self, options: &HashMap<String, Value>) -> Result<(), String> {
///         // Validate required SAML configuration
///         if !options.contains_key("idp_url") {
///             return Err("saml provider requires 'idp_url' option".to_string());
///         }
///         Ok(())
///     }
///
///     fn create_middleware(&self, options: &HashMap<String, Value>) -> Result<Box<dyn Middleware>, String> {
///         let config = SamlConfig::from_options(options)?;
///         Ok(Box::new(SamlMiddleware::new(config)))
///     }
/// }
/// ```
pub trait AuthProvider: Send + Sync {
    /// Returns the unique name of this auth provider.
    ///
    /// This name is used in middleware configuration to reference the provider:
    /// ```toml
    /// [middleware.my_auth]
    /// type = "provider_name"  # This value must match name()
    /// ```
    fn name(&self) -> &str;

    /// Validates the provider configuration options.
    ///
    /// Called during configuration validation to ensure all required
    /// options are present and valid before startup.
    ///
    /// # Arguments
    /// * `options` - Key-value options from middleware configuration
    ///
    /// # Returns
    /// * `Ok(())` - Configuration is valid
    /// * `Err(message)` - Configuration is invalid with explanation
    fn validate_config(&self, options: &HashMap<String, Value>) -> Result<(), String>;

    /// Creates a middleware instance from the provider configuration.
    ///
    /// Called when building the middleware pipeline to instantiate
    /// the actual authentication middleware.
    ///
    /// # Arguments
    /// * `options` - Key-value options from middleware configuration
    ///
    /// # Returns
    /// * `Ok(middleware)` - Successfully created middleware instance
    /// * `Err(message)` - Failed to create middleware with explanation
    fn create_middleware(
        &self,
        options: &HashMap<String, Value>,
    ) -> Result<Box<dyn Middleware>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::ConfigError;
    use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
    use crate::utils::Error;
    use async_trait::async_trait;

    // Test implementation of AuthProvider
    struct TestAuthProvider;

    impl AuthProvider for TestAuthProvider {
        fn name(&self) -> &str {
            "test_auth"
        }

        fn validate_config(&self, options: &HashMap<String, Value>) -> Result<(), String> {
            if !options.contains_key("api_key") {
                return Err("test_auth requires 'api_key' option".to_string());
            }
            Ok(())
        }

        fn create_middleware(
            &self,
            _options: &HashMap<String, Value>,
        ) -> Result<Box<dyn Middleware>, String> {
            Ok(Box::new(TestAuthMiddleware))
        }
    }

    // Test middleware implementation
    struct TestAuthMiddleware;

    #[async_trait]
    impl Middleware for TestAuthMiddleware {
        fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
            Ok(())
        }

        async fn left(
            &self,
            envelope: RequestEnvelope<serde_json::Value>,
        ) -> Result<RequestEnvelope<serde_json::Value>, Error> {
            Ok(envelope)
        }

        async fn right(
            &self,
            envelope: ResponseEnvelope<serde_json::Value>,
        ) -> Result<ResponseEnvelope<serde_json::Value>, Error> {
            Ok(envelope)
        }
    }

    #[test]
    fn test_auth_provider_validation() {
        let provider = TestAuthProvider;

        // Should fail without api_key
        let empty_opts = HashMap::new();
        assert!(provider.validate_config(&empty_opts).is_err());

        // Should succeed with api_key
        let mut valid_opts = HashMap::new();
        valid_opts.insert("api_key".to_string(), Value::String("test".to_string()));
        assert!(provider.validate_config(&valid_opts).is_ok());
    }

    #[test]
    fn test_auth_provider_create_middleware() {
        let provider = TestAuthProvider;
        let opts = HashMap::new();

        let middleware = provider.create_middleware(&opts);
        assert!(middleware.is_ok());
    }

    #[test]
    fn test_auth_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn AuthProvider>();
    }
}

//! Pluggable middleware factory trait.
//!
//! This module defines the `MiddlewareFactory` trait which allows commercial
//! editions to implement custom middleware types like:
//! - Advanced rate limiting
//! - Request/response caching
//! - Circuit breakers
//! - Data masking/anonymization

use crate::config::config::Config;
use crate::models::middleware::middleware::Middleware;
use serde_json::Value;
use std::collections::HashMap;

/// Factory trait for creating custom middleware types.
///
/// Implement this trait to add new middleware types that can be
/// referenced in pipeline configuration.
///
/// # Configuration
///
/// Middleware factories are referenced by type name:
///
/// ```toml
/// [middleware.my_rate_limiter]
/// type = "rate_limit"  # Matches MiddlewareFactory::type_name()
/// options = { requests_per_second = 100, burst = 50 }
/// ```
///
/// # Example Implementation
///
/// ```rust,ignore
/// use harmony::extensions::MiddlewareFactory;
/// use harmony::models::middleware::Middleware;
///
/// struct RateLimitFactory;
///
/// impl MiddlewareFactory for RateLimitFactory {
///     fn type_name(&self) -> &str {
///         "rate_limit"
///     }
///
///     fn create(
///         &self,
///         options: &HashMap<String, Value>,
///         _config: Option<&Config>,
///     ) -> Result<Box<dyn Middleware>, String> {
///         let config = RateLimitConfig::from_options(options)?;
///         Ok(Box::new(RateLimitMiddleware::new(config)))
///     }
/// }
/// ```
pub trait MiddlewareFactory: Send + Sync {
    /// Returns the primary type name for this middleware factory.
    ///
    /// This name is used in middleware configuration to reference the factory:
    /// ```toml
    /// [middleware.my_middleware]
    /// type = "factory_type_name"  # This value must match type_name()
    /// ```
    fn type_name(&self) -> &str;

    /// Returns alternative names for this middleware type.
    ///
    /// Aliases allow backward compatibility when renaming middleware types
    /// or providing shorthand names.
    ///
    /// Default implementation returns an empty slice (no aliases).
    fn aliases(&self) -> &[&str] {
        &[]
    }

    /// Creates a middleware instance from configuration options.
    ///
    /// # Arguments
    /// * `options` - Key-value options from middleware instance configuration
    /// * `config` - Optional reference to full application config (for advanced middleware)
    ///
    /// # Returns
    /// * `Ok(middleware)` - Successfully created middleware instance
    /// * `Err(message)` - Failed to create middleware with explanation
    fn create(
        &self,
        options: &HashMap<String, Value>,
        config: Option<&Config>,
    ) -> Result<Box<dyn Middleware>, String>;

    /// Validates the middleware configuration options.
    ///
    /// Called during configuration validation. Default implementation
    /// delegates to create() for validation.
    ///
    /// # Arguments
    /// * `options` - Key-value options from middleware instance configuration
    ///
    /// # Returns
    /// * `Ok(())` - Configuration is valid
    /// * `Err(message)` - Configuration is invalid with explanation
    fn validate(&self, options: &HashMap<String, Value>) -> Result<(), String> {
        // Default validation: try to create instance
        self.create(options, None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::ConfigError;
    use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
    use crate::utils::Error;
    use async_trait::async_trait;

    // Test implementation of MiddlewareFactory
    struct TestMiddlewareFactory;

    impl MiddlewareFactory for TestMiddlewareFactory {
        fn type_name(&self) -> &str {
            "test_middleware"
        }

        fn create(
            &self,
            options: &HashMap<String, Value>,
            _config: Option<&Config>,
        ) -> Result<Box<dyn Middleware>, String> {
            // Validate required option
            if let Some(threshold) = options.get("threshold") {
                if !threshold.is_number() {
                    return Err("threshold must be a number".to_string());
                }
            }
            Ok(Box::new(TestMiddleware))
        }
    }

    // Test middleware implementation
    struct TestMiddleware;

    #[async_trait]
    impl Middleware for TestMiddleware {
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
    fn test_middleware_factory_type_name() {
        let factory = TestMiddlewareFactory;
        assert_eq!(factory.type_name(), "test_middleware");
    }

    #[test]
    fn test_middleware_factory_create() {
        let factory = TestMiddlewareFactory;
        let opts = HashMap::new();

        let result = factory.create(&opts, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_middleware_factory_validation() {
        let factory = TestMiddlewareFactory;

        // Valid config
        let mut valid_opts = HashMap::new();
        valid_opts.insert("threshold".to_string(), Value::Number(100.into()));
        assert!(factory.validate(&valid_opts).is_ok());

        // Invalid config
        let mut invalid_opts = HashMap::new();
        invalid_opts.insert("threshold".to_string(), Value::String("not_a_number".to_string()));
        assert!(factory.validate(&invalid_opts).is_err());
    }

    #[test]
    fn test_middleware_factory_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn MiddlewareFactory>();
    }
}

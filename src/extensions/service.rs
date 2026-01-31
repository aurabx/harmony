//! Pluggable service factory trait.
//!
//! This module defines the `ServiceFactory` trait which allows commercial
//! editions to implement custom service types like:
//! - Cloud storage backends (S3, GCS, Azure Blob)
//! - Message queue adapters (RabbitMQ, Kafka)
//! - Database backends
//! - Proprietary PACS integrations

use crate::models::services::services::ServiceType;
use serde_json::Value;

/// Factory trait for creating custom service types.
///
/// Implement this trait to add new service types that can be
/// used as endpoints or backends in pipeline configuration.
///
/// # Configuration
///
/// Service factories are referenced by service name:
///
/// ```toml
/// [endpoints.my_s3_endpoint]
/// service = "s3_storage"  # Matches ServiceFactory::service_name()
/// options = { bucket = "my-bucket", region = "us-east-1" }
///
/// [backends.my_s3_backend]
/// service = "s3_storage"
/// options = { bucket = "archive-bucket" }
/// ```
///
/// # Example Implementation
///
/// ```rust,ignore
/// use harmony::extensions::ServiceFactory;
/// use harmony::models::services::ServiceType;
///
/// struct S3StorageFactory;
///
/// impl ServiceFactory for S3StorageFactory {
///     fn service_name(&self) -> &str {
///         "s3_storage"
///     }
///
///     fn create(&self) -> Box<dyn ServiceType<ReqBody = serde_json::Value>> {
///         Box::new(S3StorageService::new())
///     }
/// }
/// ```
pub trait ServiceFactory: Send + Sync {
    /// Returns the primary service name for this factory.
    ///
    /// This name is used in endpoint/backend configuration:
    /// ```toml
    /// [endpoints.my_endpoint]
    /// service = "service_name"  # This value must match service_name()
    /// ```
    fn service_name(&self) -> &str;

    /// Returns alternative names for this service type.
    ///
    /// Aliases allow backward compatibility when renaming service types
    /// or providing shorthand names.
    ///
    /// Default implementation returns an empty slice (no aliases).
    fn aliases(&self) -> &[&str] {
        &[]
    }

    /// Creates a new service instance.
    ///
    /// The returned service will be used for handling requests
    /// as either an endpoint or backend.
    ///
    /// # Returns
    /// A boxed service type instance
    fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>>;

    /// Optional description for documentation/logging
    fn description(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::ConfigError;
    use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
    use crate::models::services::services::ServiceHandler;
    use crate::router::route_config::RouteConfig;
    use crate::utils::Error;
    use async_trait::async_trait;
    use axum::response::{IntoResponse, Response};
    use std::collections::HashMap;

    // Test implementation of ServiceFactory
    struct TestServiceFactory;

    impl ServiceFactory for TestServiceFactory {
        fn service_name(&self) -> &str {
            "test_service"
        }

        fn create(&self) -> Box<dyn ServiceType<ReqBody = Value>> {
            Box::new(TestService)
        }

        fn description(&self) -> Option<&str> {
            Some("A test service for unit testing")
        }
    }

    // Test service implementation
    struct TestService;

    #[async_trait]
    impl ServiceType for TestService {
        fn validate(&self, _options: &HashMap<String, Value>) -> Result<(), ConfigError> {
            Ok(())
        }

        fn build_router(&self, _options: &HashMap<String, Value>) -> Vec<RouteConfig> {
            vec![]
        }
    }

    #[async_trait]
    impl ServiceHandler<Value> for TestService {
        type ReqBody = Value;

        async fn endpoint_incoming_request(
            &self,
            envelope: RequestEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<RequestEnvelope<Vec<u8>>, Error> {
            Ok(envelope)
        }

        async fn endpoint_outgoing_response(
            &self,
            _envelope: ResponseEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<Response, Error> {
            Ok("OK".into_response())
        }

        async fn backend_outgoing_request(
            &self,
            envelope: RequestEnvelope<Vec<u8>>,
            _options: &HashMap<String, Value>,
        ) -> Result<ResponseEnvelope<Vec<u8>>, Error> {
            Ok(ResponseEnvelope::from_backend(
                envelope.request_details,
                200,
                HashMap::new(),
                vec![],
                None,
            ))
        }
    }

    #[test]
    fn test_service_factory_name() {
        let factory = TestServiceFactory;
        assert_eq!(factory.service_name(), "test_service");
    }

    #[test]
    fn test_service_factory_create() {
        let factory = TestServiceFactory;
        let service = factory.create();

        // Verify the service can be used
        let opts = HashMap::new();
        let routes = service.build_router(&opts);
        assert!(routes.is_empty()); // TestService returns empty routes
    }

    #[test]
    fn test_service_factory_description() {
        let factory = TestServiceFactory;
        assert_eq!(
            factory.description(),
            Some("A test service for unit testing")
        );
    }

    #[test]
    fn test_service_factory_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ServiceFactory>();
    }
}

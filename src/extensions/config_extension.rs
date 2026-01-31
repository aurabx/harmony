//! Config extension trait for custom configuration validation.
//!
//! This module defines the `ConfigExtension` trait which allows commercial
//! editions to declare and validate their custom configuration sections.

use serde_json::Value;

/// Trait for validating custom extension configuration.
///
/// Implement this trait to declare configuration requirements for your
/// extension and validate the configuration at startup.
///
/// # Configuration
///
/// Extensions are configured under the `[extensions]` section:
///
/// ```toml
/// [extensions.my_extension]
/// api_key = "secret"
/// max_connections = 100
/// ```
///
/// # Example Implementation
///
/// ```rust,ignore
/// use harmony::extensions::ConfigExtension;
/// use serde_json::Value;
///
/// struct LicenseExtension;
///
/// impl ConfigExtension for LicenseExtension {
///     fn name(&self) -> &str {
///         "license"
///     }
///
///     fn is_required(&self) -> bool {
///         true  // Commercial edition requires license config
///     }
///
///     fn validate(&self, config: &Value) -> Result<(), String> {
///         let license_key = config.get("key")
///             .and_then(|v| v.as_str())
///             .ok_or("license.key is required")?;
///         
///         if license_key.len() < 32 {
///             return Err("license.key must be at least 32 characters".to_string());
///         }
///         
///         Ok(())
///     }
/// }
/// ```
pub trait ConfigExtension: Send + Sync {
    /// Returns the name of this extension.
    ///
    /// This name is used to look up configuration in the `[extensions]` section:
    /// ```toml
    /// [extensions.extension_name]  # This key must match name()
    /// ```
    fn name(&self) -> &str;

    /// Returns whether this extension's configuration is required.
    ///
    /// If true, startup will fail if the extension is not configured.
    /// If false, the extension is optional and validation is skipped if not configured.
    fn is_required(&self) -> bool {
        false
    }

    /// Validates the extension configuration.
    ///
    /// Called during startup after config loading but before adapters start.
    /// This allows extensions to validate their configuration and fail fast
    /// if the configuration is invalid.
    ///
    /// # Arguments
    /// * `config` - The JSON value from `[extensions.<name>]`
    ///
    /// # Returns
    /// * `Ok(())` - Configuration is valid
    /// * `Err(message)` - Configuration is invalid with explanation
    fn validate(&self, config: &Value) -> Result<(), String>;

    /// Returns a description of the expected configuration schema.
    ///
    /// Used for documentation and error messages.
    fn schema_description(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestExtension {
        required: bool,
    }

    impl ConfigExtension for TestExtension {
        fn name(&self) -> &str {
            "test"
        }

        fn is_required(&self) -> bool {
            self.required
        }

        fn validate(&self, config: &Value) -> Result<(), String> {
            if config.get("api_key").is_none() {
                return Err("api_key is required".to_string());
            }
            Ok(())
        }

        fn schema_description(&self) -> Option<&str> {
            Some("api_key: string (required)")
        }
    }

    #[test]
    fn test_config_extension_validation() {
        let ext = TestExtension { required: true };

        // Should fail without api_key
        let empty = serde_json::json!({});
        assert!(ext.validate(&empty).is_err());

        // Should succeed with api_key
        let valid = serde_json::json!({"api_key": "test"});
        assert!(ext.validate(&valid).is_ok());
    }

    #[test]
    fn test_config_extension_required() {
        let required_ext = TestExtension { required: true };
        let optional_ext = TestExtension { required: false };

        assert!(required_ext.is_required());
        assert!(!optional_ext.is_required());
    }

    #[test]
    fn test_config_extension_schema() {
        let ext = TestExtension { required: false };
        assert_eq!(ext.schema_description(), Some("api_key: string (required)"));
    }

    #[test]
    fn test_config_extension_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ConfigExtension>();
    }
}

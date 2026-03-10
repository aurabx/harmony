//! Plugin trait for lifecycle hooks.
//!
//! Plugins provide extension points for custom behavior during Harmony's
//! lifecycle. This enables commercial editions to add functionality like:
//! - License validation
//! - Telemetry/metrics collection
//! - Custom initialization logic
//! - Graceful shutdown handlers

use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;

/// Lifecycle hooks for Harmony extensions.
///
/// Implement this trait to add custom behavior at various points in
/// Harmony's lifecycle. All methods have default no-op implementations,
/// so you only need to implement the hooks you care about.
///
/// # Example
///
/// ```rust,ignore
/// use harmony::extensions::HarmonyPlugin;
/// use harmony::Config;
///
/// struct LicensePlugin {
///     license_key: String,
/// }
///
/// impl HarmonyPlugin for LicensePlugin {
///     fn name(&self) -> &str {
///         "license-validator"
///     }
///
///     fn on_pre_start(&self, config: &Config) -> Result<(), String> {
///         // Validate license before starting
///         if !self.validate_license(&self.license_key, config) {
///             return Err("Invalid or expired license".to_string());
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait HarmonyPlugin: Send + Sync {
    /// Returns the unique name of this plugin.
    ///
    /// Used for logging and debugging purposes.
    fn name(&self) -> &str;

    /// Called after configuration is loaded but before validation.
    ///
    /// Use this hook to modify configuration values, inject defaults,
    /// or perform early validation.
    ///
    /// # Arguments
    /// * `config` - Mutable reference to the loaded configuration
    ///
    /// # Returns
    /// * `Ok(())` - Continue startup
    /// * `Err(message)` - Abort startup with error message
    fn on_config_load(&self, _config: &mut Config) -> Result<(), String> {
        Ok(())
    }

    /// Called after configuration validation but before adapters start.
    ///
    /// Use this hook for pre-flight checks like:
    /// - License validation
    /// - External service connectivity checks
    /// - Resource availability verification
    ///
    /// # Arguments
    /// * `config` - Reference to the validated configuration
    ///
    /// # Returns
    /// * `Ok(())` - Continue startup
    /// * `Err(message)` - Abort startup with error message
    fn on_pre_start(&self, _config: &Config) -> Result<(), String> {
        Ok(())
    }

    /// Called after all adapters have started successfully.
    ///
    /// Use this hook for post-startup tasks like:
    /// - Registering with external services
    /// - Starting background tasks
    /// - Emitting startup metrics
    ///
    /// # Arguments
    /// * `config` - Reference to the configuration
    /// * `registry` - Reference to the adapter registry
    ///
    /// # Returns
    /// * `Ok(())` - Continue running
    /// * `Err(message)` - Log error but continue (non-fatal)
    fn on_post_start(&self, _config: &Config, _registry: &AdapterRegistry) -> Result<(), String> {
        Ok(())
    }

    /// Called during graceful shutdown before adapters are stopped.
    ///
    /// Use this hook for cleanup tasks like:
    /// - Deregistering from external services
    /// - Flushing metrics/logs
    /// - Releasing external resources
    ///
    /// # Returns
    /// * `Ok(())` - Continue shutdown
    /// * `Err(message)` - Log error but continue shutdown
    fn on_shutdown(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        name: String,
        on_config_load_called: std::sync::atomic::AtomicBool,
        on_pre_start_called: std::sync::atomic::AtomicBool,
    }

    impl TestPlugin {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                on_config_load_called: std::sync::atomic::AtomicBool::new(false),
                on_pre_start_called: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl HarmonyPlugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn on_config_load(&self, _config: &mut Config) -> Result<(), String> {
            self.on_config_load_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn on_pre_start(&self, _config: &Config) -> Result<(), String> {
            self.on_pre_start_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_plugin_trait_default_implementations() {
        // Default implementations should return Ok(())
        struct MinimalPlugin;
        impl HarmonyPlugin for MinimalPlugin {
            fn name(&self) -> &str {
                "minimal"
            }
        }

        let plugin = MinimalPlugin;
        let mut config = Config::default();

        assert_eq!(plugin.name(), "minimal");
        assert!(plugin.on_config_load(&mut config).is_ok());
        assert!(plugin.on_pre_start(&config).is_ok());
        assert!(plugin.on_shutdown().is_ok());
    }

    #[test]
    fn test_plugin_with_custom_hooks() {
        let plugin = TestPlugin::new("test-plugin");
        let mut config = Config::default();

        assert_eq!(plugin.name(), "test-plugin");

        // Verify hooks are not called initially
        assert!(!plugin
            .on_config_load_called
            .load(std::sync::atomic::Ordering::SeqCst));
        assert!(!plugin
            .on_pre_start_called
            .load(std::sync::atomic::Ordering::SeqCst));

        // Call hooks and verify they were executed
        assert!(plugin.on_config_load(&mut config).is_ok());
        assert!(plugin
            .on_config_load_called
            .load(std::sync::atomic::Ordering::SeqCst));

        assert!(plugin.on_pre_start(&config).is_ok());
        assert!(plugin
            .on_pre_start_called
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_plugin_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn HarmonyPlugin>();
    }
}

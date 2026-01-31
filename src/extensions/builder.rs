//! Builder pattern for customized Harmony startup.
//!
//! The `HarmonyBuilder` provides a fluent API for configuring and starting
//! Harmony with custom extensions, plugins, and providers.

use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;
use crate::config::watcher::ConfigWatcher;
use crate::extensions::auth::AuthProvider;
use crate::extensions::config_extension::ConfigExtension;
use crate::extensions::middleware::MiddlewareFactory;
use crate::extensions::plugin::HarmonyPlugin;
use crate::extensions::registry::{get_registry, initialize_builtin_factories};
use crate::extensions::service::ServiceFactory;
use crate::integrations::provider_resolver::ProviderResolver;
use crate::storage::create_storage_backend;
use runbeam_sdk::{load_token, save_token};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::prelude::*;

/// Registry for custom extensions registered via HarmonyBuilder.
///
/// This is stored globally so middleware/service resolution can access it.
pub struct ExtensionRegistry {
    pub auth_providers: Vec<Box<dyn AuthProvider>>,
    pub middleware_factories: Vec<Box<dyn MiddlewareFactory>>,
    pub service_factories: Vec<Box<dyn ServiceFactory>>,
}

impl ExtensionRegistry {
    fn new() -> Self {
        Self {
            auth_providers: Vec::new(),
            middleware_factories: Vec::new(),
            service_factories: Vec::new(),
        }
    }

    /// Find an auth provider by name
    pub fn find_auth_provider(&self, name: &str) -> Option<&dyn AuthProvider> {
        self.auth_providers
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
    }

    /// Find a middleware factory by type name
    pub fn find_middleware_factory(&self, type_name: &str) -> Option<&dyn MiddlewareFactory> {
        self.middleware_factories
            .iter()
            .find(|f| f.type_name() == type_name)
            .map(|f| f.as_ref())
    }

    /// Find a service factory by name
    pub fn find_service_factory(&self, name: &str) -> Option<&dyn ServiceFactory> {
        self.service_factories
            .iter()
            .find(|f| f.service_name() == name)
            .map(|f| f.as_ref())
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global extension registry for use by middleware/service resolution.
static EXTENSION_REGISTRY: std::sync::OnceLock<Arc<RwLock<ExtensionRegistry>>> =
    std::sync::OnceLock::new();

/// Get the global extension registry
pub fn get_extension_registry() -> Arc<RwLock<ExtensionRegistry>> {
    EXTENSION_REGISTRY
        .get_or_init(|| Arc::new(RwLock::new(ExtensionRegistry::new())))
        .clone()
}

/// Builder for customized Harmony startup.
///
/// Provides a fluent API for registering custom extensions, plugins,
/// and providers before starting Harmony.
///
/// # Example
///
/// ```rust,ignore
/// use harmony::{Config, HarmonyBuilder};
///
/// #[tokio::main]
/// async fn main() {
///     let config = Config::load("config.toml").unwrap();
///     
///     HarmonyBuilder::new(config)
///         .with_auth_provider(Box::new(SamlProvider::new()))
///         .with_middleware_factory(Box::new(RateLimitFactory::new()))
///         .with_plugin(Box::new(LicensePlugin::new()))
///         .run()
///         .await;
/// }
/// ```
pub struct HarmonyBuilder {
    config: Config,
    config_path: Option<String>,
    plugins: Vec<Box<dyn HarmonyPlugin>>,
    auth_providers: Vec<Box<dyn AuthProvider>>,
    middleware_factories: Vec<Box<dyn MiddlewareFactory>>,
    service_factories: Vec<Box<dyn ServiceFactory>>,
    config_extensions: Vec<Box<dyn ConfigExtension>>,
}

impl HarmonyBuilder {
    /// Creates a new HarmonyBuilder with the given configuration.
    ///
    /// # Arguments
    /// * `config` - The loaded Harmony configuration
    pub fn new(config: Config) -> Self {
        Self {
            config,
            config_path: None,
            plugins: Vec::new(),
            auth_providers: Vec::new(),
            middleware_factories: Vec::new(),
            service_factories: Vec::new(),
            config_extensions: Vec::new(),
        }
    }

    /// Creates a new HarmonyBuilder by loading configuration from a file path.
    ///
    /// # Arguments
    /// * `config_path` - Path to the configuration file
    ///
    /// # Returns
    /// * `Ok(builder)` - Builder with loaded configuration
    /// * `Err(error)` - Configuration loading failed
    pub fn from_config_path(config_path: &str) -> Result<Self, crate::config::config::ConfigError> {
        let config = Config::load(config_path)?;
        Ok(Self {
            config,
            config_path: Some(config_path.to_string()),
            plugins: Vec::new(),
            auth_providers: Vec::new(),
            middleware_factories: Vec::new(),
            service_factories: Vec::new(),
            config_extensions: Vec::new(),
        })
    }

    /// Sets the configuration file path for hot-reload support.
    ///
    /// If set, Harmony will watch this file for changes and reload
    /// configuration automatically.
    pub fn with_config_path(mut self, path: impl Into<String>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Registers a plugin for lifecycle hooks.
    ///
    /// Plugins are called in registration order at each lifecycle point.
    pub fn with_plugin(mut self, plugin: Box<dyn HarmonyPlugin>) -> Self {
        tracing::debug!("Registering plugin: {}", plugin.name());
        self.plugins.push(plugin);
        self
    }

    /// Registers a custom authentication provider.
    ///
    /// Auth providers can be referenced in middleware configuration:
    /// ```toml
    /// [middleware.my_auth]
    /// type = "provider_name"  # Must match AuthProvider::name()
    /// ```
    pub fn with_auth_provider(mut self, provider: Box<dyn AuthProvider>) -> Self {
        tracing::debug!("Registering auth provider: {}", provider.name());
        self.auth_providers.push(provider);
        self
    }

    /// Registers a custom middleware factory.
    ///
    /// Middleware factories can be referenced in middleware configuration:
    /// ```toml
    /// [middleware.my_middleware]
    /// type = "factory_type"  # Must match MiddlewareFactory::type_name()
    /// ```
    pub fn with_middleware_factory(mut self, factory: Box<dyn MiddlewareFactory>) -> Self {
        tracing::debug!("Registering middleware factory: {}", factory.type_name());
        self.middleware_factories.push(factory);
        self
    }

    /// Registers a custom service factory.
    ///
    /// Service factories can be referenced in endpoint/backend configuration:
    /// ```toml
    /// [endpoints.my_endpoint]
    /// service = "service_name"  # Must match ServiceFactory::service_name()
    /// ```
    pub fn with_service_factory(mut self, factory: Box<dyn ServiceFactory>) -> Self {
        tracing::debug!("Registering service factory: {}", factory.service_name());
        self.service_factories.push(factory);
        self
    }

    /// Registers a config extension for validating custom configuration.
    ///
    /// Config extensions validate their section of the `[extensions]` table:
    /// ```toml
    /// [extensions.my_extension]
    /// api_key = "secret"
    /// ```
    pub fn with_config_extension(mut self, extension: Box<dyn ConfigExtension>) -> Self {
        tracing::debug!("Registering config extension: {}", extension.name());
        self.config_extensions.push(extension);
        self
    }

    /// Builds and runs Harmony with all registered extensions.
    ///
    /// This method:
    /// 1. Calls plugin `on_config_load` hooks
    /// 2. Validates configuration
    /// 3. Calls plugin `on_pre_start` hooks
    /// 4. Initializes storage and registries
    /// 5. Starts protocol adapters
    /// 6. Calls plugin `on_post_start` hooks
    /// 7. Waits for shutdown signal
    /// 8. Calls plugin `on_shutdown` hooks
    pub async fn run(mut self) {
        // Install rustls crypto provider
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        // Initialize unified registry with built-in factories
        initialize_builtin_factories().await;

        // Register custom extensions in the unified registry
        {
            let registry = get_registry();
            let mut reg = registry
                .write()
                .expect("Failed to acquire write lock on unified registry");

            // Register custom middleware factories
            for factory in self.middleware_factories.drain(..) {
                reg.register_middleware_factory(factory);
            }

            // Register custom service factories
            for factory in self.service_factories.drain(..) {
                reg.register_service_factory(factory);
            }

            // Register auth providers
            for provider in self.auth_providers.drain(..) {
                reg.register_auth_provider(provider);
            }

            // Register config extensions
            for extension in self.config_extensions.drain(..) {
                reg.register_config_extension(extension);
            }
        }

        // Also populate legacy extension registry for backward compatibility
        {
            let registry = get_extension_registry();
            let mut reg = registry.write().await;
            reg.auth_providers = Vec::new();
            reg.middleware_factories = Vec::new();
            reg.service_factories = Vec::new();
        }

        // Validate config extensions
        {
            let registry = get_registry();
            let validation_result = {
                let reg = registry
                    .read()
                    .expect("Failed to acquire read lock on unified registry");
                reg.validate_config_extensions(&self.config)
            };

            if let Err(errors) = validation_result {
                for error in &errors {
                    tracing::error!("Config extension validation failed: {}", error);
                }
                tracing::error!("Aborting startup due to config extension validation errors.");
                return;
            }
        }

        // Phase 1: Plugin on_config_load hooks
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_config_load(&mut self.config) {
                tracing::error!(
                    "Plugin '{}' failed in on_config_load: {}. Aborting startup.",
                    plugin.name(),
                    e
                );
                return;
            }
        }

        // Initialize provider resolver
        let resolver = Arc::new(ProviderResolver::new(self.config.provider.clone()));
        crate::globals::set_provider_resolver(resolver);

        // Initialize logging before creating Arc (needs config reference)
        Self::initialize_logging_static(&self.config);

        let config = Arc::new(self.config);
        crate::globals::set_config(config.clone());

        // Set config path for management API if provided
        if let Some(ref path) = self.config_path {
            crate::globals::set_config_path(path.clone());
        }

        // Initialize storage
        let storage =
            create_storage_backend(&config.storage).expect("Failed to create storage backend");
        crate::globals::set_storage(storage);

        tracing::info!("Starting Harmony '{}'", config.proxy.effective_id());

        // Phase 2: Plugin on_pre_start hooks
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_pre_start(&config) {
                tracing::error!(
                    "Plugin '{}' failed in on_pre_start: {}. Aborting startup.",
                    plugin.name(),
                    e
                );
                return;
            }
        }

        // Create adapter registry
        let registry = Arc::new(AdapterRegistry::new());
        crate::globals::set_adapter_registry(registry.clone());

        // Start protocol adapters for each network
        for network_name in config.network.keys() {
            if let Err(e) = registry
                .start_network(network_name.clone(), config.clone())
                .await
            {
                tracing::error!("Failed to start network '{}': {}", network_name, e);
            }
        }

        tracing::info!("All adapters started. Press Ctrl+C to shutdown.");

        // Phase 3: Plugin on_post_start hooks
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_post_start(&config, &registry) {
                // Non-fatal, just log
                tracing::warn!(
                    "Plugin '{}' failed in on_post_start: {}",
                    plugin.name(),
                    e
                );
            }
        }

        // Start config watcher if config path provided
        let watcher_shutdown = tokio_util::sync::CancellationToken::new();
        if let Some(path) = self.config_path.clone() {
            Self::start_config_watcher_static(
                &path,
                &config,
                registry.clone(),
                watcher_shutdown.clone(),
            );
        }

        // Start cloud integration if enabled
        Self::start_cloud_integration_static(&config, registry.clone()).await;

        // Wait for ctrl-c signal
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl-c signal");

        // Phase 4: Plugin on_shutdown hooks
        tracing::info!("Shutting down...");
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_shutdown() {
                tracing::warn!("Plugin '{}' failed in on_shutdown: {}", plugin.name(), e);
            }
        }

        // Stop config watcher
        watcher_shutdown.cancel();

        // Stop all adapters
        if let Err(e) = registry.stop_all().await {
            tracing::error!("Error stopping adapters: {}", e);
        }

        tracing::info!("Harmony shut down gracefully.");
    }

    fn initialize_logging_static(config: &Config) {
        if config.logging.log_to_file {
            let file_appender = tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true)
                .with_writer(
                    std::fs::File::create(&config.logging.log_file_path)
                        .expect("Failed to create log file"),
                );

            let stdout_appender = tracing_subscriber::fmt::layer()
                .with_file(true)
                .with_line_number(true);

            let _ = tracing_subscriber::registry()
                .with(file_appender)
                .with(stdout_appender)
                .try_init();
        } else {
            let _ = tracing_subscriber::fmt()
                .with_file(true)
                .with_line_number(true)
                .try_init();
        }
    }

    fn start_config_watcher_static(
        path: &str,
        config: &Config,
        registry: Arc<AdapterRegistry>,
        shutdown: tokio_util::sync::CancellationToken,
    ) {
        let config_dir = Path::new(path)
            .parent()
            .expect("Failed to get config file directory");
        let pipelines_dir = config_dir.join(&config.proxy.pipelines_path);
        let pipelines_path = if pipelines_dir.exists() {
            Some(pipelines_dir.to_string_lossy().to_string())
        } else {
            None
        };

        let watcher = ConfigWatcher::new(path.to_string(), pipelines_path, registry);
        tokio::spawn(async move {
            tokio::select! {
                result = watcher.start() => {
                    if let Err(e) = result {
                        tracing::error!("Config watcher error: {}", e);
                    }
                }
                _ = shutdown.cancelled() => {
                    tracing::info!("Config watcher stopped");
                }
            }
        });
    }

    async fn start_cloud_integration_static(config: &Arc<Config>, registry: Arc<AdapterRegistry>) {
        if !config.is_cloud_enabled() {
            tracing::info!(
                "Cloud integration is disabled (primary_provider: {}, not enabled or no API configured)",
                config.proxy.primary_provider
            );
            return;
        }

        let proxy_id = config.proxy.effective_id().to_string();

        // Check for machine token from environment variable first
        let token_from_env = std::env::var("RUNBEAM_MACHINE_TOKEN")
            .ok()
            .and_then(|token_str| {
                match serde_json::from_str::<runbeam_sdk::MachineToken>(&token_str) {
                    Ok(token) => {
                        tracing::info!(
                            "Using machine token from RUNBEAM_MACHINE_TOKEN environment variable"
                        );
                        Some(token)
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse RUNBEAM_MACHINE_TOKEN: {}. Falling back to stored token.",
                            e
                        );
                        None
                    }
                }
            });

        // Try environment variable first, then fall back to secure storage
        let token_result = if let Some(ref token) = token_from_env {
            if let Err(e) = save_token(&proxy_id, "auth", token).await {
                tracing::warn!("Failed to save env token to storage: {}", e);
            }
            Ok(Some(token.clone()))
        } else {
            load_token(&proxy_id, "auth").await
        };

        match token_result {
            Ok(Some(token)) if token.is_valid() => {
                let poll_interval = config
                    .primary_poll_interval()
                    .unwrap_or_else(|| std::time::Duration::from_secs(30));
                let base_url = config.primary_api_base_url();

                tracing::info!(
                    "Found valid stored token (gateway: {}), starting cloud polling",
                    token.gateway_id
                );

                let cloud_shutdown = tokio_util::sync::CancellationToken::new();
                crate::globals::set_cloud_polling_token(cloud_shutdown.clone());

                let initial_client = runbeam_sdk::RunbeamClient::new(base_url);
                let registry_clone = registry.clone();
                let machine_token = token.machine_token.clone();

                let push_config_on_startup = std::env::var("RUNBEAM_PUSH_CONFIG_ON_STARTUP")
                    .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
                    .unwrap_or(false);

                tokio::spawn(async move {
                    let client = match initial_client.discover_base_url(&machine_token).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(
                                "Base URL discovery failed (using configured URL): {}",
                                e
                            );
                            initial_client
                        }
                    };

                    if push_config_on_startup {
                        let ready_signal =
                            crate::management::cloud_poller::start_cloud_polling_when_ready(
                                client.clone(),
                                machine_token.clone(),
                                poll_interval,
                                registry_clone.clone(),
                                cloud_shutdown.clone(),
                            )
                            .await;

                        if let Err(e) = crate::management::cloud_poller::push_config_on_startup(
                            &client,
                            &machine_token,
                        )
                        .await
                        {
                            tracing::warn!(
                                "Push config on startup failed: {}. Continuing with polling.",
                                e
                            );
                        }

                        let _ = ready_signal.send(());
                        if crate::globals::trigger_cloud_poll() {
                            tracing::info!(
                                "Triggered immediate cloud poll after startup config push"
                            );
                        }
                    } else {
                        crate::management::cloud_poller::start_cloud_polling(
                            client,
                            machine_token,
                            poll_interval,
                            registry_clone,
                            cloud_shutdown,
                        )
                        .await;
                    }
                });
            }
            Ok(Some(token)) => {
                tracing::warn!(
                    "Stored token for gateway '{}' has expired (expired at: {}). Waiting for re-authorization.",
                    token.gateway_id,
                    token.expires_at
                );
            }
            Ok(None) => {
                tracing::info!(
                    "No stored token found. Gateway must be authorized via /admin/authorize endpoint."
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load stored token: {}. Waiting for authorization.",
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let config = Config::default();
        let builder = HarmonyBuilder::new(config);
        assert!(builder.plugins.is_empty());
        assert!(builder.auth_providers.is_empty());
        assert!(builder.middleware_factories.is_empty());
        assert!(builder.service_factories.is_empty());
    }

    #[test]
    fn test_builder_with_config_path() {
        let config = Config::default();
        let builder = HarmonyBuilder::new(config).with_config_path("/path/to/config.toml");
        assert_eq!(builder.config_path, Some("/path/to/config.toml".to_string()));
    }

    #[test]
    fn test_extension_registry_find_methods() {
        let registry = ExtensionRegistry::new();
        assert!(registry.find_auth_provider("nonexistent").is_none());
        assert!(registry.find_middleware_factory("nonexistent").is_none());
        assert!(registry.find_service_factory("nonexistent").is_none());
    }
}

use crate::config::logging_config::LoggingConfig;
use crate::config::proxy_config::ProxyConfig;
use crate::config::Cli;
use crate::models::backends::backends::Backend;
use crate::models::endpoints::endpoint::Endpoint;
use crate::models::middleware::middleware::{initialise_middleware_registry, MiddlewareConfig};
use crate::models::network::config::NetworkConfig;
use crate::models::pipelines::config::Pipeline;
use crate::models::services::services::initialise_service_registry;
use crate::models::services::services::ServiceConfig;
use crate::models::targets::config::TargetConfig;
use crate::storage::StorageConfig;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

static DEFAULT_OPTIONS: Lazy<HashMap<String, serde_json::Value>> = Lazy::new(HashMap::new);

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub network: HashMap<String, NetworkConfig>,
    #[serde(default)]
    pub pipelines: HashMap<String, Pipeline>,
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
    #[serde(default)]
    pub backends: HashMap<String, Backend>,
    #[serde(default)]
    pub middleware: MiddlewareInstanceConfig, // Keep the old middleware config for compatibility
    #[serde(default)]
    pub middleware_types: HashMap<String, MiddlewareConfig>, // New middleware registry config
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>,
    #[serde(default)]
    pub targets: HashMap<String, TargetConfig>,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub transforms: (),
}

impl Config {
    pub fn from_args(cli: Cli) -> Self {
        // Load the base configuration file
        let contents =
            std::fs::read_to_string(&cli.config_path).expect("Failed to read config file");
        let mut config: Config = toml::from_str(&contents).expect("Failed to parse config");

        // Attempt to load additional configs and merge them into the current config.
        if let Ok(additional_configs) = Self::load_additional_configs(&config, &cli.config_path) {
            config = Self::merge_configs(config, additional_configs);
        }

        // Initialize both registries
        config.initialize_service_registry();
        config.initialize_middleware_registry();

        // Validate the final, merged configuration
        config.validate().expect("Configuration validation failed");
        config
    }

    fn initialize_service_registry(&self) {
        initialise_service_registry(self);
    }

    fn initialize_middleware_registry(&self) {
        initialise_middleware_registry(self);
    }

    /// Loads all additional configuration files from pipelines_path and transforms_path
    fn load_additional_configs(
        config: &Config,
        base_config_path: &str,
    ) -> Result<Vec<Config>, Box<dyn std::error::Error>> {
        let base_dir = Path::new(base_config_path)
            .parent()
            .ok_or("Failed to retrieve base directory of config file")?;

        let mut configs = Vec::new();

        // Load configurations from `pipelines_path`
        let pipelines_path = base_dir.join(&config.proxy.pipelines_path);
        configs.extend(Self::load_from_directory(&pipelines_path)?);

        // Load configurations from `transforms_path`
        let transforms_path = base_dir.join(&config.proxy.transforms_path);
        configs.extend(Self::load_from_directory(&transforms_path)?);

        Ok(configs)
    }

    /// Loads configuration files from a directory
    fn load_from_directory(dir: &Path) -> Result<Vec<Config>, Box<dyn std::error::Error>> {
        if !dir.exists() {
            return Ok(vec![]); // Skip if the directory doesn't exist
        }

        let mut configs = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
                let contents = fs::read_to_string(&path)?;
                let config: Config = toml::from_str(&contents)?;
                configs.push(config);
            }
        }

        Ok(configs)
    }

    /// Merges multiple configurations into a single base configuration
    fn merge_configs(mut base: Config, additional: Vec<Config>) -> Config {
        for config in additional {
            // Example merging logic: extend fields that are HashMaps
            base.network.extend(config.network);
            base.endpoints.extend(config.endpoints);
            base.pipelines.extend(config.pipelines);
            // base.transforms.extend(config.transforms);
            base.targets.extend(config.targets);
            // Add other fields as necessary, depending on merging strategy
        }
        base
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_proxy()?;
        self.validate_networks()?;
        self.validate_services()?;
        self.validate_middleware_types()?;
        self.validate_pipelines()?;
        self.validate_endpoints()?;
        self.validate_backends()?;
        self.validate_targets()?;
        self.validate_storage()?;

        Ok(())
    }

    fn validate_proxy(&self) -> Result<(), ConfigError> {
        if self.proxy.id.trim().is_empty() {
            return Err(ConfigError::InvalidProxy {
                name: self.proxy.id.clone(),
                reason: "No proxy id provided".to_string(),
            });
        }

        // Check if the log_level is valid; default to "error" if not provided
        let valid_log_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_log_levels.contains(&self.proxy.log_level.as_str()) {
            return Err(ConfigError::InvalidProxy {
                name: self.proxy.id.clone(),
                reason: format!(
                    "Invalid log_level '{}'. Valid options are: {:?}",
                    self.proxy.log_level, valid_log_levels
                ),
            });
        }

        Ok(())
    }

    fn validate_networks(&self) -> Result<(), ConfigError> {
        for (name, network) in &self.network {
            if network.interface.trim().is_empty() {
                return Err(ConfigError::InvalidNetwork {
                    name: name.clone(),
                    reason: "interface is empty".to_string(),
                });
            }
            if network.enable_wireguard && network.http.bind_port == 0 {
                return Err(ConfigError::InvalidNetwork {
                    name: name.clone(),
                    reason: "invalid bind port for Wireguard".to_string(),
                });
            }
        }
        Ok(())
    }

    fn validate_pipelines(&self) -> Result<(), ConfigError> {
        for (name, pipeline) in &self.pipelines {
            // Warn and skip if networks are empty or do not match
            if pipeline.networks.is_empty() {
                tracing::warn!(
                    "Pipeline '{}' has no associated networks, skipping validation",
                    name
                );
                continue;
            }
            let is_network_matched = pipeline
                .networks
                .iter()
                .any(|network| self.network.contains_key(network));
            if !is_network_matched {
                tracing::warn!(
                    "Pipeline '{}' does not match any network, skipping validation",
                    name
                );
                continue;
            }

            // Warn and skip if endpoints are empty or do not match
            if pipeline.endpoints.is_empty() {
                tracing::warn!(
                    "Pipeline '{}' has no endpoints defined, skipping validation",
                    name
                );
                continue;
            }
            for endpoint in &pipeline.endpoints {
                if !self.endpoints.contains_key(endpoint) {
                    return Err(ConfigError::InvalidPipeline {
                        name: name.clone(),
                        reason: format!("unknown endpoint '{}'", endpoint),
                    });
                }
            }

            // Warn if middleware is empty
            if pipeline.middleware.is_empty() {
                tracing::warn!(
                    "Pipeline '{}' has an empty middleware of middleware/services",
                    name
                );
            }
        }
        Ok(())
    }

    fn validate_endpoints(&self) -> Result<(), ConfigError> {
        for (name, endpoint) in &self.endpoints {
            let service =
                endpoint
                    .resolve_service()
                    .map_err(|err| ConfigError::InvalidEndpoint {
                        name: name.clone(),
                        reason: err,
                    })?;

            let options = endpoint.options.as_ref().unwrap_or(&DEFAULT_OPTIONS);
            service
                .validate(options)
                .map_err(|err| ConfigError::InvalidEndpoint {
                    name: name.clone(),
                    reason: format!("Service validation failed: {:?}", err),
                })?;
        }
        Ok(())
    }

    fn validate_backends(&self) -> Result<(), ConfigError> {
        for (name, backend) in &self.backends {
            let service = backend
                .resolve_service()
                .map_err(|err| ConfigError::InvalidBackend {
                    name: name.clone(),
                    reason: err,
                })?;

            let options = backend.options.as_ref().unwrap_or(&DEFAULT_OPTIONS);
            service
                .validate(options)
                .map_err(|err| ConfigError::InvalidBackend {
                    name: name.clone(),
                    reason: format!("Service validation failed: {:?}", err),
                })?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn validate_middleware(&self) -> Result<(), ConfigError> {
        for _endpoint in self.endpoints.values() {
            // todo: Actually implement middleware validation
            // let handler = endpoint.kind.resolve_handler(name)?;
            // handler.validate()?; // Validate the resolved endpoint
        }
        Ok(())
    }

    fn validate_targets(&self) -> Result<(), ConfigError> {
        for (name, target) in &self.targets {
            if target.url.trim().is_empty() {
                return Err(ConfigError::MissingTargets {
                    name: name.clone(),
                    reason: "Target URL is empty".to_string(),
                });
            }
        }
        Ok(())
    }

    fn validate_services(&self) -> Result<(), ConfigError> {
        // @todo
        Ok(())
    }

    fn validate_middleware_types(&self) -> Result<(), ConfigError> {
        for (name, middleware_config) in &self.middleware_types {
            // Basic validation - could be extended
            if middleware_config.module.is_empty() {
                // Built-in middleware, validate that it exists
                match name.as_str() {
                    "jwtauth" | "auth" | "connect" | "passthru" => {} // Valid built-in middleware
                    _ => {
                        return Err(ConfigError::InvalidMiddleware {
                            name: name.clone(),
                            reason: format!("Unknown built-in middleware type: {}", name),
                        })
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_storage(&self) -> Result<(), ConfigError> {
        match self.storage.backend.as_str() {
            "filesystem" => {
                // Validate filesystem backend options
                if let Some(path) = self.storage.options.get("path") {
                    if let Some(path_str) = path.as_str() {
                        if path_str.trim().is_empty() {
                            return Err(ConfigError::InvalidStorage {
                                backend: self.storage.backend.clone(),
                                reason: "Storage path cannot be empty".to_string(),
                            });
                        }
                    } else {
                        return Err(ConfigError::InvalidStorage {
                            backend: self.storage.backend.clone(),
                            reason: "Storage path must be a string".to_string(),
                        });
                    }
                }
                // Path is optional and defaults to "./tmp"
                Ok(())
            }
            _ => Err(ConfigError::InvalidStorage {
                backend: self.storage.backend.clone(),
                reason: format!("Unsupported storage backend: {}", self.storage.backend),
            }),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidProxy { name: String, reason: String },
    MissingTargets { name: String, reason: String },
    InvalidEndpoint { name: String, reason: String },
    InvalidBackend { name: String, reason: String },
    InvalidNetwork { name: String, reason: String },
    InvalidPipeline { name: String, reason: String },
    InvalidMiddleware { name: String, reason: String }, // Added for middleware validation
    InvalidStorage { backend: String, reason: String }, // Added for storage validation
}

// Rename the existing MiddlewareConfig to avoid confusion
#[derive(Debug, Deserialize, Default, Clone)]
pub struct MiddlewareInstanceConfig {
    #[serde(default)]
    pub jwt_auth: Option<crate::models::middleware::types::jwtauth::JwtAuthConfig>,
    #[serde(default)]
    pub auth_sidecar: Option<crate::models::middleware::types::auth::AuthSidecarConfig>,
    #[serde(default)]
    pub aurabox_connect: Option<crate::models::middleware::types::connect::AuraboxConnectConfig>,
}

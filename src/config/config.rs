use crate::config::logging_config::LoggingConfig;
use crate::config::proxy_config::ProxyConfig;
use crate::config::resolution::resolve_references;
use crate::config::runbeam_config::RunbeamConfig;
use crate::config::Cli;
use crate::models::backends::backends::Backend;
use crate::models::endpoints::endpoint::Endpoint;
use crate::models::middleware::instance::{MiddlewareInstance, MiddlewareInstanceConfig};
use crate::models::middleware::middleware::{initialise_middleware_registry, MiddlewareConfig};
use crate::models::network::config::NetworkConfig;
use crate::models::peers::config::PeerConfig;
use crate::models::pipelines::config::Pipeline;
use crate::models::services::services::initialise_service_registry;
use crate::models::services::services::ServiceConfig;
use crate::models::services::types::management::ManagementConfig;
use crate::models::targets::config::TargetConfig;
use crate::storage::StorageConfig;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
// use serde_json::json;

static DEFAULT_OPTIONS: Lazy<HashMap<String, serde_json::Value>> = Lazy::new(HashMap::new);

/// Policy definition at top level
#[derive(Debug, Deserialize, Clone)]
pub struct PolicyDefinition {
    pub id: String,
    pub name: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<String>, // Rule IDs
}

/// Rule definition at top level
#[derive(Debug, Deserialize, Clone)]
pub struct RuleDefinition {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default = "default_weight")]
    pub weight: i64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

fn default_enabled() -> bool {
    true
}

fn default_weight() -> i64 {
    0
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub runbeam: RunbeamConfig,
    #[serde(default)]
    pub management: ManagementConfig,
    #[serde(default)]
    pub network: HashMap<String, NetworkConfig>,
    #[serde(default)]
    pub pipelines: HashMap<String, Pipeline>,
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
    #[serde(default)]
    pub backends: HashMap<String, Backend>,
    #[serde(default)]
    pub middleware: HashMap<String, MiddlewareInstance>, // Middleware instances
    #[serde(default)]
    pub middleware_legacy: MiddlewareInstanceConfig, // Keep the old middleware config for compatibility
    #[serde(default)]
    pub middleware_types: HashMap<String, MiddlewareConfig>, // New middleware registry config
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>,
    #[serde(default)]
    pub targets: HashMap<String, TargetConfig>,
    #[serde(default)]
    pub peers: HashMap<String, PeerConfig>,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub transforms: (),
    /// Top-level policy definitions
    #[serde(default)]
    pub policies: HashMap<String, PolicyDefinition>,
    /// Top-level rule definitions
    #[serde(default)]
    pub rules: HashMap<String, RuleDefinition>,
    /// Resolved absolute path to transforms directory (not serialized)
    #[serde(skip)]
    pub resolved_transforms_path: Option<String>,
}

impl Config {
    pub fn inject_management_service(&mut self) -> Result<(), ConfigError> {
        // Only inject if management is enabled
        if !self.management.enabled {
            return Ok(());
        }

        // Inject management endpoint if not already present
        if !self.endpoints.contains_key("management") {
            self.endpoints.insert(
                "management".to_string(),
                Endpoint {
                    service: "management".to_string(),
                    peer_ref: None,
                    connection: None,
                    authentication: None,
                    options: Some({
                        let mut options = HashMap::new();
                        options.insert(
                            "config".to_string(),
                            serde_json::json!({
                                "enabled": self.management.enabled,
                                "base_path": self.management.base_path,
                            }),
                        );
                        options.insert(
                            "pipelines".to_string(),
                            serde_json::to_value(&self.pipelines).unwrap_or_default(),
                        );
                        options
                    }),
                },
            );
        }

        // Inject management backend if not already present
        if !self.backends.contains_key("management") {
            self.backends.insert(
                "management".to_string(),
                Backend {
                    service: "management".to_string(),
                    target_ref: None,
                    connection: None,
                    authentication: None,
                    timeout_secs: None,
                    max_retries: None,
                    options: Some({
                        let mut options = HashMap::new();
                        options.insert(
                            "config".to_string(),
                            serde_json::json!({
                                "enabled": self.management.enabled,
                                "base_path": self.management.base_path,
                            }),
                        );
                        options.insert(
                            "pipelines".to_string(),
                            serde_json::to_value(&self.pipelines).unwrap_or_default(),
                        );
                        options
                    }),
                },
            );
        }

        // Inject management pipeline if not already present
        if !self.pipelines.contains_key("management") {
            // Use specified network or create default management network
            let network = match &self.management.network {
                Some(network_name) => {
                    if !self.network.contains_key(network_name) {
                        let available_networks: Vec<&String> = self.network.keys().collect();
                        return Err(ConfigError::InvalidManagement {
                            reason: format!(
                                "Management network '{}' not found in configuration. Available networks: {}",
                                network_name,
                                if available_networks.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    available_networks.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                                }
                            )
                        });
                    }
                    network_name.clone()
                }
                None => {
                    // Auto-generate default management network on localhost:9090
                    tracing::info!("Management API enabled without network specified, creating default management network on 127.0.0.1:9090");

                    let default_network = NetworkConfig {
                        enable_wireguard: false,
                        interface: "wg0".to_string(),
                        tcp_config: crate::models::network::config::TcpConfig {
                            bind_address: "127.0.0.1".to_string(),
                            bind_port: 9090,
                        },
                    };

                    self.network
                        .insert("management".to_string(), default_network);
                    self.management.network = Some("management".to_string());

                    "management".to_string()
                }
            };

            self.pipelines.insert(
                "management".to_string(),
                Pipeline {
                    description: "Management API pipeline".to_string(),
                    networks: vec![network],
                    endpoints: vec!["management".to_string()],
                    backends: vec!["management".to_string()],
                    middleware: Vec::new(),
                },
            );
        }

        // Ensure management service is registered
        if !self.services.contains_key("management") {
            self.services.insert(
                "management".to_string(),
                ServiceConfig {
                    module: "".to_string(),
                },
            );
        }

        Ok(())
    }

    pub fn from_args(cli: Cli) -> Self {
        // Verify the config file has a .toml extension
        let config_path = Path::new(&cli.config_path);
        if config_path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            panic!(
                "Configuration file must have a .toml extension: {}",
                cli.config_path
            );
        }

        // Load the base configuration file
        let contents =
            std::fs::read_to_string(&cli.config_path).expect("Failed to read config file");
        let mut config: Config = toml::from_str(&contents).expect("Failed to parse config");

        // Resolve transforms_path relative to config file directory
        let base_dir = config_path
            .parent()
            .expect("Failed to get config file directory");
        let transforms_path = base_dir.join(&config.proxy.transforms_path);
        config.resolved_transforms_path = Some(transforms_path.to_string_lossy().to_string());

        // Attempt to load additional configs and merge them into the current config.
        if let Ok(additional_configs) = Self::load_additional_configs(&config, &cli.config_path) {
            config = Self::merge_configs(config, additional_configs);
        }

        // Resolve references (targets/peers)
        if let Err(e) = resolve_references(&mut config) {
            panic!("Configuration reference resolution failed: {}", e);
        }

        // Inject management service if enabled
        config
            .inject_management_service()
            .expect("Failed to inject management service");

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
            // Extend fields loaded from per-file configs
            base.network.extend(config.network);
            base.endpoints.extend(config.endpoints);
            base.backends.extend(config.backends);
            base.pipelines.extend(config.pipelines);
            // base.transforms.extend(config.transforms);
            base.targets.extend(config.targets);
            base.peers.extend(config.peers);
            // Merge middleware instances
            base.middleware.extend(config.middleware);
            // Merge middleware registries if provided
            base.middleware_types.extend(config.middleware_types);
            // Merge services if provided
            base.services.extend(config.services);
            // Merge policies and rules
            base.policies.extend(config.policies);
            base.rules.extend(config.rules);
        }
        base
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_proxy()?;
        self.validate_logging()?;
        self.validate_runbeam()?;
        self.validate_networks()?;
        self.validate_management()?;
        self.validate_services()?;
        self.validate_middleware_types()?;
        self.validate_pipelines()?;
        self.validate_endpoints()?;
        self.validate_backends()?;
        self.validate_targets()?;
        self.validate_peers()?;
        self.validate_storage()?;

        Ok(())
    }

    fn validate_proxy(&self) -> Result<(), ConfigError> {
        self.proxy
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: self.proxy.id.clone(),
                reason: e,
            })
    }

    fn validate_logging(&self) -> Result<(), ConfigError> {
        self.logging
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: "logging".to_string(),
                reason: e,
            })
    }

    fn validate_runbeam(&self) -> Result<(), ConfigError> {
        self.runbeam
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: "runbeam".to_string(),
                reason: e,
            })
    }

    fn validate_networks(&self) -> Result<(), ConfigError> {
        for (name, network) in &self.network {
            if network.interface.trim().is_empty() {
                return Err(ConfigError::InvalidNetwork {
                    name: name.clone(),
                    reason: "interface is empty".to_string(),
                });
            }
            if network.enable_wireguard && network.tcp_config.bind_port == 0 {
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
            // Check if service type is allowed as endpoint
            match endpoint.service.to_lowercase().as_str() {
                "dicom_scu" => {
                    return Err(ConfigError::InvalidEndpoint {
                        name: name.clone(),
                        reason: "Service 'dicom_scu' cannot be used as an endpoint. Use 'dicom_scp' for DICOM endpoints.".to_string(),
                    });
                }
                // "dicom" is allowed for backward compatibility (maps to dicom_scu)
                // but should only be used as a backend, not endpoint
                "dicom" => {
                    return Err(ConfigError::InvalidEndpoint {
                        name: name.clone(),
                        reason: "Service 'dicom' (legacy name) cannot be used as an endpoint. Use 'dicom_scp' for DICOM endpoints.".to_string(),
                    });
                }
                _ => {}
            }

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
            // Check if service type is allowed as backend
            match backend.service.to_lowercase().as_str() {
                "dicom_scp" => {
                    return Err(ConfigError::InvalidBackend {
                        name: name.clone(),
                        reason: "Service 'dicom_scp' cannot be used as a backend. Use 'dicom_scu' for DICOM backends.".to_string(),
                    });
                }
                _ => {}
            }

            // Try to resolve the backend service; if it fails, warn and skip validation
            let service = match backend.resolve_service() {
                Ok(svc) => svc,
                Err(err) => {
                    tracing::warn!(
                        "Skipping backend '{}' due to service resolution error: {}",
                        name,
                        err
                    );
                    continue;
                }
            };

            let options = backend.options.as_ref().unwrap_or(&DEFAULT_OPTIONS);
            if let Err(err) = service.validate(options) {
                tracing::warn!(
                    "Skipping backend '{}' due to validation error: {:?}",
                    name,
                    err
                );
                continue;
            }
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
            target.validate().map_err(|e| ConfigError::InvalidTarget {
                name: name.clone(),
                reason: e,
            })?;
        }
        Ok(())
    }

    fn validate_peers(&self) -> Result<(), ConfigError> {
        for (name, peer) in &self.peers {
            peer.validate().map_err(|e| ConfigError::InvalidPeer {
                name: name.clone(),
                reason: e,
            })?;
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
                    "jwtauth" | "basic_auth" | "connect" | "passthru" | "json_extractor"
                    | "json" | "jmix_builder" | "dicomweb_bridge" | "dicomweb" | "transform"
                    | "metadata_transform" | "path_filter" | "policies" => {}
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

    fn validate_management(&self) -> Result<(), ConfigError> {
        self.management
            .validate()
            .map_err(|err| ConfigError::InvalidManagement { reason: err })
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
    InvalidTarget { name: String, reason: String },
    InvalidPeer { name: String, reason: String },
    InvalidManagement { reason: String },
    InvalidEndpoint { name: String, reason: String },
    InvalidBackend { name: String, reason: String },
    InvalidNetwork { name: String, reason: String },
    InvalidPipeline { name: String, reason: String },
    InvalidMiddleware { name: String, reason: String }, // Added for middleware validation
    InvalidStorage { backend: String, reason: String }, // Added for storage validation
}

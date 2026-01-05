use crate::config::env_substitution::substitute_env_vars;
use crate::config::logging_config::LoggingConfig;
use crate::config::provider_config::ProviderConfig;
use crate::config::proxy_config::ProxyConfig;
use crate::config::resolution::resolve_references;
use crate::config::runbeam_config::RunbeamConfig;
use crate::config::Cli;
use crate::models::backends::backends::Backend;
use crate::models::connection::AuthenticationDefinition;
use crate::models::endpoints::endpoint::Endpoint;
use crate::models::mesh::config::{Mesh, MeshEgress, MeshIngress, RemoteIngress};
use crate::models::middleware::instance::MiddlewareInstance;
use crate::models::middleware::middleware::{initialise_middleware_registry, MiddlewareConfig};
use crate::models::network::config::NetworkConfig;
use crate::models::peers::config::PeerConfig;
use crate::models::pipelines::config::Pipeline;
use crate::models::services::services::initialise_service_registry;
use crate::models::services::services::ServiceConfig;
use crate::management::ManagementConfig;
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
    /// Provider configurations for resource resolution
    #[serde(default)]
    pub provider: HashMap<String, ProviderConfig>,
    /// [DEPRECATED] Use provider.runbeam instead
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
    pub middleware_types: HashMap<String, MiddlewareConfig>, // Middleware registry config
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
    /// Top-level authentication definitions (DSL v1.9.0+)
    #[serde(default)]
    pub authentications: HashMap<String, AuthenticationDefinition>,
    /// Top-level policy definitions
    #[serde(default)]
    pub policies: HashMap<String, PolicyDefinition>,
    /// Top-level rule definitions
    #[serde(default)]
    pub rules: HashMap<String, RuleDefinition>,
    /// Data mesh definitions
    #[serde(default)]
    pub mesh: HashMap<String, Mesh>,
    /// Mesh ingress definitions
    #[serde(default)]
    pub ingress: HashMap<String, MeshIngress>,
    /// Mesh egress definitions
    #[serde(default)]
    pub egress: HashMap<String, MeshEgress>,
    /// Remote ingress definitions (URLs of remote mesh members)
    #[serde(default)]
    pub remote_ingress: HashMap<String, RemoteIngress>,
    /// Resolved absolute path to transforms directory (not serialized)
    #[serde(skip)]
    pub resolved_transforms_path: Option<String>,
    /// Resolved absolute path to mesh directory (not serialized)
    #[serde(skip)]
    pub resolved_mesh_path: Option<String>,
    /// Backends that could not be resolved due to missing targets (not serialized)
    #[serde(skip)]
    pub unresolved_backends: std::collections::HashSet<String>,
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
                        tcp_config: Some(crate::models::network::config::TcpConfig {
                            bind_address: "127.0.0.1".to_string(),
                            bind_port: 9090,
                            cert_path: None,
                            key_path: None,
                            force_https: false,
                        }),
                        http3: None,
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
                    middleware: crate::models::pipelines::config::PipelineMiddleware::default(),
                    mesh: crate::models::pipelines::config::PipelineMesh::default(),
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

    /// Get provider configuration by name.
    /// Falls back to legacy [runbeam] config if name is "runbeam" and no explicit provider exists.
    pub fn get_provider(&self, name: &str) -> Option<ProviderConfig> {
        // Check explicit provider first
        if let Some(provider) = self.provider.get(name) {
            return Some(provider.clone());
        }

        // Fallback for "runbeam" to legacy config (only if enabled)
        if name == "runbeam" && self.runbeam.enabled {
            return Some(ProviderConfig {
                api: self.runbeam.cloud_api_base_url.clone(),
                poll_interval_secs: self.runbeam.poll_interval_secs,
            });
        }

        // Local provider is always available (implicitly, no polling)
        if name == "local" {
            return Some(ProviderConfig {
                api: None,
                poll_interval_secs: 0,
            });
        }

        None
    }

    /// Get the primary provider configuration.
    /// Defaults to "runbeam" if not specified.
    pub fn get_primary_provider(&self) -> Option<ProviderConfig> {
        self.get_provider(&self.proxy.primary_provider)
    }

    /// Get the poll interval from the primary provider.
    /// Returns None if the primary provider doesn't exist or polling is disabled.
    pub fn primary_poll_interval(&self) -> Option<std::time::Duration> {
        self.get_primary_provider().and_then(|p| {
            if p.polling_enabled() {
                Some(std::time::Duration::from_secs(p.poll_interval_secs))
            } else {
                None
            }
        })
    }

    /// Get the API base URL from the primary provider.
    /// Falls back to the default Runbeam Cloud URL if not specified.
    pub fn primary_api_base_url(&self) -> String {
        self.get_primary_provider()
            .and_then(|p| p.api)
            .unwrap_or_else(|| "https://api.runbeam.cloud".to_string())
    }

    /// Check if the primary provider is enabled and has cloud polling configured.
    pub fn is_cloud_enabled(&self) -> bool {
        // For backward compatibility: check legacy [runbeam] section first
        if self.runbeam.enabled {
            return true;
        }
        // Then check primary provider
        self.get_primary_provider()
            .map(|p| p.is_remote() && p.polling_enabled())
            .unwrap_or(false)
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
        
        // Apply environment variable substitution
        let (contents_substituted, _audit) = substitute_env_vars(&contents);
        let mut config: Config = toml::from_str(&contents_substituted).expect("Failed to parse config");

        // Resolve transforms_path relative to config file directory
        let base_dir = config_path
            .parent()
            .expect("Failed to get config file directory");
        let transforms_path = base_dir.join(&config.proxy.transforms_path);
        config.resolved_transforms_path = Some(transforms_path.to_string_lossy().to_string());

        // Resolve mesh_path relative to config file directory
        let mesh_path = base_dir.join(&config.proxy.mesh_path);
        config.resolved_mesh_path = Some(mesh_path.to_string_lossy().to_string());

        // Attempt to load additional configs and merge them into the current config.
        match Self::load_additional_configs(&config, &cli.config_path) {
            Ok(additional_configs) => {
                config = Self::merge_configs(config, additional_configs);
            }
            Err(e) => {
                tracing::error!("Failed to load additional configurations: {}", e);
            }
        }

        // Resolve references (targets/peers)
        match resolve_references(&mut config) {
            Ok(unresolved) => {
                config.unresolved_backends = unresolved;
            }
            Err(e) => {
                panic!("Configuration reference resolution failed: {}", e);
            }
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

        // Load configurations from `mesh_path`
        let mesh_path = base_dir.join(&config.proxy.mesh_path);
        configs.extend(Self::load_from_directory(&mesh_path)?);

        Ok(configs)
    }

    /// Loads configuration files from a directory
    fn load_from_directory(dir: &Path) -> Result<Vec<Config>, Box<dyn std::error::Error>> {
        if !dir.exists() {
            return Ok(vec![]); // Skip if the directory doesn't exist
        }

        let mut configs = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
                match fs::read_to_string(&path) {
                    Ok(contents) => {
                        // Apply environment variable substitution
                        let (contents_substituted, _audit) = substitute_env_vars(&contents);
                        match toml::from_str(&contents_substituted) {
                            Ok(config) => configs.push(config),
                            Err(e) => {
                                tracing::error!("Failed to parse config file {:?}: {}", path, e);
                            }
                        }
                    },
                    Err(e) => {
                        tracing::error!("Failed to read config file {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(configs)
    }

    /// Merges multiple configurations into a single base configuration
    fn merge_configs(mut base: Config, additional: Vec<Config>) -> Config {
        for config in additional {
            // Extend fields loaded from per-file configs
            base.provider.extend(config.provider);
            base.network.extend(config.network);
            base.endpoints.extend(config.endpoints);
            base.backends.extend(config.backends);
            // Debug log pipelines being merged
            for (name, pipeline) in &config.pipelines {
                tracing::debug!(
                    "Merging pipeline '{}': left_chain={:?}, right_chain={:?}",
                    name,
                    pipeline.middleware.left_chain(),
                    pipeline.middleware.right_chain()
                );
            }
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
            // Merge mesh configurations
            base.mesh.extend(config.mesh);
            base.ingress.extend(config.ingress);
            base.egress.extend(config.egress);
            base.remote_ingress.extend(config.remote_ingress);
        }

        // Extract nested mesh ingress/egress from pipelines to top-level
        base.extract_pipeline_mesh();

        base
    }

    /// Extract nested mesh ingress/egress definitions from pipelines to top-level config.
    /// This allows the DSL format `[pipelines.my_pipeline.mesh.ingress.my_ingress]` to be
    /// promoted to the top-level `ingress` and `egress` maps for validation and routing.
    fn extract_pipeline_mesh(&mut self) {
        for (pipeline_name, pipeline) in &self.pipelines {
            // Extract ingress definitions
            let extracted_ingress = pipeline.extract_ingress(pipeline_name);
            for (name, ingress) in extracted_ingress {
                if self.ingress.contains_key(&name) {
                    tracing::warn!(
                        "Ingress '{}' from pipeline '{}' conflicts with existing ingress, skipping",
                        name,
                        pipeline_name
                    );
                } else {
                    tracing::debug!(
                        "Extracted ingress '{}' from pipeline '{}'",
                        name,
                        pipeline_name
                    );
                    self.ingress.insert(name, ingress);
                }
            }

            // Extract egress definitions
            let extracted_egress = pipeline.extract_egress(pipeline_name);
            for (name, egress) in extracted_egress {
                if self.egress.contains_key(&name) {
                    tracing::warn!(
                        "Egress '{}' from pipeline '{}' conflicts with existing egress, skipping",
                        name,
                        pipeline_name
                    );
                } else {
                    tracing::debug!(
                        "Extracted egress '{}' from pipeline '{}'",
                        name,
                        pipeline_name
                    );
                    self.egress.insert(name, egress);
                }
            }
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_proxy()?;
        self.validate_logging()?;
        self.validate_providers()?;
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
        self.validate_mesh()?;

        Ok(())
    }

    fn validate_proxy(&self) -> Result<(), ConfigError> {
        // Validate basic proxy config
        self.proxy
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: self.proxy.effective_id().to_string(),
                reason: e,
            })?;

        // Enforce required environment variables (Harmony DSL 1.9.0)
        for var in &self.proxy.required_env_vars {
            if std::env::var(var).is_err() {
                return Err(ConfigError::InvalidProxy {
                    name: self.proxy.effective_id().to_string(),
                    reason: format!("Missing required environment variable: {}", var),
                });
            }
        }
        Ok(())
    }

    fn validate_logging(&self) -> Result<(), ConfigError> {
        self.logging
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: "logging".to_string(),
                reason: e,
            })
    }

    fn validate_providers(&self) -> Result<(), ConfigError> {
        for (name, provider) in &self.provider {
            provider.validate(name).map_err(|e| ConfigError::InvalidProvider {
                name: name.clone(),
                reason: e,
            })?;
        }
        Ok(())
    }

    fn validate_runbeam(&self) -> Result<(), ConfigError> {
        // Log deprecation warning if legacy [runbeam] section is used
        if self.runbeam.enabled && !self.provider.contains_key("runbeam") {
            tracing::warn!(
                "Deprecated [runbeam] section in config. Migrate to [provider.runbeam] format."
            );
        }

        self.runbeam
            .validate()
            .map_err(|e| ConfigError::InvalidProxy {
                name: "runbeam".to_string(),
                reason: e,
            })
    }

    fn validate_networks(&self) -> Result<(), ConfigError> {
        use crate::models::network::config::TcpConfig;

        for (name, network) in &self.network {
            if network.interface.trim().is_empty() {
                return Err(ConfigError::InvalidNetwork {
                    name: name.clone(),
                    reason: "interface is empty".to_string(),
                });
            }

            // When WireGuard is enabled, ensure there is a valid TCP bind port configured.
            if network.enable_wireguard {
                let effective_tcp: TcpConfig = network.tcp_config.clone().unwrap_or_default();
                if effective_tcp.bind_port == 0 {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: "invalid bind port for Wireguard".to_string(),
                    });
                }
            }

            // Basic sanity checks for HTTP/3 configuration, if present.
            if let Some(http3) = &network.http3 {
                if http3.bind_address.trim().is_empty() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: "http3.bind_address is empty".to_string(),
                    });
                }
                if http3.bind_port == 0 {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: "http3.bind_port must be non-zero".to_string(),
                    });
                }
                if http3.cert_path.trim().is_empty() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: "http3.cert_path is empty".to_string(),
                    });
                }
                if http3.key_path.trim().is_empty() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: "http3.key_path is empty".to_string(),
                    });
                }

                // Validate cert file exists and is readable
                let cert_path = std::path::Path::new(&http3.cert_path);
                if !cert_path.exists() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: format!("http3.cert_path '{}' does not exist", http3.cert_path),
                    });
                }
                if !cert_path.is_file() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: format!("http3.cert_path '{}' is not a file", http3.cert_path),
                    });
                }

                // Validate key file exists and is readable
                let key_path = std::path::Path::new(&http3.key_path);
                if !key_path.exists() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: format!("http3.key_path '{}' does not exist", http3.key_path),
                    });
                }
                if !key_path.is_file() {
                    return Err(ConfigError::InvalidNetwork {
                        name: name.clone(),
                        reason: format!("http3.key_path '{}' is not a file", http3.key_path),
                    });
                }
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

            // Warn and skip if backends are unresolved
            let unresolved_pipeline_backends: Vec<&String> = pipeline.backends
                .iter()
                .filter(|b| self.unresolved_backends.contains(*b))
                .collect();
            if !unresolved_pipeline_backends.is_empty() {
                tracing::warn!(
                    "Pipeline '{}' references unresolved backends {:?}, skipping pipeline",
                    name,
                    unresolved_pipeline_backends
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
                    "Pipeline '{}' has no middleware configured",
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
            // Skip validation for backends with unresolved targets
            if self.unresolved_backends.contains(name) {
                tracing::debug!("Skipping validation for unresolved backend '{}'", name);
                continue;
            }

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
        use crate::models::middleware::middleware::builtin_middleware_types;

        for (name, middleware_config) in &self.middleware_types {
            // Basic validation - could be extended
            if middleware_config.module.is_empty() {
                // Built-in middleware, validate that it exists
                let name_lower = name.to_lowercase();
                if !builtin_middleware_types().contains(&name_lower.as_str()) {
                    return Err(ConfigError::InvalidMiddleware {
                        name: name.clone(),
                        reason: format!("Unknown built-in middleware type: {}", name),
                    });
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

    fn validate_mesh(&self) -> Result<(), ConfigError> {
        // Validate mesh ingress definitions
        for (name, ingress) in &self.ingress {
            ingress.validate().map_err(|e| ConfigError::InvalidMeshIngress {
                name: name.clone(),
                reason: e,
            })?;

            // Verify ingress references a valid pipeline
            let pipeline = self.pipelines.get(&ingress.pipeline).ok_or_else(|| {
                ConfigError::InvalidMeshIngress {
                    name: name.clone(),
                    reason: format!("Ingress references unknown pipeline '{}'", ingress.pipeline),
                }
            })?;

            // If endpoint override is specified, verify it belongs to the pipeline
            if let Some(ref ep) = ingress.endpoint {
                if !pipeline.endpoints.contains(ep) {
                    return Err(ConfigError::InvalidMeshIngress {
                        name: name.clone(),
                        reason: format!(
                            "Ingress endpoint override '{}' not in pipeline '{}'",
                            ep, ingress.pipeline
                        ),
                    });
                }
            } else if pipeline.endpoints.is_empty() {
                return Err(ConfigError::InvalidMeshIngress {
                    name: name.clone(),
                    reason: format!(
                        "Pipeline '{}' has no endpoints for ingress fallback",
                        ingress.pipeline
                    ),
                });
            }
        }

        // Validate mesh egress definitions
        for (name, egress) in &self.egress {
            egress.validate().map_err(|e| ConfigError::InvalidMeshEgress {
                name: name.clone(),
                reason: e,
            })?;

            // Verify egress references a valid pipeline
            let pipeline = self.pipelines.get(&egress.pipeline).ok_or_else(|| {
                ConfigError::InvalidMeshEgress {
                    name: name.clone(),
                    reason: format!("Egress references unknown pipeline '{}'", egress.pipeline),
                }
            })?;

            // If backend override is specified, verify it belongs to the pipeline
            if let Some(ref be) = egress.backend {
                if !pipeline.backends.contains(be) {
                    return Err(ConfigError::InvalidMeshEgress {
                        name: name.clone(),
                        reason: format!(
                            "Egress backend override '{}' not in pipeline '{}'",
                            be, egress.pipeline
                        ),
                    });
                }
            } else if pipeline.backends.is_empty() {
                return Err(ConfigError::InvalidMeshEgress {
                    name: name.clone(),
                    reason: format!(
                        "Pipeline '{}' has no backends for egress fallback",
                        egress.pipeline
                    ),
                });
            }
        }

        // Validate mesh definitions
        for (name, mesh) in &self.mesh {
            mesh.validate().map_err(|e| ConfigError::InvalidMesh {
                name: name.clone(),
                reason: e,
            })?;

            // For Runbeam provider, mesh id should be explicitly set
            if mesh.provider == crate::models::mesh::config::MeshProvider::Runbeam && mesh.id.is_none() {
                tracing::warn!(
                    "Runbeam mesh '{}' does not have 'id' field set. Sync config from Runbeam Cloud to populate the id.",
                    name
                );
            }

            // Verify all mesh ingress references
            for ingress_ref in &mesh.ingress {
                self.validate_mesh_ingress_reference(name, ingress_ref)?;
            }

            // Verify all mesh egress references
            for egress_ref in &mesh.egress {
                self.validate_mesh_egress_reference(name, egress_ref)?;
            }
        }

        Ok(())
    }

    /// Validate a mesh ingress reference.
    /// Supports both local names and provider-based references.
    /// Missing ingress items are logged as warnings and do not cause validation to fail.
    fn validate_mesh_ingress_reference(&self, mesh_name: &str, reference: &str) -> Result<(), ConfigError> {
        use crate::config::resource_reference::ParsedReference;

        // Parse the reference
        let parsed = match ParsedReference::parse(reference) {
            Ok(p) => p,
            Err(e) => {
                return Err(ConfigError::InvalidMesh {
                    name: mesh_name.to_string(),
                    reason: format!("Invalid ingress reference '{}': {}", reference, e),
                });
            }
        };

        // For local references, verify the resource exists locally
        if parsed.is_local() {
            if let Some(local_name) = parsed.local_name() {
                // Check local ingress and remote_ingress maps
                if !self.ingress.contains_key(local_name) && !self.remote_ingress.contains_key(local_name) {
                    tracing::warn!(
                        "Mesh '{}' references missing local ingress '{}' (not found in ingress or remote_ingress)",
                        mesh_name, local_name
                    );
                }
            }
        } else {
            // Remote reference - verify the provider exists
            if self.get_provider(&parsed.provider).is_none() {
                tracing::warn!(
                    "Mesh '{}' references missing remote ingress '{}' with unknown provider '{}'",
                    mesh_name, reference, parsed.provider
                );
            } else {
                // Remote references will be resolved at runtime
                tracing::debug!(
                    "Mesh '{}' has remote ingress reference '{}' (provider: {})",
                    mesh_name, reference, parsed.provider
                );
            }
        }

        Ok(())
    }

    /// Validate a mesh egress reference.
    /// Supports both local names and provider-based references.
    /// Missing egress items are logged as warnings and do not cause validation to fail.
    fn validate_mesh_egress_reference(&self, mesh_name: &str, reference: &str) -> Result<(), ConfigError> {
        use crate::config::resource_reference::ParsedReference;

        // Parse the reference
        let parsed = match ParsedReference::parse(reference) {
            Ok(p) => p,
            Err(e) => {
                return Err(ConfigError::InvalidMesh {
                    name: mesh_name.to_string(),
                    reason: format!("Invalid egress reference '{}': {}", reference, e),
                });
            }
        };

        // For local references, verify the resource exists locally
        if parsed.is_local() {
            if let Some(local_name) = parsed.local_name() {
                if !self.egress.contains_key(local_name) {
                    tracing::warn!(
                        "Mesh '{}' references missing local egress '{}'",
                        mesh_name, local_name
                    );
                }
            }
        } else {
            // Remote reference - verify the provider exists
            if self.get_provider(&parsed.provider).is_none() {
                tracing::warn!(
                    "Mesh '{}' references missing remote egress '{}' with unknown provider '{}'",
                    mesh_name, reference, parsed.provider
                );
            } else {
                // Remote references will be resolved at runtime
                tracing::debug!(
                    "Mesh '{}' has remote egress reference '{}' (provider: {})",
                    mesh_name, reference, parsed.provider
                );
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidProxy { name: String, reason: String },
    InvalidProvider { name: String, reason: String },
    InvalidTarget { name: String, reason: String },
    InvalidPeer { name: String, reason: String },
    InvalidManagement { reason: String },
    InvalidEndpoint { name: String, reason: String },
    InvalidBackend { name: String, reason: String },
    InvalidNetwork { name: String, reason: String },
    InvalidPipeline { name: String, reason: String },
    InvalidMiddleware { name: String, reason: String },
    InvalidStorage { backend: String, reason: String },
    InvalidMesh { name: String, reason: String },
    InvalidMeshIngress { name: String, reason: String },
    InvalidMeshEgress { name: String, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_primary_provider_default() {
        // Default config has primary_provider = "runbeam"
        // but without explicit provider config, it falls back to legacy runbeam check
        let config = Config::default();
        assert_eq!(config.proxy.primary_provider, "runbeam");

        // Legacy runbeam is disabled by default, so get_primary_provider returns None
        // unless runbeam.enabled is true or explicit provider.runbeam exists
        let primary = config.get_primary_provider();
        assert!(primary.is_none());
    }

    #[test]
    fn test_get_primary_provider_with_explicit_provider() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://api.runbeam.cloud".to_string()),
                poll_interval_secs: 60,
            },
        );

        let primary = config.get_primary_provider();
        assert!(primary.is_some());
        let p = primary.unwrap();
        assert_eq!(p.api, Some("https://api.runbeam.cloud".to_string()));
        assert_eq!(p.poll_interval_secs, 60);
    }

    #[test]
    fn test_get_primary_provider_local() {
        let mut config = Config::default();
        config.proxy.primary_provider = "local".to_string();

        // "local" is always available implicitly
        let primary = config.get_primary_provider();
        assert!(primary.is_some());
        let p = primary.unwrap();
        assert!(p.api.is_none());
        assert_eq!(p.poll_interval_secs, 0); // Local has no polling
    }

    #[test]
    fn test_primary_poll_interval() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://api.runbeam.cloud".to_string()),
                poll_interval_secs: 45,
            },
        );

        let interval = config.primary_poll_interval();
        assert!(interval.is_some());
        assert_eq!(interval.unwrap(), std::time::Duration::from_secs(45));
    }

    #[test]
    fn test_primary_poll_interval_disabled() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://api.runbeam.cloud".to_string()),
                poll_interval_secs: 0, // Polling disabled
            },
        );

        let interval = config.primary_poll_interval();
        assert!(interval.is_none());
    }

    #[test]
    fn test_primary_poll_interval_local() {
        let mut config = Config::default();
        config.proxy.primary_provider = "local".to_string();

        // Local provider has poll_interval_secs = 0
        let interval = config.primary_poll_interval();
        assert!(interval.is_none());
    }

    #[test]
    fn test_primary_api_base_url() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://custom.api.com".to_string()),
                poll_interval_secs: 30,
            },
        );

        let url = config.primary_api_base_url();
        assert_eq!(url, "https://custom.api.com");
    }

    #[test]
    fn test_primary_api_base_url_fallback() {
        let config = Config::default();
        // No provider configured, falls back to default
        let url = config.primary_api_base_url();
        assert_eq!(url, "https://api.runbeam.cloud");
    }

    #[test]
    fn test_is_cloud_enabled_with_legacy_runbeam() {
        let mut config = Config::default();
        config.runbeam.enabled = true;

        assert!(config.is_cloud_enabled());
    }

    #[test]
    fn test_is_cloud_enabled_with_provider() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://api.runbeam.cloud".to_string()),
                poll_interval_secs: 30,
            },
        );

        assert!(config.is_cloud_enabled());
    }

    #[test]
    fn test_is_cloud_enabled_local() {
        let mut config = Config::default();
        config.proxy.primary_provider = "local".to_string();

        // Local provider is not remote, so cloud is not enabled
        assert!(!config.is_cloud_enabled());
    }

    #[test]
    fn test_is_cloud_enabled_disabled_polling() {
        let mut config = Config::default();
        config.provider.insert(
            "runbeam".to_string(),
            ProviderConfig {
                api: Some("https://api.runbeam.cloud".to_string()),
                poll_interval_secs: 0, // Polling disabled
            },
        );

        // Has remote API but polling is disabled
        assert!(!config.is_cloud_enabled());
    }
}

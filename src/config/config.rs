use std::collections::HashMap;
use once_cell::sync::Lazy;
use serde::Deserialize;
use crate::models::backends::config::Backend;
use crate::config::Cli;
use crate::config::logging_config::LoggingConfig;
use crate::config::proxy_config::ProxyConfig;
use crate::models::endpoints::config::Endpoint;
use crate::models::groups::config::Group;
use crate::models::middleware::config::MiddlewareConfig;
use crate::models::network::config::NetworkConfig;
use crate::models::services::config::ServiceConfig;
use crate::models::targets::config::TargetConfig;

static DEFAULT_OPTIONS: Lazy<HashMap<String, serde_json::Value>> = Lazy::new(|| HashMap::new());

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub network: HashMap<String, NetworkConfig>,
    #[serde(default)]
    pub groups: HashMap<String, Group>,
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
    #[serde(default)]
    pub backends: HashMap<String, Backend>,
    #[serde(default)]
    pub middleware: MiddlewareConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>, // Added to support `basic.toml` services
    #[serde(default)]
    pub targets: HashMap<String, TargetConfig>,  // Added for backend target support
}

impl Config {

    pub fn from_args(cli: Cli) -> Self {
        let contents = std::fs::read_to_string(cli.config_path)
            .expect("Failed to read config file");
        let config: Config = toml::from_str(&contents).expect("Failed to parse config");
        config.validate().expect("Configuration validation failed");
        config
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_proxy()?;
        self.validate_networks()?;
        self.validate_groups()?;
        self.validate_endpoints()?;
        self.validate_backends()?;
        self.validate_targets()?;
        Ok(())
    }

    fn validate_proxy(&self) -> Result<(), ConfigError> {
        if self.proxy.id.trim().is_empty() {
            return Err(ConfigError::InvalidProxy {
                name: self.proxy.id.clone(),
                reason: "No proxy id provided".to_string()
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

        // Check if store_dir is set; default to "/tmp"
        if self.proxy.store_dir.trim().is_empty() {
            return Err(ConfigError::InvalidProxy {
                name: self.proxy.id.clone(),
                reason: "store_dir cannot be empty".to_string(),
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

    fn validate_groups(&self) -> Result<(), ConfigError> {
        for (name, group) in &self.groups {
            // Warn and skip if networks are empty or do not match
            if group.networks.is_empty() {
                tracing::warn!("Group '{}' has no associated networks, skipping validation", name);
                continue;
            }
            let is_network_matched = group.networks.iter().any(|network| self.network.contains_key(network));
            if !is_network_matched {
                tracing::warn!("Group '{}' does not match any network, skipping validation", name);
                continue;
            }

            // Warn and skip if endpoints are empty or do not match
            if group.endpoints.is_empty() {
                tracing::warn!("Group '{}' has no endpoints defined, skipping validation", name);
                continue;
            }
            for endpoint in &group.endpoints {
                if !self.endpoints.contains_key(endpoint) {
                    return Err(ConfigError::InvalidGroup {
                        name: name.clone(),
                        reason: format!("unknown endpoint '{}'", endpoint),
                    });
                }
            }

            // Warn if middleware is empty
            if group.middleware.is_empty() {
                tracing::warn!("Group '{}' has an empty middleware of middleware/services", name);
            }

            // Warn and skip if backends are empty or do not match
            if group.backends.is_empty() {
                tracing::warn!("Group '{}' has no backends defined, skipping validation", name);
                continue;
            }
            for backend in &group.backends {
                if !self.backends.contains_key(backend) {
                    return Err(ConfigError::InvalidGroup {
                        name: name.clone(),
                        reason: format!("unknown backend '{}'", backend),
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_endpoints(&self) -> Result<(), ConfigError> {
        for (name, endpoint) in &self.endpoints {
            let options = endpoint.options.as_ref().unwrap_or(&DEFAULT_OPTIONS);
            endpoint.kind.validate(options)?; // Validate with options
        }
        Ok(())
    }

    fn validate_middleware(&self) -> Result<(), ConfigError> {
        for (name, endpoint) in &self.endpoints {
            // todo: Actually implement middleware validation
            // let handler = endpoint.kind.resolve_handler(name)?;
            // handler.validate()?; // Validate the resolved endpoint
        }
        Ok(())
    }

    fn validate_backends(&self) -> Result<(), ConfigError> {
        for (name, backend) in &self.backends {
            if backend.targets.is_empty() {
                return Err(ConfigError::InvalidBackend {
                    name: name.clone(),
                    reason: "no targets specified".to_string(),
                });
            }
            for target in &backend.targets {
                if !self.targets.contains_key(target) {
                    return Err(ConfigError::InvalidBackend {
                        name: name.clone(),
                        reason: format!("unknown target '{}'", target),
                    });
                }
            }
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
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidProxy { name: String, reason: String },
    MissingTargets { name: String, reason: String },
    InvalidEndpoint { name: String, reason: String },
    InvalidBackend { name: String, reason: String },
    InvalidNetwork { name: String, reason: String },
    InvalidGroup { name: String, reason: String },
}
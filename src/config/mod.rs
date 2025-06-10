mod config;

use clap::Parser;
use serde::Deserialize;
use std::collections::{HashMap};
use std::fs;

use crate::endpoints::config::Endpoint;
use crate::backends::config::Backend;
use crate::network::config::NetworkConfig;
use crate::middleware::config::MiddlewareConfig;
use crate::groups::config::Group;
use crate::config::config::*;
use crate::backends::dicom::{validate_dicom_backend, validate_dicom_endpoint};

#[derive(Parser, Debug)]
#[command(name = "harmony")]
#[command(about = "Harmony proxy", long_about = None)]
pub struct Cli {
    #[arg(short, long, default_value = "/etc/harmony/harmony-config.toml")]
    pub config: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub proxy: ProxyConfig,
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
}


impl Config {

    pub fn from_args() -> Self {
        let args = Cli::parse();
        let contents = fs::read_to_string(args.config).expect("Failed to read config file");
        let config: Config = toml::from_str(&contents).expect("Failed to parse config");
        config.validate().expect("Invalid configuration");
        config
    }
    
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.proxy.validate()?;
        self.validate_networks()?;
        self.validate_groups()?;
        self.validate_references()?;
        Ok(())
    }

    fn validate_networks(&self) -> Result<(), ConfigError> {
        if self.network.is_empty() {
            return Err(ConfigError::MissingNetworks);
        }

        for (network_name, network) in &self.network {
            if network.enable_wireguard {
                if network.interface.trim().is_empty() {
                    return Err(ConfigError::InvalidNetwork {
                        name: network_name.clone(),
                        reason: "WireGuard enabled but interface not specified".to_string(),
                    });
                }
            }

            // Validate HTTP config
            if network.http.bind_address.trim().is_empty() {
                return Err(ConfigError::InvalidNetwork {
                    name: network_name.clone(),
                    reason: "HTTP bind address not specified".to_string(),
                });
            }

            if network.http.bind_port == 0 {
                return Err(ConfigError::InvalidNetwork {
                    name: network_name.clone(),
                    reason: "HTTP bind port must be non-zero".to_string(),
                });
            }
        }
        Ok(())
    }


    fn validate_groups(&self) -> Result<(), ConfigError> {
        if self.groups.is_empty() {
            return Err(ConfigError::MissingGroups);
        }

        for (group_name, group) in &self.groups {
            // Ensure group has at least one network
            if group.networks.is_empty() {
                return Err(ConfigError::InvalidGroup {
                    name: group_name.clone(),
                    reason: "Group must have at least one network".to_string(),
                });
            }

            // Validate network references
            for network in &group.networks {
                if !self.network.contains_key(network) {
                    return Err(ConfigError::UnknownReference {
                        group: group_name.clone(),
                        kind: "network".to_string(),
                        value: network.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_references(&self) -> Result<(), ConfigError> {
        for (group_name, group) in &self.groups {
            // Validate network references
            if group.networks.is_empty() {
                return Err(ConfigError::InvalidGroup {
                    name: group_name.clone(),
                    reason: "Group must have at least one network".to_string(),
                });
            }

            for network in &group.networks {
                if !self.network.contains_key(network) {
                    return Err(ConfigError::UnknownReference {
                        group: group_name.clone(),
                        kind: "network".to_string(),
                        value: network.clone(),
                    });
                }
            }

            // Validate endpoints
            for endpoint_name in &group.endpoints {
                let endpoint = self.endpoints.get(endpoint_name).ok_or_else(|| {
                    ConfigError::UnknownReference {
                        group: group_name.clone(),
                        kind: "endpoint".to_string(),
                        value: endpoint_name.clone(),
                    }
                })?;

                validate_dicom_endpoint(endpoint)?;
            }

            // Validate backends
            for backend_name in &group.backends {
                let backend = self.backends.get(backend_name).ok_or_else(|| {
                    ConfigError::UnknownReference {
                        group: group_name.clone(),
                        kind: "backend".to_string(),
                        value: backend_name.clone(),
                    }
                })?;

                validate_dicom_backend(backend)?;
            }

            // Validate middleware references
            self.validate_middleware_references(group_name, &group.middleware.incoming, "incoming")?;
            self.validate_middleware_references(group_name, &group.middleware.outgoing, "outgoing")?;
        }
        Ok(())
    }


    fn validate_middleware_references(&self, group_name: &str, middleware_list: &[String], direction: &str) -> Result<(), ConfigError> {
        for middleware_name in middleware_list {
            match middleware_name.as_str() {
                "jwt_auth" => {
                    if self.middleware.jwt_auth.is_none() {
                        return Err(ConfigError::MissingMiddlewareConfig {
                            group: group_name.to_string(),
                            middleware: middleware_name.clone(),
                            direction: direction.to_string(),
                        });
                    }
                }
                "auth_sidecar" => {
                    if self.middleware.auth_sidecar.is_none() {
                        return Err(ConfigError::MissingMiddlewareConfig {
                            group: group_name.to_string(),
                            middleware: middleware_name.clone(),
                            direction: direction.to_string(),
                        });
                    }
                }
                "aurabox_connect" => {
                    if self.middleware.aurabox_connect.is_none() {
                        return Err(ConfigError::MissingMiddlewareConfig {
                            group: group_name.to_string(),
                            middleware: middleware_name.clone(),
                            direction: direction.to_string(),
                        });
                    }
                }
                _ => {
                    return Err(ConfigError::UnknownMiddleware {
                        group: group_name.to_string(),
                        middleware: middleware_name.clone(),
                        direction: direction.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidProxyID,
    MissingNetworks,
    MissingGroups,
    InvalidEndpoint { name: String, reason: String },
    InvalidBackend { name: String, reason: String },
    InvalidNetwork { name: String, reason: String },
    InvalidGroup { name: String, reason: String },
    UnknownReference { group: String, kind: String, value: String },
    UnknownMiddleware { group: String, middleware: String, direction: String },
    MissingMiddlewareConfig { group: String, middleware: String, direction: String },
    InvalidPeer(String),
    MissingPeers,
}

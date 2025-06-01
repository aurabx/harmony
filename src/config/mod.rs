use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::collections::{HashMap, HashSet};

#[derive(Parser, Debug)]
#[command(name = "harmony")]
#[command(about = "JSON DICOM Exchange Proxy", long_about = None)]
pub struct Cli {
    #[arg(short, long, default_value = "/etc/harmony/harmony-config.toml")]
    pub config: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub proxy: ProxyConfig,
    pub network: NetworkConfig,
    #[serde(default)]
    pub endpoints: HashMap<String, Endpoint>,
    #[serde(default)]
    pub internal_services: HashMap<String, InternalService>,
    #[serde(default)]
    pub transform_rules: HashMap<String, TransformRule>,
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
        // self.network.validate()?;

        // if self.endpoints.is_empty() {
        //     return Err(ConfigError::MissingEndpoints);
        // }
        // if self.internal_services.is_empty() {
        //     return Err(ConfigError::MissingInternalServices);
        // }

        let mut group_set = HashSet::new();

        for (name, endpoint) in &self.endpoints {
            if endpoint.group.trim().is_empty() {
                return Err(ConfigError::InvalidEndpointGroup(name.clone()));
            }
            group_set.insert(endpoint.group.clone());
        }

        for (name, service) in &self.internal_services {
            if service.group.trim().is_empty() {
                return Err(ConfigError::InvalidServiceGroup(name.clone()));
            }
            group_set.insert(service.group.clone());
        }

        for (rule_name, rule) in &self.transform_rules {
            if !group_set.contains(&rule.from_group) {
                return Err(ConfigError::UnknownGroup {
                    rule: rule_name.clone(),
                    kind: "from_group".to_string(),
                    value: rule.from_group.clone(),
                });
            }
            if !group_set.contains(&rule.to_group) {
                return Err(ConfigError::UnknownGroup {
                    rule: rule_name.clone(),
                    kind: "to_group".to_string(),
                    value: rule.to_group.clone(),
                });
            }
            if rule.transform_chain.is_empty() {
                return Err(ConfigError::EmptyTransformChain(rule_name.clone()));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ProxyConfig {
    pub id: String,
    pub log_level: String,
    pub store_dir: String,
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id.trim().is_empty() {
            return Err(ConfigError::InvalidProxyID);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct NetworkConfig {
    pub enable_wireguard: bool,
    pub interface: String,
    #[serde(default)]
    pub http: HttpConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub bind_address: String,
    pub bind_port: u16,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 3000,
        }
    }
}

// impl NetworkConfig {
//     pub fn validate(&self) -> Result<(), ConfigError> {
//         if self.enable_wireguard {
//             match &self.peers {
//                 Some(peers) if !peers.is_empty() => {
//                     for peer in peers {
//                         peer.validate()?;
//                     }
//                 }
//                 _ => return Err(ConfigError::MissingPeers),
//             }
//         }
//         Ok(())
//     }
// }

#[derive(Debug, Deserialize)]
pub struct PeerConfig {
    pub id: String,
    pub ip: String,
    pub public_key: String,
}

impl PeerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id.trim().is_empty() || self.ip.trim().is_empty() || self.public_key.trim().is_empty() {
            return Err(ConfigError::InvalidPeer(self.id.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub group: String,
    pub path_prefix: String,
    #[serde(default)]
    pub middleware: Option<Vec<String>>,
    #[serde(flatten)]
    pub kind: EndpointKind,
}


#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EndpointKind {
    Dicom {
        #[serde(default)]
        aet: Option<String>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
    },
    Fhir,
    Jdx,
    Basic,
    Deadletter
}


impl Default for EndpointKind {
    fn default() -> Self {
        EndpointKind::Deadletter {
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InternalService {
    pub group: String,
    #[serde(flatten,rename = "type")]
    pub kind: InternalServiceKind,
    #[serde(default)]
    pub middleware: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InternalServiceKind {
    Dicom {
        aet: String,
        host: String,
        port: u16,
    },
    Fhir {
        url: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct TransformRule {
    pub from_group: String,
    pub to_group: String,
    pub transform_chain: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct MiddlewareConfig {
    pub jwt_auth: Option<JwtAuthConfig>,
    pub auth_sidecar: Option<AuthSidecarConfig>,
    pub aurabox_connect: Option<AuraboxConnectConfig>,
}

#[derive(Debug, Deserialize)]
pub struct JwtAuthConfig {
    pub jwks_url: String,
    pub audience: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthSidecarConfig {
    pub token_path: String,
}

#[derive(Debug, Deserialize)]
pub struct AuraboxConnectConfig {
    pub enabled: bool,
    pub fallback_timeout_ms: u64,
}

#[derive(Debug, Deserialize, Default)]
pub struct LoggingConfig {
    pub log_to_file: bool,
    pub log_file_path: String,
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidProxyID,
    MissingEndpoints,
    MissingInternalServices,
    InvalidEndpointGroup(String),
    InvalidServiceGroup(String),
    MissingPeers,
    InvalidPeer(String),
    UnknownGroup { rule: String, kind: String, value: String },
    EmptyTransformChain(String),
}
use serde::Deserialize;

#[derive(Debug, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
pub struct NetworkConfig {
    #[serde(default = "default_enable_wireguard")]
    pub enable_wireguard: bool,
    #[serde(default = "default_interface")]
    pub interface: String,
    /// Optional TCP network bind settings - used by TCP-based protocol adapters (HTTP/1.x, DIMSE, etc.)
    /// Accepts both 'tcp_config' and 'http' (alias) in TOML for backward compatibility.
    /// When omitted, no TCP HTTP listener will be started for this network.
    #[serde(default, alias = "http")]
    pub tcp_config: Option<TcpConfig>,
    /// Optional HTTP/3 (QUIC) listener configuration.
    /// When present, an Http3Adapter may be started for this network.
    #[serde(default)]
    pub http3: Option<Http3Config>,
}

fn default_enable_wireguard() -> bool {
    false
}

fn default_interface() -> String {
    "wg0".to_string()
}

/// TCP network bind configuration
///
/// Can be configured as either `[network.name.tcp_config]` or `[network.name.http]` in TOML.
/// These are TCP network settings used by all TCP-based protocol adapters, not just HTTP.
#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct TcpConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default = "default_bind_port")]
    pub bind_port: u16,
}

fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

fn default_bind_port() -> u16 {
    8080
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            bind_port: default_bind_port(),
        }
    }
}

/// HTTP/3 (QUIC) network bind configuration
///
/// Configured under `[network.name.http3]` in TOML.
#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Http3Config {
    /// UDP bind address for HTTP/3 listener
    pub bind_address: String,
    /// UDP port for HTTP/3 listener
    pub bind_port: u16,
    /// Path to PEM-encoded certificate chain
    pub cert_path: String,
    /// Path to PEM-encoded private key
    pub key_path: String,
}

// #[derive(Debug, Deserialize)]
// pub struct PeerConfig {
//     pub id: String,
//     pub ip: String,
//     pub public_key: String,
// }
//
// impl PeerConfig {
//     pub fn validate(&self) -> Result<(), ConfigError> {
//         if self.id.trim().is_empty() || self.ip.trim().is_empty() || self.public_key.trim().is_empty() {
//             return Err(ConfigError::InvalidPeer(self.id.clone()));
//         }
//         Ok(())
//     }
// }

use serde::Deserialize;
use crate::config::config::ConfigError;

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct NetworkConfig {
    #[serde(default = "default_enable_wireguard")]
    pub enable_wireguard: bool,
    #[serde(default = "default_interface")]
    pub interface: String,
    #[serde(default)]
    pub http: HttpConfig,
}

fn default_enable_wireguard() -> bool {
    false
}

fn default_interface() -> String {
    "wg0".to_string()
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct HttpConfig {
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

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            bind_port: default_bind_port(),
        }
    }
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
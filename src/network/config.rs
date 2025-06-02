use serde::Deserialize;
use crate::config::ConfigError;

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
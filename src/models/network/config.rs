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
    /// Path to PEM-encoded certificate chain for TLS/HTTPS
    /// When both cert_path and key_path are set, HTTPS is enabled
    #[serde(default)]
    pub cert_path: Option<String>,
    /// Path to PEM-encoded private key for TLS/HTTPS
    /// When both cert_path and key_path are set, HTTPS is enabled
    #[serde(default)]
    pub key_path: Option<String>,
    /// Force HTTPS redirect when true
    /// Only applies when TLS is NOT configured (no cert/key paths)
    /// When true, returns HTTP 301 redirect to https:// URL for all requests
    #[serde(default)]
    pub force_https: bool,
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
            cert_path: None,
            key_path: None,
            force_https: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http3_config_deserializes_from_toml() {
        let toml = r#"
            [network.test]
            interface = "wg0"
            enable_wireguard = false

            [network.test.http3]
            bind_address = "127.0.0.1"
            bind_port = 4433
            cert_path = "./certs/test-cert.pem"
            key_path = "./certs/test-key.pem"
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            network: std::collections::HashMap<String, NetworkConfig>,
        }

        let wrapper: Wrapper = toml::from_str(toml).expect("valid http3 toml");
        let net = wrapper.network.get("test").expect("network.test present");
        let http3 = net.http3.as_ref().expect("http3 config present");
        assert_eq!(http3.bind_address, "127.0.0.1");
        assert_eq!(http3.bind_port, 4433);
        assert_eq!(http3.cert_path, "./certs/test-cert.pem");
        assert_eq!(http3.key_path, "./certs/test-key.pem");
    }

    #[test]
    fn tcp_config_with_tls_deserializes_from_toml() {
        let toml = r#"
            [network.test]
            interface = "wg0"
            enable_wireguard = false

            [network.test.http]
            bind_address = "0.0.0.0"
            bind_port = 443
            cert_path = "./certs/server-cert.pem"
            key_path = "./certs/server-key.pem"
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            network: std::collections::HashMap<String, NetworkConfig>,
        }

        let wrapper: Wrapper = toml::from_str(toml).expect("valid tcp/tls toml");
        let net = wrapper.network.get("test").expect("network.test present");
        let tcp = net.tcp_config.as_ref().expect("tcp config present");
        assert_eq!(tcp.bind_address, "0.0.0.0");
        assert_eq!(tcp.bind_port, 443);
        assert_eq!(tcp.cert_path, Some("./certs/server-cert.pem".to_string()));
        assert_eq!(tcp.key_path, Some("./certs/server-key.pem".to_string()));
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

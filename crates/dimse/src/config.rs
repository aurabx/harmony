//! Configuration types for DIMSE services

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

use crate::DEFAULT_DIMSE_PORT;

/// Configuration for DIMSE services
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimseConfig {
    /// Local Application Entity Title
    pub local_aet: String,
    
    /// Bind address for SCP listener
    #[serde(default = "default_bind_addr")]
    pub bind_addr: IpAddr,
    
    /// Port for SCP listener
    #[serde(default = "default_port")]
    pub port: u16,
    
    /// Maximum PDU size in bytes
    #[serde(default = "default_max_pdu")]
    pub max_pdu: u32,
    
    /// Connection timeout in milliseconds
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_ms: u64,
    
    /// Association timeout in milliseconds  
    #[serde(default = "default_association_timeout")]
    pub association_timeout_ms: u64,
    
    /// Storage directory for temporary DICOM files
    #[serde(default = "default_storage_dir")]
    pub storage_dir: PathBuf,
    
    /// TLS configuration (optional)
    pub tls: Option<TlsConfig>,
    
    /// Preferred transfer syntaxes (in order of preference)
    #[serde(default = "default_transfer_syntaxes")]
    pub preferred_transfer_syntaxes: Vec<String>,
    
    /// Maximum number of concurrent associations
    #[serde(default = "default_max_associations")]
    pub max_associations: u32,
    
    /// Enable C-ECHO service
    #[serde(default = "default_true")]
    pub enable_echo: bool,
    
    /// Enable C-FIND service
    #[serde(default = "default_true")]
    pub enable_find: bool,
    
    /// Enable C-MOVE service
    #[serde(default = "default_true")]
    pub enable_move: bool,
}

/// Configuration for a remote DICOM node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteNode {
    /// Remote Application Entity Title
    pub ae_title: String,
    
    /// Remote host address
    pub host: String,
    
    /// Remote port
    pub port: u16,
    
    /// Use TLS for this connection
    #[serde(default)]
    pub use_tls: bool,
    
    /// Connection timeout in milliseconds (overrides global setting)
    pub connect_timeout_ms: Option<u64>,
    
    /// Maximum PDU size for this node (overrides global setting)
    pub max_pdu: Option<u32>,
}

/// TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to certificate file (PEM format)
    pub cert_path: PathBuf,
    
    /// Path to private key file (PEM format)
    pub key_path: PathBuf,
    
    /// Path to CA bundle file (optional, for client certificate verification)
    pub ca_bundle_path: Option<PathBuf>,
    
    /// Require client certificates
    #[serde(default)]
    pub require_client_cert: bool,
}

impl Default for DimseConfig {
    fn default() -> Self {
        Self {
            local_aet: "HARMONY_SCP".to_string(),
            bind_addr: default_bind_addr(),
            port: default_port(),
            max_pdu: default_max_pdu(),
            connect_timeout_ms: default_connect_timeout(),
            association_timeout_ms: default_association_timeout(),
            storage_dir: default_storage_dir(),
            tls: None,
            preferred_transfer_syntaxes: default_transfer_syntaxes(),
            max_associations: default_max_associations(),
            enable_echo: true,
            enable_find: true,
            enable_move: true,
        }
    }
}

impl DimseConfig {
    /// Get connection timeout as Duration
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_timeout_ms)
    }
    
    /// Get association timeout as Duration
    pub fn association_timeout(&self) -> Duration {
        Duration::from_millis(self.association_timeout_ms)
    }
    
    /// Check if TLS is enabled
    pub fn tls_enabled(&self) -> bool {
        self.tls.is_some()
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> crate::error::Result<()> {
        // Validate AE title
        if self.local_aet.is_empty() || self.local_aet.len() > 16 {
            return Err(crate::error::DimseError::config(
                "Local AE title must be 1-16 characters"
            ));
        }
        
        // Validate port
        if self.port == 0 {
            return Err(crate::error::DimseError::config(
                "Port must be greater than 0"
            ));
        }
        
        // Validate PDU size
        if self.max_pdu < 16384 || self.max_pdu > 131072 {
            return Err(crate::error::DimseError::config(
                "Max PDU size must be between 16384 and 131072 bytes"
            ));
        }
        
        // Validate storage directory
        if !self.storage_dir.exists() {
            std::fs::create_dir_all(&self.storage_dir)
                .map_err(|e| crate::error::DimseError::config(
                    format!("Failed to create storage directory: {}", e)
                ))?;
        }
        
        Ok(())
    }
}

impl RemoteNode {
    /// Create a new remote node configuration
    pub fn new(ae_title: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            ae_title: ae_title.into(),
            host: host.into(),
            port,
            use_tls: false,
            connect_timeout_ms: None,
            max_pdu: None,
        }
    }
    
    /// Enable TLS for this node
    pub fn with_tls(mut self) -> Self {
        self.use_tls = true;
        self
    }
    
    /// Set connection timeout for this node
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.connect_timeout_ms = Some(timeout_ms);
        self
    }
    
    /// Validate the remote node configuration
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.ae_title.is_empty() || self.ae_title.len() > 16 {
            return Err(crate::error::DimseError::config(
                "Remote AE title must be 1-16 characters"
            ));
        }
        
        if self.host.is_empty() {
            return Err(crate::error::DimseError::config(
                "Remote host cannot be empty"
            ));
        }
        
        if self.port == 0 {
            return Err(crate::error::DimseError::config(
                "Remote port must be greater than 0"
            ));
        }
        
        Ok(())
    }
}

// Default value functions
fn default_bind_addr() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
}

fn default_port() -> u16 {
    DEFAULT_DIMSE_PORT
}

fn default_max_pdu() -> u32 {
    65536
}

fn default_connect_timeout() -> u64 {
    30_000 // 30 seconds
}

fn default_association_timeout() -> u64 {
    300_000 // 5 minutes
}

fn default_storage_dir() -> PathBuf {
    PathBuf::from("./tmp/dimse")
}

fn default_transfer_syntaxes() -> Vec<String> {
    vec![
        "1.2.840.10008.1.2".to_string(),      // Implicit VR Little Endian
        "1.2.840.10008.1.2.1".to_string(),    // Explicit VR Little Endian
        "1.2.840.10008.1.2.2".to_string(),    // Explicit VR Big Endian
    ]
}

fn default_max_associations() -> u32 {
    10
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DimseConfig::default();
        assert_eq!(config.local_aet, "HARMONY_SCP");
        assert_eq!(config.port, DEFAULT_DIMSE_PORT);
        assert!(config.enable_echo);
        assert!(config.enable_find);
        assert!(config.enable_move);
    }

    #[test]
    fn test_remote_node_builder() {
        let node = RemoteNode::new("TEST_AET", "localhost", 11112)
            .with_tls()
            .with_timeout(10_000);
            
        assert_eq!(node.ae_title, "TEST_AET");
        assert_eq!(node.host, "localhost");
        assert_eq!(node.port, 11112);
        assert!(node.use_tls);
        assert_eq!(node.connect_timeout_ms, Some(10_000));
    }

    #[test]
    fn test_config_validation() {
        let mut config = DimseConfig::default();
        assert!(config.validate().is_ok());
        
        // Test invalid AE title
        config.local_aet = "".to_string();
        assert!(config.validate().is_err());
        
        config.local_aet = "A".repeat(17);
        assert!(config.validate().is_err());
    }
}
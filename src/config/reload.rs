use super::config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Represents the differences between two configurations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiff {
    /// Changes that can be applied with zero downtime (atomic config swap)
    pub zero_downtime_changes: Vec<String>,

    /// Networks that need adapter restart
    pub adapter_restarts_required: Vec<String>,

    /// New networks to add
    pub networks_to_add: Vec<String>,

    /// Networks to remove
    pub networks_to_remove: Vec<String>,

    /// Sections that changed but are only read at startup. The new values
    /// are stored, but they take effect only after a process restart.
    #[serde(default)]
    pub restart_required_changes: Vec<String>,
}

impl ConfigDiff {
    /// Returns true if any changes require adapter restarts
    pub fn requires_adapter_restart(&self) -> bool {
        !self.adapter_restarts_required.is_empty()
            || !self.networks_to_add.is_empty()
            || !self.networks_to_remove.is_empty()
    }

    /// Returns true if there are any changes at all
    pub fn has_changes(&self) -> bool {
        !self.zero_downtime_changes.is_empty()
            || !self.restart_required_changes.is_empty()
            || self.requires_adapter_restart()
    }
}

/// Computes the difference between an old and new configuration
pub fn compute_diff(old: &Config, new: &Config) -> ConfigDiff {
    let mut diff = ConfigDiff {
        zero_downtime_changes: Vec::new(),
        adapter_restarts_required: Vec::new(),
        networks_to_add: Vec::new(),
        networks_to_remove: Vec::new(),
        restart_required_changes: Vec::new(),
    };

    // Check for network topology changes
    let old_networks: HashSet<_> = old.network.keys().collect();
    let new_networks: HashSet<_> = new.network.keys().collect();

    // Networks to add/remove
    for network in new_networks.difference(&old_networks) {
        diff.networks_to_add.push((*network).clone());
    }
    for network in old_networks.difference(&new_networks) {
        diff.networks_to_remove.push((*network).clone());
    }

    // Check for changes in existing networks
        for network_name in old_networks.intersection(&new_networks) {
        let old_net = &old.network[*network_name];
        let new_net = &new.network[*network_name];

        // Check if bind address or port changed (treat missing tcp_config as default)
        let old_tcp = old_net
            .tcp_config
            .clone()
            .unwrap_or_default();
        let new_tcp = new_net
            .tcp_config
            .clone()
            .unwrap_or_default();
        if old_tcp.bind_address != new_tcp.bind_address || old_tcp.bind_port != new_tcp.bind_port {
            diff.adapter_restarts_required.push((*network_name).clone());
            continue;
        }

        // Check WireGuard changes
        if old_net.enable_wireguard != new_net.enable_wireguard
            || old_net.interface != new_net.interface
        {
            diff.adapter_restarts_required.push((*network_name).clone());
            continue;
        }

        // For now, treat any other network config change as requiring restart
        // TODO: Add dimse field checking when NetworkConfig has dimse
        if old_net != new_net {
            diff.adapter_restarts_required.push((*network_name).clone());
            continue;
        }
    }

    // Check for middleware changes (zero-downtime)
    if old.middleware != new.middleware {
        diff.zero_downtime_changes.push("middleware".to_string());
    }

    // Check for pipeline changes (zero-downtime)
    if old.pipelines != new.pipelines {
        diff.zero_downtime_changes.push("pipelines".to_string());
    }

    // Check for endpoint changes (zero-downtime)
    if old.endpoints != new.endpoints {
        diff.zero_downtime_changes.push("endpoints".to_string());
    }

    // Check for backend changes (zero-downtime)
    if old.backends != new.backends {
        diff.zero_downtime_changes.push("backends".to_string());
    }

    // The logger is installed once by logging::init_logging.
    if old.logging != new.logging {
        diff.restart_required_changes.push("logging".to_string());
    }

    // The storage backend is created once at startup and held in globals.
    if old.storage != new.storage {
        diff.restart_required_changes.push("storage".to_string());
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::Config;

    #[test]
    fn test_no_changes() {
        let config = Config::default();
        let diff = compute_diff(&config, &config);

        assert!(!diff.has_changes());
        assert!(!diff.requires_adapter_restart());
    }

    /// The logger is installed once at startup, so a log_level change cannot
    /// be applied live and must not be reported as applied.
    #[test]
    fn test_logging_change_requires_restart() {
        let old_config = Config::default();
        let mut new_config = Config::default();
        new_config.logging.log_level = "debug".to_string();

        let diff = compute_diff(&old_config, &new_config);

        assert!(diff.has_changes());
        assert!(!diff.requires_adapter_restart());
        assert_eq!(diff.restart_required_changes, vec!["logging".to_string()]);
        assert!(diff.zero_downtime_changes.is_empty());
    }

    /// The storage backend is built once at startup, so a storage change
    /// cannot be applied live and must not be reported as applied.
    #[test]
    fn test_storage_change_requires_restart() {
        let old_config = Config::default();
        let mut new_config = Config::default();
        new_config.storage.backend = "something-else".to_string();

        let diff = compute_diff(&old_config, &new_config);

        assert!(diff.has_changes());
        assert_eq!(diff.restart_required_changes, vec!["storage".to_string()]);
        assert!(diff.zero_downtime_changes.is_empty());
    }

    /// Diffs serialised before restart_required_changes existed must still load.
    #[test]
    fn test_diff_without_restart_required_changes_deserialises() {
        let json = r#"{"zero_downtime_changes":[],"adapter_restarts_required":[],"networks_to_add":[],"networks_to_remove":[]}"#;
        let diff: ConfigDiff = serde_json::from_str(json).expect("old diff shape must parse");
        assert!(diff.restart_required_changes.is_empty());
    }

    #[test]
    fn test_middleware_change_only() {
        let old_config = Config::default();
        let mut new_config = Config::default();

        // Add middleware to new config
        new_config.middleware.insert(
            "test".to_string(),
            crate::models::middleware::instance::MiddlewareInstance {
                middleware_type: "test".to_string(),
                authentication: None,
                options: Default::default(),
            },
        );

        let diff = compute_diff(&old_config, &new_config);

        assert!(diff.has_changes());
        assert!(!diff.requires_adapter_restart());
        assert!(diff
            .zero_downtime_changes
            .contains(&"middleware".to_string()));
    }

    #[test]
    fn test_network_port_change() {
        use crate::models::network::config::{NetworkConfig, TcpConfig};

        let mut old_config = Config::default();
        let mut new_config = Config::default();

        // Add same network to both, but with different ports
        let mut old_net = NetworkConfig::default();
        old_net.tcp_config = Some(TcpConfig {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8080,
            cert_path: None,
            key_path: None,
            force_https: false,
        });
        old_config.network.insert("default".to_string(), old_net);

        let mut new_net = NetworkConfig::default();
        new_net.tcp_config = Some(TcpConfig {
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8081,
            cert_path: None,
            key_path: None,
            force_https: false,
        });
        new_config.network.insert("default".to_string(), new_net);

        let diff = compute_diff(&old_config, &new_config);

        assert!(diff.has_changes());
        assert!(diff.requires_adapter_restart());
        assert!(diff
            .adapter_restarts_required
            .contains(&"default".to_string()));
    }

    #[test]
    fn test_network_add_remove() {
        let mut old_config = Config::default();
        let mut new_config = Config::default();

        old_config
            .network
            .insert("old_network".to_string(), Default::default());

        new_config
            .network
            .insert("new_network".to_string(), Default::default());

        let diff = compute_diff(&old_config, &new_config);

        assert!(diff.has_changes());
        assert!(diff.requires_adapter_restart());
        assert!(diff.networks_to_add.contains(&"new_network".to_string()));
        assert!(diff.networks_to_remove.contains(&"old_network".to_string()));
    }
}

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
        !self.zero_downtime_changes.is_empty() || self.requires_adapter_restart()
    }
}

/// Computes the difference between an old and new configuration
pub fn compute_diff(old: &Config, new: &Config) -> ConfigDiff {
    let mut diff = ConfigDiff {
        zero_downtime_changes: Vec::new(),
        adapter_restarts_required: Vec::new(),
        networks_to_add: Vec::new(),
        networks_to_remove: Vec::new(),
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

        // Check if bind address or port changed
        if old_net.tcp_config.bind_address != new_net.tcp_config.bind_address
            || old_net.tcp_config.bind_port != new_net.tcp_config.bind_port
        {
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

    // Check for logging changes (zero-downtime)
    if old.logging != new.logging {
        diff.zero_downtime_changes.push("logging".to_string());
    }

    // Check for storage changes (zero-downtime)
    if old.storage != new.storage {
        diff.zero_downtime_changes.push("storage".to_string());
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

    #[test]
    fn test_middleware_change_only() {
        let old_config = Config::default();
        let mut new_config = Config::default();

        // Add middleware to new config
        new_config.middleware.insert(
            "test".to_string(),
            crate::models::middleware::instance::MiddlewareInstance {
                middleware_type: "test".to_string(),
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
        let mut old_config = Config::default();
        let mut new_config = Config::default();

        // Add same network to both, but with different ports
        let mut old_net = crate::models::network::config::NetworkConfig::default();
        old_net.tcp_config.bind_port = 8080;
        old_config.network.insert("default".to_string(), old_net);

        let mut new_net = crate::models::network::config::NetworkConfig::default();
        new_net.tcp_config.bind_port = 8081;
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

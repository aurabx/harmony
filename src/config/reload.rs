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

        // DIMSE listeners come from endpoint and backend options, which are
        // read only when the adapter starts. Restart when they change.
        if crate::adapters::dimse::listener_signature(old, network_name)
            != crate::adapters::dimse::listener_signature(new, network_name)
        {
            diff.adapter_restarts_required.push((*network_name).clone());
            continue;
        }
    }

    // Check for middleware changes (zero-downtime)
    if old.middleware != new.middleware {
        diff.zero_downtime_changes.push("middleware".to_string());
    }

    // Policies and rules are read by the policies middleware and the DIMSE
    // peer check on each request or association (zero-downtime)
    if old.policies != new.policies {
        diff.zero_downtime_changes.push("policies".to_string());
    }
    if old.rules != new.rules {
        diff.zero_downtime_changes.push("rules".to_string());
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

    /// A config with an HTTP pipeline and a DIMSE pipeline on `default`.
    ///
    /// `scp_options` is appended to the `dicom_scp` endpoint's options and
    /// `policy_ips` fills the `ip_allow` rule on the DIMSE pipeline.
    fn dimse_config(scp_options: &str, policy_ips: &str, http_prefix: &str) -> Config {
        let toml = format!(
            r#"
[network.default]
enable_wireguard = false
interface = "wg0"
[network.default.http]
bind_address = "127.0.0.1"
bind_port = 8080

[pipelines.api]
networks = ["default"]
endpoints = ["api"]

[pipelines.store]
networks = ["default"]
endpoints = ["store_scp"]
middleware = ["gate"]

[endpoints.api]
service = "http"
[endpoints.api.options]
path_prefix = "{http_prefix}"

[endpoints.store_scp]
service = "dicom_scp"
[endpoints.store_scp.options]
local_aet = "STORE_SCP"
port = 11112
{scp_options}

[middleware.gate]
type = "policies"
[middleware.gate.options]
policies = ["office"]

[policies.office]
id = "office"
rules = ["office_allow"]

[rules.office_allow]
id = "office_allow"
type = "ip_allow"
[rules.office_allow.options]
ip_addresses = [{policy_ips}]
"#
        );
        toml::from_str(&toml).expect("TOML parse error")
    }

    fn restarts_default(old: &Config, new: &Config) -> bool {
        compute_diff(old, new)
            .adapter_restarts_required
            .contains(&"default".to_string())
    }

    /// Turning DCMTK off must restart the network, or the running storescp
    /// keeps the port and the pipeline's IP rules never apply.
    #[test]
    fn dimse_listener_option_change_restarts_the_network() {
        let old = dimse_config("use_dcmtk_store = true", "", "/api");
        let new = dimse_config("use_dcmtk_store = false", "", "/api");
        assert!(restarts_default(&old, &new));
    }

    #[test]
    fn adding_a_dimse_endpoint_to_a_network_restarts_it() {
        let mut old = dimse_config("", "", "/api");
        old.pipelines.remove("store");
        let new = dimse_config("", "", "/api");
        assert!(restarts_default(&old, &new));
    }

    /// The internal SCP reads IP rules for each association, so a policy
    /// change needs no restart.
    #[test]
    fn dimse_policy_change_does_not_restart_the_network() {
        let old = dimse_config("", r#""10.0.0.0/8""#, "/api");
        let new = dimse_config("", r#""192.168.0.0/16""#, "/api");
        let diff = compute_diff(&old, &new);
        assert!(!diff.requires_adapter_restart(), "{:?}", diff);
        assert!(diff.zero_downtime_changes.contains(&"rules".to_string()));
    }

    #[test]
    fn http_endpoint_change_does_not_restart_a_dimse_network() {
        let old = dimse_config("", "", "/api");
        let new = dimse_config("", "", "/api/v2");
        let diff = compute_diff(&old, &new);
        assert!(!diff.requires_adapter_restart(), "{:?}", diff);
        assert!(diff
            .zero_downtime_changes
            .contains(&"endpoints".to_string()));
    }
}

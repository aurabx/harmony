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

    /// Tables that changed and are read, wholly or partly, at startup. The
    /// new values are swapped in and reach anything that reads the live
    /// config, but full effect needs a process restart.
    #[serde(default)]
    pub process_restart_required: Vec<String>,
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
            || self.requires_adapter_restart()
            || !self.process_restart_required.is_empty()
    }
}

/// Computes the difference between an old and new configuration
pub fn compute_diff(old: &Config, new: &Config) -> ConfigDiff {
    let mut diff = ConfigDiff {
        zero_downtime_changes: Vec::new(),
        adapter_restarts_required: Vec::new(),
        networks_to_add: Vec::new(),
        networks_to_remove: Vec::new(),
        process_restart_required: Vec::new(),
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

    classify_tables(old, new, &mut diff);

    diff
}

/// Sort every changed top-level table by how a reload applies it.
///
/// `Config` is destructured without `..`, so adding a field fails to compile
/// until it is placed here. A table left out of every list would make a
/// reload that changes only that table look like no change, and the watcher
/// would drop the reload.
fn classify_tables(old: &Config, new: &Config, diff: &mut ConfigDiff) {
    let Config {
        // Compared per network in `compute_diff`.
        network: _,
        // Read when used, so the config swap applies them.
        middleware,
        policies,
        rules,
        pipelines,
        endpoints,
        backends,
        authentications,
        // Merged into backends, endpoints and pipelines at load, so their
        // effect also shows in those tables.
        targets,
        peers,
        management,
        // Indexed into each HTTP router's mesh registry when it starts.
        mesh,
        ingress,
        egress,
        remote_ingress,
        // Read wholly or partly at startup: the log subscriber, the storage
        // backend, cloud polling, plugin setup, file paths, and service and
        // middleware type registries that can only be set once.
        logging,
        storage,
        provider,
        proxy,
        runbeam,
        services,
        middleware_types,
        extensions,
        // Derived from other fields, or not configuration.
        transforms: _,
        resolved_transforms_path: _,
        resolved_mesh_path: _,
        unresolved_backends: _,
        endpoint_sources: _,
    } = new;

    let zero_downtime = [
        ("middleware", *middleware != old.middleware),
        ("policies", *policies != old.policies),
        ("rules", *rules != old.rules),
        ("pipelines", *pipelines != old.pipelines),
        ("endpoints", *endpoints != old.endpoints),
        ("backends", *backends != old.backends),
        ("authentications", *authentications != old.authentications),
        ("targets", *targets != old.targets),
        ("peers", *peers != old.peers),
        ("management", *management != old.management),
    ];
    diff.zero_downtime_changes.extend(
        zero_downtime
            .iter()
            .filter(|(_, changed)| *changed)
            .map(|(name, _)| name.to_string()),
    );

    let mesh_changed = *mesh != old.mesh
        || *ingress != old.ingress
        || *egress != old.egress
        || *remote_ingress != old.remote_ingress;
    if mesh_changed {
        let mut kept: Vec<&String> = new
            .network
            .keys()
            .filter(|name| old.network.contains_key(*name))
            .filter(|name| !diff.adapter_restarts_required.contains(name))
            .collect();
        kept.sort();
        diff.adapter_restarts_required
            .extend(kept.into_iter().cloned());
    }

    let startup_only = [
        ("logging", *logging != old.logging),
        ("storage", *storage != old.storage),
        ("provider", *provider != old.provider),
        ("proxy", *proxy != old.proxy),
        ("runbeam", *runbeam != old.runbeam),
        ("services", *services != old.services),
        (
            "middleware_types",
            *middleware_types != old.middleware_types,
        ),
        ("extensions", *extensions != old.extensions),
    ];
    diff.process_restart_required.extend(
        startup_only
            .iter()
            .filter(|(_, changed)| *changed)
            .map(|(name, _)| name.to_string()),
    );
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

    /// Two networks, `a` and `b`, plus whatever `extra` adds.
    fn tables_config(extra: &str) -> Config {
        let toml = format!(
            r#"
[network.a]
enable_wireguard = false
interface = "wg0"

[network.b]
enable_wireguard = false
interface = "wg0"
{extra}
"#
        );
        toml::from_str(&toml).expect("TOML parse error")
    }

    /// A reload that changes only one of these tables used to be dropped as
    /// "no changes".
    #[test]
    fn tables_read_on_use_are_zero_downtime_changes() {
        let cases = [
            (
                "authentications",
                "[authentications.basic]\nid = \"basic\"\nmethod = \"basic\"",
            ),
            (
                "targets",
                "[targets.pacs]\nconnection = { host = \"pacs.example.test\" }",
            ),
            (
                "peers",
                "[peers.clinic]\nconnection = { host = \"clinic.example.test\" }",
            ),
            ("management", "[management]\nenabled = true"),
        ];
        let old = tables_config("");
        for (table, extra) in cases {
            let new = tables_config(extra);
            let diff = compute_diff(&old, &new);
            assert_eq!(
                diff.zero_downtime_changes,
                vec![table.to_string()],
                "table {}",
                table
            );
            assert!(!diff.requires_adapter_restart(), "table {}", table);
        }
    }

    /// The HTTP router indexes mesh ingress URLs when it starts, so a change
    /// to any mesh table restarts every network kept by the reload.
    #[test]
    fn mesh_table_changes_restart_every_kept_network() {
        let cases = [
            (
                "mesh",
                "[mesh.m]\ntype = \"http\"\nprovider = \"local\"\ningress = []\negress = []",
            ),
            (
                "ingress",
                "[ingress.in]\npipeline = \"p\"\ntype = \"http\"\nurls = [\"http://a.test\"]",
            ),
            ("egress", "[egress.out]\npipeline = \"p\"\ntype = \"http\""),
            (
                "remote_ingress",
                "[remote_ingress.far]\nurls = [\"http://far.test\"]",
            ),
        ];
        let old = tables_config("");
        for (table, extra) in cases {
            let new = tables_config(&format!(
                "[network.c]\nenable_wireguard = false\ninterface = \"wg0\"\n\n{}",
                extra
            ));
            let diff = compute_diff(&old, &new);
            assert_eq!(
                diff.adapter_restarts_required,
                vec!["a", "b"],
                "table {}",
                table
            );
            assert_eq!(diff.networks_to_add, vec!["c"], "table {}", table);
        }
    }

    /// Startup-only tables are reported so the watcher can warn, and the
    /// reload is still applied.
    #[test]
    fn startup_only_tables_require_a_process_restart() {
        let old = tables_config("");
        let new = tables_config(
            "[proxy]\nid = \"renamed\"\n\n[services.http]\nmodule = \"\"\n\n\
             [logging]\nlog_level = \"trace\"\n\n[provider.runbeam]\napi = \"https://example.test\"",
        );
        let diff = compute_diff(&old, &new);
        assert_eq!(
            diff.process_restart_required,
            vec!["logging", "provider", "proxy", "services"]
        );
        assert!(diff.has_changes());
        assert!(!diff.requires_adapter_restart());
    }
}

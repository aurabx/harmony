pub mod envelope_unwrapper;
pub mod query_provider;
mod status_mapper;

use crate::adapters::ProtocolAdapter;
use crate::config::config::Config;
use crate::models::protocol::Protocol;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use query_provider::DimseOp;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Registry of started DIMSE SCPs to prevent duplicate listeners.
/// Key format: "{bind_addr}:{port}"
static STARTED_SCP: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// DIMSE protocol adapter
///
/// Handles DICOM DIMSE protocol via SCP (Service Class Provider) listeners.
/// Multiple pipelines sharing the same `(bind_addr, port)` are multiplexed
/// onto a single TCP listener, with each DIMSE operation (C-FIND, C-STORE,
/// etc.) routed to the pipeline that registered for it.
pub struct DimseAdapter {
    /// Network name this adapter serves
    pub network_name: String,
}

/// Collected SCP endpoint configuration before grouping.
#[derive(Debug, PartialEq)]
pub(crate) struct ScpEndpointConfig {
    pub(crate) pipeline_name: String,
    pub(crate) endpoint_name: String,
    /// The legacy `dicom`/`dicom_scu` backend this listener comes from, when
    /// it is a persistent store SCP rather than a `dicom_scp` endpoint.
    pub(crate) backend_name: Option<String>,
    options: HashMap<String, serde_json::Value>,
}

impl ScpEndpointConfig {
    /// The config block that sets this listener's options, for messages.
    pub(crate) fn config_label(&self) -> String {
        match &self.backend_name {
            Some(backend) => format!("backend '{}'", backend),
            None => format!("endpoint '{}'", self.endpoint_name),
        }
    }

    /// The option that hands this endpoint's port to DCMTK `storescp`, if any.
    ///
    /// An explicit `use_dcmtk_store` wins. Without it, `persistent_store_scp
    /// = true` turns DCMTK on.
    pub(crate) fn dcmtk_option(&self) -> Option<&'static str> {
        match self
            .options
            .get("use_dcmtk_store")
            .and_then(|v| v.as_bool())
        {
            Some(true) => Some("use_dcmtk_store"),
            Some(false) => None,
            None => self
                .options
                .get("persistent_store_scp")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                .then_some("persistent_store_scp"),
        }
    }
}

/// The endpoints served by one DIMSE listener on `(bind_addr, port)`.
#[derive(Debug, PartialEq)]
pub(crate) struct ListenerGroup {
    pub(crate) bind_addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) endpoints: Vec<ScpEndpointConfig>,
}

impl ListenerGroup {
    /// The first endpoint that hands this listener to DCMTK `storescp`, with
    /// the option that does it. One such endpoint decides for the whole port.
    pub(crate) fn dcmtk_trigger(&self) -> Option<(&ScpEndpointConfig, &'static str)> {
        self.endpoints
            .iter()
            .find_map(|ep| ep.dcmtk_option().map(|option| (ep, option)))
    }
}

/// Collect the DIMSE listeners the adapter for `network_name` would start.
///
/// The adapter and config validation both use this, so they always agree on
/// which endpoints share a port and whether DCMTK serves it. Groups come out
/// in `(bind_addr, port)` order.
pub(crate) fn listener_groups(config: &Config, network_name: &str) -> Vec<ListenerGroup> {
    let network_config = config.network.get(network_name);
    let mut groups: BTreeMap<(IpAddr, u16), Vec<ScpEndpointConfig>> = BTreeMap::new();
    for ep in collect_scp_endpoints(config, network_name) {
        let addr = DimseAdapter::resolve_bind_addr(&ep.options, network_config);
        let port = DimseAdapter::resolve_port(&ep.options, network_config);
        groups.entry((addr, port)).or_default().push(ep);
    }
    groups
        .into_iter()
        .map(|((bind_addr, port), endpoints)| ListenerGroup {
            bind_addr,
            port,
            endpoints,
        })
        .collect()
}

/// Whether the adapter registry starts a DIMSE adapter for `network_name`.
///
/// Mirrors `AdapterRegistry::determine_required_protocols`: only a pipeline
/// endpoint whose service needs `Protocol::Dimse` starts the adapter. A
/// legacy persistent-SCP backend on its own does not, so it opens no
/// listener.
fn network_runs_dimse(config: &Config, network_name: &str) -> bool {
    config
        .pipelines
        .values()
        .filter(|p| p.networks.iter().any(|n| n == network_name))
        .flat_map(|p| p.endpoints.iter())
        .filter_map(|name| config.endpoints.get(name))
        .filter_map(|ep| crate::models::services::services::resolve_service(&ep.service).ok())
        .any(|service| service.required_protocol() == Protocol::Dimse)
}

/// The DIMSE listeners `network_name` runs under `config`, in a form that
/// compares equal when the adapter would start the same listeners.
///
/// Listener settings are read once when the adapter starts, so a reload must
/// restart the network whenever this changes. Otherwise a running storescp
/// keeps a port the new config hands to the internal SCP, which is the one
/// that enforces IP rules. Endpoints are sorted because their collection
/// order follows a `HashMap`.
pub(crate) fn listener_signature(config: &Config, network_name: &str) -> Vec<ListenerGroup> {
    if !network_runs_dimse(config, network_name) {
        return Vec::new();
    }
    let mut groups = listener_groups(config, network_name);
    for group in &mut groups {
        group.endpoints.sort_by(|a, b| {
            (&a.pipeline_name, &a.endpoint_name).cmp(&(&b.pipeline_name, &b.endpoint_name))
        });
    }
    groups
}

/// Every DIMSE listener the running proxy would open, across all networks
/// that start a DIMSE adapter, in network-name order.
pub(crate) fn all_listener_groups(config: &Config) -> Vec<ListenerGroup> {
    let mut network_names: Vec<&String> = config.network.keys().collect();
    network_names.sort();
    network_names
        .into_iter()
        .filter(|network| network_runs_dimse(config, network))
        .flat_map(|network| listener_groups(config, network))
        .collect()
}

/// Why the DCMTK listener `dcmtk` cannot run, if it cannot.
///
/// DCMTK `storescp` writes incoming files without running a pipeline or the
/// association-time IP check. It binds every interface, so it takes its port
/// for every listener on that port, in any network. A pipeline with
/// `ip_allow` or `ip_deny` rules on any of them would admit every peer. A
/// policies middleware that cannot be read counts as a conflict, so the check
/// fails closed. Returns `None` when `dcmtk` is not served by DCMTK.
pub(crate) fn dcmtk_ip_rule_conflict(config: &Config, dcmtk: &ListenerGroup) -> Option<String> {
    let (trigger, option) = dcmtk.dcmtk_trigger()?;
    let groups = all_listener_groups(config);
    let same_port = groups
        .iter()
        .filter(|g| g.port == dcmtk.port)
        .flat_map(|g| g.endpoints.iter());
    for ep in same_port {
        let problem = match query_provider::pipeline_has_ip_rules(config, &ep.pipeline_name) {
            Ok(false) => continue,
            Ok(true) => "has ip_allow or ip_deny rules".to_string(),
            Err(e) => format!("has a policies middleware that cannot be checked ({})", e),
        };
        return Some(format!(
            "pipeline '{}' ({}) {}, but DIMSE port {} is served by DCMTK storescp, \
             which cannot enforce IP rules. '{}' on {} turns DCMTK on. \
             Set use_dcmtk_store = false there, or remove the IP rules",
            ep.pipeline_name,
            ep.config_label(),
            problem,
            dcmtk.port,
            option,
            trigger.config_label()
        ));
    }
    None
}

/// Every `dicom_scp` endpoint and legacy persistent SCP backend on the
/// pipelines that use `network_name`.
fn collect_scp_endpoints(config: &Config, network_name: &str) -> Vec<ScpEndpointConfig> {
    let mut scp_configs: Vec<ScpEndpointConfig> = Vec::new();

    for (pipeline_name, pipeline_cfg) in &config.pipelines {
        if !pipeline_cfg.networks.iter().any(|n| n == network_name) {
            continue;
        }

        // Collect dicom_scp endpoints
        for endpoint_name in &pipeline_cfg.endpoints {
            if let Some(endpoint) = config.endpoints.get(endpoint_name) {
                if endpoint.service == "dicom_scp" {
                    scp_configs.push(ScpEndpointConfig {
                        pipeline_name: pipeline_name.clone(),
                        endpoint_name: endpoint_name.clone(),
                        backend_name: None,
                        options: endpoint.options.clone().unwrap_or_default(),
                    });
                }
            }
        }

        // Also check for persistent SCP backends (legacy support)
        for backend_name in &pipeline_cfg.backends {
            if let Some(backend) = config.backends.get(backend_name) {
                if backend.service == "dicom" || backend.service == "dicom_scu" {
                    if let Some(options) = &backend.options {
                        let needs_persistent = options
                            .get("persistent_store_scp")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            || (options.contains_key("host") && options.contains_key("port"));

                        if needs_persistent && options.contains_key("incoming_store_port") {
                            scp_configs.push(ScpEndpointConfig {
                                pipeline_name: pipeline_name.clone(),
                                endpoint_name: format!("persistent_{}", backend_name),
                                backend_name: Some(backend_name.clone()),
                                options: options.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    scp_configs
}

/// How long the DIMSE adapter waits for its SCP tasks after shutdown before
/// aborting them. Kept below the registry's adapter shutdown timeout so the
/// adapter cleans up its own tasks rather than being aborted itself.
const DIMSE_SCP_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Holds an SCP's listen address in the process-wide registry for as long as
/// its task lives. Releasing it on drop means an aborted task frees the
/// address too, so a restart does not skip the port as "already running".
struct ScpRegistration(String);

impl Drop for ScpRegistration {
    fn drop(&mut self) {
        DimseAdapter::unregister_scp(&self.0);
    }
}

/// The SCP tasks one DIMSE adapter started. They are separate tokio tasks, so
/// aborting the adapter task alone would leave them serving. Dropping this
/// aborts any still running.
struct ScpTasks(Vec<JoinHandle<()>>);

impl ScpTasks {
    /// Waits for every SCP task until DIMSE_SCP_STOP_TIMEOUT, then aborts the
    /// rest and waits for them to finish dropping.
    async fn stop(&mut self, network_name: &str) {
        let deadline = tokio::time::Instant::now() + DIMSE_SCP_STOP_TIMEOUT;
        for handle in &mut self.0 {
            if tokio::time::timeout_at(deadline, &mut *handle).await.is_err() {
                tracing::warn!(
                    "DIMSE SCP for network '{}' did not stop within {:?}; aborting it",
                    network_name,
                    DIMSE_SCP_STOP_TIMEOUT
                );
                handle.abort();
                // The JoinError here only reports the cancellation just requested.
                let _ = handle.await;
            }
        }
        self.0.clear();
    }
}

impl Drop for ScpTasks {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

impl DimseAdapter {
    /// Create a new DIMSE adapter for the given network
    pub fn new(network_name: impl Into<String>) -> Self {
        Self {
            network_name: network_name.into(),
        }
    }

    /// Register an SCP in the global registry
    fn register_scp(key: String) -> bool {
        let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
        if guard.contains(&key) {
            false
        } else {
            guard.insert(key);
            true
        }
    }

    /// Unregister an SCP from the global registry
    fn unregister_scp(key: &str) {
        let mut guard = STARTED_SCP.lock().expect("SCP registry poisoned");
        guard.retain(|k| k != key);
    }

    /// Resolve the bind address for an SCP endpoint, with fallback chain:
    /// endpoint options → network TCP config → 0.0.0.0
    fn resolve_bind_addr(
        options: &HashMap<String, serde_json::Value>,
        network_config: Option<&crate::models::network::config::NetworkConfig>,
    ) -> IpAddr {
        options
            .get("bind_addr")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .or_else(|| {
                network_config
                    .and_then(|nc| nc.tcp_config.as_ref())
                    .and_then(|tcp| tcp.bind_address.parse::<IpAddr>().ok())
            })
            .unwrap_or_else(|| IpAddr::from(std::net::Ipv4Addr::new(0, 0, 0, 0)))
    }

    /// Resolve the port for an SCP endpoint, with fallback chain:
    /// incoming_store_port → port → network TCP config → DEFAULT_DIMSE_PORT
    fn resolve_port(
        options: &HashMap<String, serde_json::Value>,
        network_config: Option<&crate::models::network::config::NetworkConfig>,
    ) -> u16 {
        options
            .get("incoming_store_port")
            .or_else(|| options.get("port"))
            .and_then(|v| v.as_u64())
            .map(|p| p as u16)
            .or_else(|| {
                network_config
                    .and_then(|nc| nc.tcp_config.as_ref())
                    .map(|tcp| tcp.bind_port)
            })
            .unwrap_or(dimse::DEFAULT_DIMSE_PORT)
    }

    /// Resolve the storage directory, with fallback chain:
    /// endpoint options → global storage adapter → ./tmp/dimse
    fn resolve_storage_dir(options: &HashMap<String, serde_json::Value>) -> std::path::PathBuf {
        if let Some(dir) = options.get("storage_dir").and_then(|v| v.as_str()) {
            std::path::PathBuf::from(dir)
        } else if let Some(storage) = crate::globals::get_storage() {
            storage
                .ensure_dir_str("dimse")
                .unwrap_or_else(|_| std::path::PathBuf::from("./tmp/dimse"))
        } else {
            std::path::PathBuf::from("./tmp/dimse")
        }
    }

    /// Extract the enabled DIMSE operations from endpoint options.
    fn enabled_ops(options: &HashMap<String, serde_json::Value>) -> Vec<DimseOp> {
        let mut ops = Vec::new();
        if options
            .get("enable_find")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            ops.push(DimseOp::Find);
        }
        if options
            .get("enable_move")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            ops.push(DimseOp::Move);
        }
        if options
            .get("enable_get")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            ops.push(DimseOp::Get);
        }
        if options
            .get("enable_store")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            ops.push(DimseOp::Store);
        }
        ops
    }

    /// Build a merged `DimseConfig` from multiple endpoints that share a
    /// `(bind_addr, port)`.
    ///
    /// The merged config takes the union of enabled operations and AET tables.
    /// The AET and other scalar settings are taken from the first endpoint
    /// (all endpoints on the same port should use the same AET).
    fn build_merged_config(
        bind_addr: IpAddr,
        port: u16,
        endpoints: &[&ScpEndpointConfig],
    ) -> dimse::DimseConfig {
        let first_options = &endpoints[0].options;

        let local_aet = first_options
            .get("local_aet")
            .and_then(|v| v.as_str())
            .unwrap_or("HARMONY_SCP")
            .to_string();

        let storage_dir = Self::resolve_storage_dir(first_options);

        let mut config = dimse::DimseConfig {
            local_aet: local_aet.clone(),
            bind_addr,
            port,
            storage_dir,
            // Start with everything disabled; we enable per-endpoint below
            enable_echo: false,
            enable_find: false,
            enable_move: false,
            enable_get: false,
            enable_store: false,
            ..Default::default()
        };

        let mut max_dataset_bytes = 0u64;

        // Merge settings from all endpoints sharing this listen address
        for ep in endpoints {
            let opts = &ep.options;

            // Union of enabled operations
            if opts
                .get("enable_echo")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                config.enable_echo = true;
            }
            if opts
                .get("enable_find")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                config.enable_find = true;
            }
            if opts
                .get("enable_move")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                config.enable_move = true;
            }
            if opts
                .get("enable_get")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                config.enable_get = true;
            }
            if opts
                .get("enable_store")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                config.enable_store = true;
            }

            // Max PDU: use the largest value across endpoints
            if let Some(max_pdu_val) = opts.get("max_pdu").and_then(|v| v.as_u64()) {
                if (16384..=131072).contains(&max_pdu_val) && max_pdu_val as u32 > config.max_pdu {
                    config.max_pdu = max_pdu_val as u32;
                }
            }

            // Largest instance: use the largest value across endpoints, as for
            // max_pdu. An endpoint that does not set the option counts with
            // the default, so the listener goes below the default only when
            // every endpoint on it asks for less. The dicom_scp endpoint
            // validation rejects a value that is not a positive integer
            // before the adapter starts.
            let endpoint_limit = opts
                .get("max_dataset_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(dimse::config::default_max_dataset_bytes);
            if max_dataset_bytes != 0 && endpoint_limit != max_dataset_bytes {
                tracing::warn!(
                    "Endpoint '{}' (pipeline '{}') has max_dataset_bytes {} but another endpoint on {}:{} \
                     has {}. The listener will use {}. An endpoint that does not set the option counts \
                     with the default of {}.",
                    ep.endpoint_name,
                    ep.pipeline_name,
                    endpoint_limit,
                    bind_addr,
                    port,
                    max_dataset_bytes,
                    max_dataset_bytes.max(endpoint_limit),
                    dimse::config::default_max_dataset_bytes(),
                );
            }
            max_dataset_bytes = max_dataset_bytes.max(endpoint_limit);

            // Warn if AETs differ across endpoints on the same port
            let ep_aet = opts
                .get("local_aet")
                .and_then(|v| v.as_str())
                .unwrap_or("HARMONY_SCP");
            if ep_aet != local_aet {
                tracing::warn!(
                    "Endpoint '{}' (pipeline '{}') has AET '{}' but port {}:{} is already \
                     using AET '{}'. The listener will use '{}'.",
                    ep.endpoint_name,
                    ep.pipeline_name,
                    ep_aet,
                    bind_addr,
                    port,
                    local_aet,
                    local_aet,
                );
            }

            // Merge AET tables
            if let Some(aet_table_val) = opts.get("aet_table") {
                if let Some(aet_obj) = aet_table_val.as_object() {
                    for (aet_name, node_val) in aet_obj {
                        if let Some(node_obj) = node_val.as_object() {
                            let ae_title = node_obj
                                .get("ae_title")
                                .and_then(|v| v.as_str())
                                .unwrap_or(aet_name)
                                .to_string();
                            let host = node_obj
                                .get("host")
                                .and_then(|v| v.as_str())
                                .unwrap_or("localhost")
                                .to_string();
                            let node_port = node_obj
                                .get("port")
                                .and_then(|v| v.as_u64())
                                .map(|p| p as u16)
                                .unwrap_or(11112);

                            config.aet_table.insert(
                                aet_name.clone(),
                                dimse::RemoteNode::new(ae_title, host, node_port),
                            );
                        }
                    }
                }
            }
        }

        config.max_dataset_bytes = max_dataset_bytes;
        config
    }
}

#[async_trait]
impl ProtocolAdapter for DimseAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Dimse
    }

    fn from_network(
        network_name: String,
        _network_config: &crate::models::network::config::NetworkConfig,
    ) -> Box<dyn ProtocolAdapter> {
        Box::new(DimseAdapter::new(network_name))
    }

    async fn start(
        &self,
        config: Arc<Config>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let network_name = self.network_name.clone();

        tracing::debug!("Starting DIMSE adapter for network '{}'", network_name);

        let groups = listener_groups(&config, &network_name);

        if groups.is_empty() {
            tracing::debug!(
                "No DIMSE endpoints or persistent SCPs found for network '{}'",
                network_name
            );
            return Ok(tokio::spawn(async move {
                shutdown.cancelled().await;
            }));
        }

        // Log which DICOM endpoints will be served
        let endpoint_names: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.endpoints.iter())
            .map(|c| c.endpoint_name.as_str())
            .collect();
        tracing::info!(
            "DIMSE adapter for network '{}' will serve {} endpoint(s): [{}]",
            network_name,
            endpoint_names.len(),
            endpoint_names.join(", ")
        );

        // Spawn task to manage all SCP listeners for this network
        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(async move {
            let mut scp_handles = ScpTasks(Vec::new());

            for group in &groups {
                let bind_addr = &group.bind_addr;
                let port = &group.port;
                let group_endpoints: Vec<&ScpEndpointConfig> = group.endpoints.iter().collect();

                let group_names: Vec<String> = group_endpoints
                    .iter()
                    .map(|ep| {
                        format!("{}:{}", ep.pipeline_name, ep.endpoint_name)
                    })
                    .collect();

                tracing::info!(
                    "Starting shared DIMSE SCP on {}:{} for {} endpoint(s): [{}]",
                    bind_addr,
                    port,
                    group_endpoints.len(),
                    group_names.join(", "),
                );

                // Build merged DimseConfig (union of enabled operations)
                let dimse_config =
                    Self::build_merged_config(*bind_addr, *port, &group_endpoints);

                // Build operation→pipeline routing table
                let routes: Vec<(String, String, Vec<DimseOp>)> = group_endpoints
                    .iter()
                    .map(|ep| {
                        let ops = Self::enabled_ops(&ep.options);
                        (ep.pipeline_name.clone(), ep.endpoint_name.clone(), ops)
                    })
                    .collect();

                let use_dcmtk = group.dcmtk_trigger().is_some();

                // storescp runs no pipeline and no admission check, so it
                // cannot enforce IP rules. Config validation rejects this.
                // Refusing here keeps the port closed for a config that
                // changed after validation, such as one a plugin edited.
                if use_dcmtk {
                    if let Some(reason) = dcmtk_ip_rule_conflict(&config, group) {
                        tracing::error!(
                            "Refusing to start DCMTK storescp on {}:{}: {}",
                            bind_addr,
                            port,
                            reason
                        );
                        continue;
                    }
                }

                let key = format!("{}:{}", bind_addr, port);
                if !Self::register_scp(key.clone()) {
                    tracing::info!(
                        "DIMSE SCP at {}:{} already running, skipping",
                        bind_addr,
                        port,
                    );
                    continue;
                }

                let local_aet = dimse_config.local_aet.clone();

                let result = if use_dcmtk {
                    Self::start_dcmtk_scp(
                        key,
                        local_aet,
                        *port,
                        dimse_config,
                        routes,
                        shutdown_clone.clone(),
                    )
                    .await
                } else {
                    Self::start_internal_scp(
                        key,
                        local_aet,
                        *bind_addr,
                        *port,
                        dimse_config,
                        routes,
                        shutdown_clone.clone(),
                    )
                    .await
                };

                match result {
                    Ok(h) => scp_handles.0.push(h),
                    Err(e) => {
                        tracing::error!(
                            "Failed to start DIMSE SCP on {}:{}: {}",
                            bind_addr,
                            port,
                            e
                        );
                    }
                }
            }

            // Wait for shutdown signal
            shutdown_clone.cancelled().await;
            tracing::info!("DIMSE adapter for network '{}' shutting down", network_name);

            scp_handles.stop(&network_name).await;

            tracing::info!("DIMSE adapter for network '{}' shut down", network_name);
        });

        Ok(handle)
    }

    fn summary(&self) -> String {
        format!("DimseAdapter for network '{}'", self.network_name)
    }
}

/// Wait for `child` to exit, or kill it when `shutdown` fires.
///
/// storescp knows nothing of Harmony's shutdown. Without this it would
/// outlive its adapter and keep the port, admitting every peer under a
/// config that no longer asks for DCMTK. Returns only once the process has
/// exited and been reaped, so the port is free when the caller unregisters
/// it and a restarted adapter can bind it.
async fn supervise_child(
    mut child: tokio::process::Child,
    shutdown: CancellationToken,
    name: &str,
) {
    tokio::select! {
        status = child.wait() => match status {
            Ok(status) => tracing::info!("{} exited: {}", name, status),
            Err(e) => tracing::error!("{} exited with error: {}", name, e),
        },
        _ = shutdown.cancelled() => match child.kill().await {
            Ok(()) => tracing::info!("{} stopped on shutdown", name),
            Err(e) => tracing::error!("Failed to stop {} on shutdown: {}", name, e),
        },
    }
}

/// Build a multiplexing query provider from the given operation routes.
fn build_provider(
    routes: Vec<(String, String, Vec<DimseOp>)>,
) -> Arc<dyn dimse::scp::QueryProvider> {
    if routes.len() == 1 && routes[0].2.is_empty() {
        // Single endpoint with no explicit ops -- use a direct provider
        // (backward compat: the pipeline handles everything)
        Arc::new(query_provider::PipelineQueryProvider::new(
            routes[0].0.clone(),
            routes[0].1.clone(),
        ))
    } else {
        Arc::new(query_provider::MultiplexingQueryProvider::new(routes))
    }
}

impl DimseAdapter {
    /// Start DCMTK storescp process
    async fn start_dcmtk_scp(
        key: String,
        local_aet: String,
        port: u16,
        dimse_config: dimse::DimseConfig,
        routes: Vec<(String, String, Vec<DimseOp>)>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        use tokio::process::Command;

        let storage_dir = dimse_config.storage_dir.clone();
        let bind_addr = dimse_config.bind_addr;

        let handle = tokio::spawn(async move {
            let _registration = ScpRegistration(key);
            let _ = tokio::fs::create_dir_all(&storage_dir).await;

            // Try to start DCMTK storescp
            let mut cmd = Command::new("storescp");
            cmd.arg("-v")
                .arg("-od")
                .arg(storage_dir.to_string_lossy().to_string())
                .arg("-aet")
                .arg(local_aet.clone())
                .arg(port.to_string());

            tracing::info!(
                "Starting DCMTK storescp AET='{}' on :{} -> {}",
                local_aet,
                port,
                storage_dir.display()
            );

            // Backstop: if this task is aborted rather than shut down, the
            // dropped Child still takes storescp with it.
            cmd.kill_on_drop(true);

            match cmd.spawn() {
                Ok(child) => supervise_child(child, shutdown, "storescp").await,
                Err(e) => {
                    tracing::error!(
                        "Failed to spawn storescp: {} -- falling back to internal SCP",
                        e
                    );
                    let provider = build_provider(routes);
                    let scp = dimse::DimseScp::new(dimse_config, provider);
                    if let Err(e2) = scp.run(shutdown).await {
                        tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e2);
                    } else {
                        tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
                    }
                }
            }
        });

        // Readiness loop (best-effort): try to connect to the listening port
        Self::wait_for_scp_ready(bind_addr, port).await;

        Ok(handle)
    }

    /// Start internal DIMSE SCP with a (possibly multiplexed) query provider
    async fn start_internal_scp(
        key: String,
        local_aet: String,
        bind_addr: IpAddr,
        port: u16,
        dimse_config: dimse::DimseConfig,
        routes: Vec<(String, String, Vec<DimseOp>)>,
        shutdown: CancellationToken,
    ) -> anyhow::Result<JoinHandle<()>> {
        let handle = tokio::spawn(async move {
            let _registration = ScpRegistration(key);
            let provider = build_provider(routes);
            let scp = dimse::DimseScp::new(dimse_config, provider);

            tracing::info!(
                "Starting internal DIMSE SCP AET='{}' on {}:{}",
                local_aet,
                bind_addr,
                port
            );

            if let Err(e) = scp.run(shutdown).await {
                tracing::error!("DIMSE SCP '{}' failed: {}", local_aet, e);
            } else {
                tracing::info!("DIMSE SCP '{}' stopped gracefully", local_aet);
            }
        });

        // Readiness loop
        Self::wait_for_scp_ready(bind_addr, port).await;

        Ok(handle)
    }

    /// Wait for SCP to be ready by attempting TCP connection
    async fn wait_for_scp_ready(bind_addr: IpAddr, port: u16) {
        let target = if bind_addr.is_unspecified() {
            format!("127.0.0.1:{}", port)
        } else {
            format!("{}:{}", bind_addr, port)
        };

        for _ in 0..40 {
            if std::net::TcpStream::connect(&target).is_ok() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::Config;

    #[test]
    fn test_dimse_adapter_creation() {
        let adapter = DimseAdapter::new("test_network");
        assert_eq!(adapter.network_name, "test_network");
        assert_eq!(adapter.protocol(), Protocol::Dimse);
    }

    #[test]
    fn test_dimse_adapter_summary() {
        let adapter = DimseAdapter::new("dimse_net");
        assert_eq!(adapter.summary(), "DimseAdapter for network 'dimse_net'");
    }

    #[tokio::test]
    async fn test_dimse_adapter_lifecycle() {
        let adapter = DimseAdapter::new("test_network");
        let config = Arc::new(Config::default());
        let shutdown = CancellationToken::new();

        // Start adapter (should complete without error even with empty config)
        let handle = adapter.start(config, shutdown.clone()).await.unwrap();

        // Trigger shutdown
        shutdown.cancel();

        // Wait for adapter to shut down
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "Adapter should shut down gracefully");
    }

    #[test]
    fn test_dimse_adapter_is_protocol_adapter() {
        // Ensure DimseAdapter can be used as trait object
        let adapter: Box<dyn ProtocolAdapter> = Box::new(DimseAdapter::new("test"));
        assert_eq!(adapter.protocol(), Protocol::Dimse);
    }

    #[test]
    fn test_scp_registry() {
        // Clear registry for test isolation
        {
            let mut guard = STARTED_SCP.lock().unwrap();
            guard.clear();
        }

        let key1 = "0.0.0.0:11112".to_string();
        let key2 = "0.0.0.0:11113".to_string();

        // First registration should succeed
        assert!(DimseAdapter::register_scp(key1.clone()));

        // Duplicate registration should fail
        assert!(!DimseAdapter::register_scp(key1.clone()));

        // Different key should succeed
        assert!(DimseAdapter::register_scp(key2.clone()));

        // Unregister and re-register should succeed
        DimseAdapter::unregister_scp(&key1);
        assert!(DimseAdapter::register_scp(key1.clone()));

        // Cleanup
        DimseAdapter::unregister_scp(&key1);
        DimseAdapter::unregister_scp(&key2);
    }

    /// An SCP task that is aborted, as the registry does after its shutdown
    /// timeout, must still release its address. Otherwise the restarted
    /// adapter skips the port as "already running" and nothing serves it.
    #[tokio::test]
    async fn aborted_scp_task_releases_its_registration() {
        let key = "127.0.0.1:59871".to_string();
        assert!(DimseAdapter::register_scp(key.clone()));

        let task = tokio::spawn({
            let key = key.clone();
            async move {
                let _registration = ScpRegistration(key);
                std::future::pending::<()>().await;
            }
        });
        tokio::task::yield_now().await;

        task.abort();
        // The JoinError only reports the requested cancellation.
        let _ = task.await;

        assert!(
            DimseAdapter::register_scp(key.clone()),
            "aborted SCP task left its address registered"
        );
        DimseAdapter::unregister_scp(&key);
    }

    /// Dropping the adapter's SCP task list, as happens when the registry
    /// aborts the adapter task, must abort the SCP tasks it started.
    #[tokio::test]
    async fn dropping_scp_tasks_aborts_them() {
        let key = "127.0.0.1:59872".to_string();
        assert!(DimseAdapter::register_scp(key.clone()));
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel::<()>();

        let tasks = ScpTasks(vec![tokio::spawn({
            let key = key.clone();
            async move {
                let _registration = ScpRegistration(key);
                let _signal_on_drop = dropped_tx;
                std::future::pending::<()>().await;
            }
        })]);
        tokio::task::yield_now().await;
        drop(tasks);

        // The sender is dropped when the aborted task's future is dropped.
        tokio::time::timeout(std::time::Duration::from_secs(2), dropped_rx)
            .await
            .expect("SCP task kept running after its ScpTasks was dropped")
            .unwrap_err();
        assert!(DimseAdapter::register_scp(key.clone()));
        DimseAdapter::unregister_scp(&key);
    }

    #[test]
    fn test_enabled_ops_extraction() {
        let mut opts = HashMap::new();
        opts.insert(
            "enable_find".to_string(),
            serde_json::Value::Bool(true),
        );
        opts.insert(
            "enable_store".to_string(),
            serde_json::Value::Bool(true),
        );
        opts.insert(
            "enable_get".to_string(),
            serde_json::Value::Bool(false),
        );

        let ops = DimseAdapter::enabled_ops(&opts);
        assert!(ops.contains(&DimseOp::Find));
        assert!(ops.contains(&DimseOp::Store));
        assert!(!ops.contains(&DimseOp::Get));
        assert!(!ops.contains(&DimseOp::Move));
    }

    #[test]
    fn test_resolve_bind_addr_defaults() {
        let opts = HashMap::new();
        let addr = DimseAdapter::resolve_bind_addr(&opts, None);
        assert_eq!(addr, IpAddr::from(std::net::Ipv4Addr::new(0, 0, 0, 0)));
    }

    #[test]
    fn test_resolve_bind_addr_from_options() {
        let mut opts = HashMap::new();
        opts.insert(
            "bind_addr".to_string(),
            serde_json::Value::String("127.0.0.1".to_string()),
        );
        let addr = DimseAdapter::resolve_bind_addr(&opts, None);
        assert_eq!(addr, IpAddr::from(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn test_resolve_port_defaults() {
        let opts = HashMap::new();
        let port = DimseAdapter::resolve_port(&opts, None);
        assert_eq!(port, dimse::DEFAULT_DIMSE_PORT);
    }

    #[test]
    fn test_resolve_port_from_options() {
        let mut opts = HashMap::new();
        opts.insert(
            "port".to_string(),
            serde_json::Value::Number(serde_json::Number::from(4242u16)),
        );
        let port = DimseAdapter::resolve_port(&opts, None);
        assert_eq!(port, 4242);
    }

    #[test]
    fn test_build_merged_config_single_endpoint() {
        let ep = ScpEndpointConfig {
            pipeline_name: "p1".into(),
            endpoint_name: "e1".into(),
            backend_name: None,
            options: {
                let mut m = HashMap::new();
                m.insert("local_aet".into(), serde_json::json!("MY_SCP"));
                m.insert("enable_find".into(), serde_json::json!(true));
                m.insert("enable_echo".into(), serde_json::json!(true));
                m
            },
        };

        let addr = IpAddr::from(std::net::Ipv4Addr::LOCALHOST);
        let cfg = DimseAdapter::build_merged_config(addr, 11112, &[&ep]);

        assert_eq!(cfg.local_aet, "MY_SCP");
        assert!(cfg.enable_find);
        assert!(cfg.enable_echo);
        assert!(!cfg.enable_store);
        assert!(!cfg.enable_get);
    }

    #[test]
    fn test_build_merged_config_union_of_ops() {
        let ep1 = ScpEndpointConfig {
            pipeline_name: "p1".into(),
            endpoint_name: "e1".into(),
            backend_name: None,
            options: {
                let mut m = HashMap::new();
                m.insert("local_aet".into(), serde_json::json!("SCP"));
                m.insert("enable_find".into(), serde_json::json!(true));
                m.insert("enable_echo".into(), serde_json::json!(true));
                m
            },
        };
        let ep2 = ScpEndpointConfig {
            pipeline_name: "p2".into(),
            endpoint_name: "e2".into(),
            backend_name: None,
            options: {
                let mut m = HashMap::new();
                m.insert("local_aet".into(), serde_json::json!("SCP"));
                m.insert("enable_store".into(), serde_json::json!(true));
                m
            },
        };

        let addr = IpAddr::from(std::net::Ipv4Addr::LOCALHOST);
        let cfg = DimseAdapter::build_merged_config(addr, 11112, &[&ep1, &ep2]);

        assert!(cfg.enable_find, "find from ep1");
        assert!(cfg.enable_echo, "echo from ep1");
        assert!(cfg.enable_store, "store from ep2");
        assert!(!cfg.enable_get, "get not enabled by either");
    }

    /// Whether a process with `pid` still exists.
    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// A storescp left running after shutdown would keep its port and admit
    /// every peer, so shutdown must kill and reap it.
    #[cfg(unix)]
    #[tokio::test]
    async fn supervise_child_kills_the_process_on_shutdown() {
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id().expect("child pid");
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(supervise_child(child, shutdown.clone(), "sleep"));

        shutdown.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("supervise_child should return promptly after shutdown")
            .expect("task should not panic");

        assert!(!process_exists(pid), "process {} survived shutdown", pid);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn supervise_child_returns_when_the_process_exits() {
        let child = tokio::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            supervise_child(child, CancellationToken::new(), "true"),
        )
        .await
        .expect("supervise_child should return when the process exits");
    }

    fn endpoint_with_options(options: serde_json::Value) -> ScpEndpointConfig {
        ScpEndpointConfig {
            pipeline_name: "p".into(),
            endpoint_name: "e".into(),
            backend_name: None,
            options: serde_json::from_value(options).expect("options object"),
        }
    }

    #[test]
    fn dcmtk_option_follows_use_dcmtk_store_then_persistent_store_scp() {
        let cases = [
            (serde_json::json!({}), None),
            (
                serde_json::json!({"use_dcmtk_store": true}),
                Some("use_dcmtk_store"),
            ),
            (
                serde_json::json!({"persistent_store_scp": true}),
                Some("persistent_store_scp"),
            ),
            (
                serde_json::json!({"persistent_store_scp": true, "use_dcmtk_store": false}),
                None,
            ),
            (
                serde_json::json!({"persistent_store_scp": false, "use_dcmtk_store": true}),
                Some("use_dcmtk_store"),
            ),
            // A non-bool use_dcmtk_store is ignored, as the adapter always did.
            (
                serde_json::json!({"persistent_store_scp": true, "use_dcmtk_store": "no"}),
                Some("persistent_store_scp"),
            ),
        ];
        for (options, expected) in cases {
            let ep = endpoint_with_options(options.clone());
            assert_eq!(ep.dcmtk_option(), expected, "options: {}", options);
        }
    }

    fn endpoint_with_limit(name: &str, limit: Option<u64>) -> ScpEndpointConfig {
        let mut options = HashMap::new();
        options.insert("local_aet".into(), serde_json::json!("SCP"));
        if let Some(limit) = limit {
            options.insert("max_dataset_bytes".into(), serde_json::json!(limit));
        }
        ScpEndpointConfig {
            pipeline_name: name.into(),
            endpoint_name: name.into(),
            backend_name: None,
            options,
        }
    }

    fn merged_limit(endpoints: &[&ScpEndpointConfig]) -> u64 {
        let addr = IpAddr::from(std::net::Ipv4Addr::LOCALHOST);
        DimseAdapter::build_merged_config(addr, 11112, endpoints).max_dataset_bytes
    }

    #[test]
    fn test_build_merged_config_max_dataset_bytes() {
        let default = dimse::config::default_max_dataset_bytes();
        let unset = endpoint_with_limit("unset", None);
        let small = endpoint_with_limit("small", Some(1_000_000));
        let smaller = endpoint_with_limit("smaller", Some(500_000));
        let large = endpoint_with_limit("large", Some(default * 2));

        assert_eq!(merged_limit(&[&unset]), default, "the default applies when unset");
        assert_eq!(merged_limit(&[&small]), 1_000_000, "one endpoint can go below the default");
        assert_eq!(merged_limit(&[&small, &smaller]), 1_000_000, "the largest value wins");
        assert_eq!(
            merged_limit(&[&small, &unset]),
            default,
            "an endpoint that does not set the option counts with the default"
        );
        assert_eq!(merged_limit(&[&unset, &large]), default * 2);
    }
}

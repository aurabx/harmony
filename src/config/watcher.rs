use super::config::Config;
use super::reload::{compute_diff, ConfigDiff};
use crate::adapters::registry::AdapterRegistry;
use crate::globals;
use anyhow::Result;
use async_trait::async_trait;
use notify::{Event, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

/// Config file watcher with debouncing
pub struct ConfigWatcher {
    config_path: String,
    pipelines_path: Option<String>,
    registry: Arc<AdapterRegistry>,
    debounce_duration: Duration,
    /// Networks whose stop, restart or start failed on an earlier reload.
    /// They are retried on the next reload, because the new config is
    /// already active and would otherwise show no change for them.
    failed_networks: tokio::sync::Mutex<HashSet<String>>,
}

impl ConfigWatcher {
    pub fn new(
        config_path: String,
        pipelines_path: Option<String>,
        registry: Arc<AdapterRegistry>,
    ) -> Self {
        Self {
            config_path,
            pipelines_path,
            registry,
            debounce_duration: Duration::from_millis(200),
            failed_networks: tokio::sync::Mutex::new(HashSet::new()),
        }
    }

    /// Start watching the config file and pipelines directory for changes
    pub async fn start(self) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(100);

        let config_path = self.config_path.clone();
        let config_path_clone = config_path.clone();
        let pipelines_path = self.pipelines_path.clone();

        // Start file watcher
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() {
                    let _ = tx.blocking_send(());
                }
            }
        })?;

        // Watch main config file
        watcher.watch(Path::new(&config_path), RecursiveMode::NonRecursive)?;
        tracing::info!("📡 Watching config file for changes: {}", config_path_clone);

        // Watch pipelines directory if provided
        if let Some(ref path) = pipelines_path {
            let pipelines_dir = Path::new(path);
            if pipelines_dir.exists() {
                watcher.watch(pipelines_dir, RecursiveMode::Recursive)?;
                tracing::info!("📡 Watching pipelines directory for changes: {}", path);
            }
        }

        // Debounce and handle reload
        // Initialize to a time far in the past so first reload isn't blocked by cooldown
        let mut last_reload = tokio::time::Instant::now()
            .checked_sub(Duration::from_secs(2))
            .unwrap_or_else(|| tokio::time::Instant::now());

        while rx.recv().await.is_some() {
            // Debounce: wait for stable file state
            sleep(self.debounce_duration).await;

            // Avoid reloading too frequently
            if last_reload.elapsed() < Duration::from_secs(1) {
                continue;
            }

            match self.reload_config().await {
                Ok(diff) => {
                    if diff.has_changes() {
                        tracing::info!("✓ Config reloaded successfully");
                        if diff.requires_adapter_restart() {
                            tracing::info!(
                                "  Networks restarted: {:?}",
                                diff.adapter_restarts_required
                            );
                            tracing::info!("  Networks added: {:?}", diff.networks_to_add);
                            tracing::info!("  Networks removed: {:?}", diff.networks_to_remove);
                        }
                        if !diff.zero_downtime_changes.is_empty() {
                            tracing::info!(
                                "  Zero-downtime changes: {:?}",
                                diff.zero_downtime_changes
                            );
                        }
                    }
                    last_reload = tokio::time::Instant::now();
                }
                Err(e) => {
                    tracing::error!("❌ Config reload failed: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Reload configuration and apply changes
    async fn reload_config(&self) -> Result<ConfigDiff> {
        // Load new config using safe loader that returns Result instead of exiting
        let new_config = match Config::load(&self.config_path) {
            Ok(config) => config,
            Err(e) => {
                let msg = match e {
                    crate::config::config::ConfigError::InvalidProxy { reason, .. } => reason,
                    crate::config::config::ConfigError::InvalidNetwork { name, reason } => format!("Invalid network '{}': {}", name, reason),
                    crate::config::config::ConfigError::InvalidPipeline { name, reason } => format!("Invalid pipeline '{}': {}", name, reason),
                    _ => format!("{:?}", e),
                };
                return Err(anyhow::anyhow!("Config validation failed: {}", msg));
            }
        };

        // Update provider resolver with new provider config
        if let Some(resolver) = globals::get_provider_resolver() {
            resolver.update_providers(new_config.provider.clone()).await;
        }

        // Get current config
        let old_config =
            globals::get_config().ok_or_else(|| anyhow::anyhow!("No config currently loaded"))?;

        // Compute diff
        let diff = compute_diff(&old_config, &new_config);

        let mut failed_networks = self.failed_networks.lock().await;
        if !diff.has_changes() && failed_networks.is_empty() {
            return Ok(diff);
        }

        if !diff.process_restart_required.is_empty() {
            tracing::warn!(
                "Changes to {:?} are only partly applied; restart the process for full effect",
                diff.process_restart_required
            );
        }

        apply_reload(
            self.registry.as_ref(),
            &diff,
            Arc::new(new_config),
            &mut failed_networks,
        )
        .await?;

        Ok(diff)
    }
}

/// The adapter operations a reload performs.
///
/// `AdapterRegistry` is the real implementation. Tests substitute a recorder
/// to check what `apply_reload` does and in what order.
#[async_trait]
pub(crate) trait NetworkControl: Send + Sync {
    async fn stop_network(&self, network: &str) -> Result<()>;
    async fn restart_network(&self, network: String, config: Arc<Config>) -> Result<()>;
    async fn start_network(&self, network: String, config: Arc<Config>) -> Result<()>;
}

#[async_trait]
impl NetworkControl for AdapterRegistry {
    async fn stop_network(&self, network: &str) -> Result<()> {
        AdapterRegistry::stop_network(self, network).await
    }

    async fn restart_network(&self, network: String, config: Arc<Config>) -> Result<()> {
        AdapterRegistry::restart_network(self, network, config).await
    }

    async fn start_network(&self, network: String, config: Arc<Config>) -> Result<()> {
        AdapterRegistry::start_network(self, network, config).await
    }
}

/// Publish `new_config` and bring the adapters in line with `diff`.
///
/// The config is swapped first. A restarted adapter, and anything that reads
/// the global config such as the DIMSE peer check, then never runs under the
/// old config. Every network is attempted even when one fails, so one bad
/// network does not leave the rest unchanged.
///
/// `failed` carries networks that failed on an earlier reload. Those still
/// configured are restarted here, then `failed` is refilled with this
/// reload's failures. Returns an error naming each network that failed; the
/// new config stays published either way, because it was validated before
/// this point.
pub(crate) async fn apply_reload<C: NetworkControl + ?Sized>(
    control: &C,
    diff: &ConfigDiff,
    new_config: Arc<Config>,
    failed: &mut HashSet<String>,
) -> Result<()> {
    globals::set_config(new_config.clone());

    let mut retries: Vec<String> = failed
        .drain()
        .filter(|n| new_config.network.contains_key(n))
        .filter(|n| !diff.adapter_restarts_required.contains(n))
        .filter(|n| !diff.networks_to_add.contains(n))
        .collect();
    retries.sort();
    if !retries.is_empty() {
        tracing::info!(
            "Retrying networks that failed on an earlier reload: {:?}",
            retries
        );
    }

    let mut failures = Vec::new();
    for network in &diff.networks_to_remove {
        if let Err(e) = control.stop_network(network).await {
            failures.push(format!("stopping '{}': {}", network, e));
        }
    }
    for network in diff.adapter_restarts_required.iter().chain(&retries) {
        if let Err(e) = control
            .restart_network(network.clone(), new_config.clone())
            .await
        {
            failures.push(format!("restarting '{}': {}", network, e));
            failed.insert(network.clone());
        }
    }
    for network in &diff.networks_to_add {
        if let Err(e) = control
            .start_network(network.clone(), new_config.clone())
            .await
        {
            failures.push(format!("starting '{}': {}", network, e));
            failed.insert(network.clone());
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "New config is active, but some networks failed and will be retried on the next reload: {}",
            failures.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watcher_creation() {
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new(
            "test-config.toml".to_string(),
            Some("pipelines".to_string()),
            registry,
        );

        assert_eq!(watcher.config_path, "test-config.toml");
        assert_eq!(watcher.pipelines_path, Some("pipelines".to_string()));
        assert_eq!(watcher.debounce_duration, Duration::from_millis(200));
    }

    #[tokio::test]
    async fn test_watcher_debounce_duration() {
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new("config.toml".to_string(), None, registry);

        // Verify default debounce duration is 200ms
        assert_eq!(watcher.debounce_duration, Duration::from_millis(200));
    }

    #[test]
    fn test_watcher_has_correct_debounce() {
        // Verify debounce duration is exactly 200ms as specified
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new("test.toml".to_string(), None, registry);

        assert_eq!(watcher.debounce_duration.as_millis(), 200);
    }

    /// Records each call with the proxy id of the config published at that
    /// moment, and fails calls for the networks in `fail`.
    struct Recorder {
        calls: std::sync::Mutex<Vec<(String, String, String)>>,
        fail: HashSet<String>,
    }

    impl Recorder {
        fn new(fail: &[&str]) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                fail: fail.iter().map(|n| n.to_string()).collect(),
            }
        }

        fn record(&self, op: &str, network: &str) -> Result<()> {
            let published = globals::get_config()
                .map(|c| c.proxy.id.clone())
                .unwrap_or_default();
            self.calls
                .lock()
                .unwrap()
                .push((op.to_string(), network.to_string(), published));
            if self.fail.contains(network) {
                Err(anyhow::anyhow!("simulated failure"))
            } else {
                Ok(())
            }
        }

        fn calls(&self) -> Vec<(String, String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl NetworkControl for Recorder {
        async fn stop_network(&self, network: &str) -> Result<()> {
            self.record("stop", network)
        }
        async fn restart_network(&self, network: String, _config: Arc<Config>) -> Result<()> {
            self.record("restart", &network)
        }
        async fn start_network(&self, network: String, _config: Arc<Config>) -> Result<()> {
            self.record("start", &network)
        }
    }

    /// A config with proxy id `id` and the given networks.
    fn config_with(id: &str, networks: &[&str]) -> Arc<Config> {
        let mut config = Config::default();
        config.proxy.id = id.to_string();
        for name in networks {
            config.network.insert(
                name.to_string(),
                crate::models::network::config::NetworkConfig::default(),
            );
        }
        Arc::new(config)
    }

    fn diff_with(remove: &[&str], restart: &[&str], add: &[&str]) -> ConfigDiff {
        let names = |list: &[&str]| list.iter().map(|n| n.to_string()).collect();
        ConfigDiff {
            zero_downtime_changes: Vec::new(),
            adapter_restarts_required: names(restart),
            networks_to_add: names(add),
            networks_to_remove: names(remove),
            process_restart_required: Vec::new(),
        }
    }

    /// Restore the global config a test replaced, for tests that are not
    /// serialised against this one.
    fn restore(previous: Option<Arc<Config>>) {
        if let Some(config) = previous {
            globals::set_config(config);
        }
    }

    /// Every stop, restart and start must already see the new config, and
    /// one failing network must not stop the others.
    #[tokio::test]
    #[serial_test::serial]
    async fn apply_reload_publishes_the_config_before_touching_any_network() {
        let previous = globals::get_config();
        let recorder = Recorder::new(&["b"]);
        let diff = diff_with(&["gone"], &["a", "b"], &["c"]);
        let mut failed = HashSet::new();

        let result = apply_reload(
            &recorder,
            &diff,
            config_with("reload-order-test", &["a", "b", "c"]),
            &mut failed,
        )
        .await;
        restore(previous);

        let err = result.expect_err("network b should fail");
        assert!(
            err.to_string().contains("restarting 'b'"),
            "unexpected error: {}",
            err
        );
        let ops: Vec<(String, String)> = recorder
            .calls()
            .iter()
            .map(|(op, network, published)| {
                assert_eq!(published, "reload-order-test", "{} '{}'", op, network);
                (op.clone(), network.clone())
            })
            .collect();
        let expected: Vec<(String, String)> = [
            ("stop", "gone"),
            ("restart", "a"),
            ("restart", "b"),
            ("start", "c"),
        ]
        .iter()
        .map(|(op, n)| (op.to_string(), n.to_string()))
        .collect();
        assert_eq!(ops, expected);
        assert_eq!(failed, HashSet::from(["b".to_string()]));
    }

    /// A network that failed is restarted on the next reload even when that
    /// reload changes nothing about it, and dropped once it is removed.
    #[tokio::test]
    #[serial_test::serial]
    async fn apply_reload_retries_networks_that_failed_before() {
        let previous = globals::get_config();
        let mut failed = HashSet::from(["flaky".to_string(), "removed".to_string()]);
        let recorder = Recorder::new(&[]);

        let result = apply_reload(
            &recorder,
            &diff_with(&[], &[], &[]),
            config_with("retry-test", &["flaky", "steady"]),
            &mut failed,
        )
        .await;
        restore(previous);

        result.expect("the retry should succeed");
        let calls: Vec<(String, String)> = recorder
            .calls()
            .into_iter()
            .map(|(op, network, _)| (op, network))
            .collect();
        assert_eq!(calls, vec![("restart".to_string(), "flaky".to_string())]);
        assert!(
            failed.is_empty(),
            "failed set should be cleared: {:?}",
            failed
        );
    }

    /// The real registry reports a missing network as a start failure.
    #[tokio::test]
    #[serial_test::serial]
    async fn apply_reload_with_the_registry_reports_a_missing_network() {
        let previous = globals::get_config();
        let registry = AdapterRegistry::new();
        let mut failed = HashSet::new();

        let result = apply_reload(
            &registry,
            &diff_with(&[], &[], &["missing", "quiet"]),
            config_with("registry-test", &["quiet"]),
            &mut failed,
        )
        .await;
        restore(previous);

        let err = result.expect_err("the missing network should fail");
        assert!(err.to_string().contains("starting 'missing'"), "{}", err);
        assert_eq!(registry.get_running_networks().await, vec!["quiet"]);
        assert_eq!(failed, HashSet::from(["missing".to_string()]));
    }

    // Note: More comprehensive integration tests for config reloading
    // are in tests/config_reload_integration.rs which handles the full
    // config lifecycle including validation, diff computation, and
    // adapter registry coordination.
}

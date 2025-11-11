use super::config::Config;
use super::reload::{compute_diff, ConfigDiff};
use super::Cli;
use crate::adapters::registry::AdapterRegistry;
use crate::globals;
use anyhow::Result;
use notify::{Event, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

/// Config file watcher with debouncing
pub struct ConfigWatcher {
    config_path: String,
    registry: Arc<AdapterRegistry>,
    debounce_duration: Duration,
}

impl ConfigWatcher {
    pub fn new(config_path: String, registry: Arc<AdapterRegistry>) -> Self {
        Self {
            config_path,
            registry,
            debounce_duration: Duration::from_millis(200),
        }
    }

    /// Start watching the config file for changes
    pub async fn start(self) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(100);

        let config_path = self.config_path.clone();
        let config_path_clone = config_path.clone();

        // Start file watcher
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() {
                    let _ = tx.blocking_send(());
                }
            }
        })?;

        watcher.watch(Path::new(&config_path), RecursiveMode::NonRecursive)?;

        tracing::info!("📡 Watching config file for changes: {}", config_path_clone);

        // Debounce and handle reload
        let mut last_reload = tokio::time::Instant::now();

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
        // Load new config
        let cli = Cli::new(self.config_path.clone());
        let new_config = Config::from_args(cli);

        // Get current config
        let old_config =
            globals::get_config().ok_or_else(|| anyhow::anyhow!("No config currently loaded"))?;

        // Compute diff
        let diff = compute_diff(&old_config, &new_config);

        if !diff.has_changes() {
            return Ok(diff);
        }

        let new_config_arc = Arc::new(new_config);

        // Handle network topology changes (adapter restarts)
        if diff.requires_adapter_restart() {
            // Remove networks
            for network in &diff.networks_to_remove {
                self.registry.stop_network(network).await?;
            }

            // Restart changed networks
            for network in &diff.adapter_restarts_required {
                self.registry
                    .restart_network(network.clone(), new_config_arc.clone())
                    .await?;
            }

            // Add new networks
            for network in &diff.networks_to_add {
                self.registry
                    .start_network(network.clone(), new_config_arc.clone())
                    .await?;
            }
        }

        // Update global config (zero-downtime swap)
        globals::set_config(new_config_arc);

        Ok(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_watcher_creation() {
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new("test-config.toml".to_string(), registry);

        assert_eq!(watcher.config_path, "test-config.toml");
        assert_eq!(watcher.debounce_duration, Duration::from_millis(200));
    }

    #[tokio::test]
    async fn test_watcher_debounce_duration() {
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new("config.toml".to_string(), registry);

        // Verify default debounce duration is 200ms
        assert_eq!(watcher.debounce_duration, Duration::from_millis(200));
    }

    #[test]
    fn test_watcher_has_correct_debounce() {
        // Verify debounce duration is exactly 200ms as specified
        let registry = Arc::new(AdapterRegistry::new());
        let watcher = ConfigWatcher::new("test.toml".to_string(), registry);

        assert_eq!(watcher.debounce_duration.as_millis(), 200);
    }

    // Note: More comprehensive integration tests for config reloading
    // are in tests/config_reload_integration.rs which handles the full
    // config lifecycle including validation, diff computation, and
    // adapter registry coordination.
}

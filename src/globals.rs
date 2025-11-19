use crate::adapters::registry::AdapterRegistry;
use crate::config::config::Config;
use crate::storage::StorageBackend;
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

/// Global configuration using ArcSwap for lock-free reads
static CONFIG: Lazy<ArcSwap<Option<Config>>> = Lazy::new(|| ArcSwap::from_pointee(None));
static STORAGE_CELL: Lazy<RwLock<Option<Arc<dyn StorageBackend>>>> =
    Lazy::new(|| RwLock::new(None));
static ADAPTER_REGISTRY: Lazy<RwLock<Option<Arc<AdapterRegistry>>>> =
    Lazy::new(|| RwLock::new(None));
static CONFIG_PATH: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static CLOUD_POLLING_TOKEN: Lazy<RwLock<Option<CancellationToken>>> =
    Lazy::new(|| RwLock::new(None));

/// Set the global configuration (initial setup or reload)
pub fn set_config(config: Arc<Config>) {
    CONFIG.store(Arc::new(Some((*config).clone())));
}

/// Get the current configuration (lock-free read)
pub fn get_config() -> Option<Arc<Config>> {
    let guard = CONFIG.load();
    guard.as_ref().as_ref().map(|c| Arc::new(c.clone()))
}

pub fn set_storage(storage: Arc<dyn StorageBackend>) {
    let mut cell = STORAGE_CELL.write().unwrap();
    *cell = Some(storage);
}

pub fn get_storage() -> Option<Arc<dyn StorageBackend>> {
    STORAGE_CELL.read().unwrap().clone()
}

/// Reset global storage. Primarily for testing purposes.
/// In production, storage should only be set once during initialization.
pub fn reset_storage() {
    let mut cell = STORAGE_CELL.write().unwrap();
    *cell = None;
}

/// Set the global adapter registry
pub fn set_adapter_registry(registry: Arc<AdapterRegistry>) {
    let mut cell = ADAPTER_REGISTRY.write().unwrap();
    *cell = Some(registry);
}

/// Get the global adapter registry
pub fn get_adapter_registry() -> Option<Arc<AdapterRegistry>> {
    ADAPTER_REGISTRY.read().unwrap().clone()
}

/// Set the global config file path
pub fn set_config_path(path: String) {
    let mut cell = CONFIG_PATH.write().unwrap();
    *cell = Some(path);
}

/// Get the global config file path
pub fn get_config_path() -> Option<String> {
    CONFIG_PATH.read().unwrap().clone()
}

/// Set the global cloud polling cancellation token
/// If a token already exists, it will be cancelled before being replaced
pub fn set_cloud_polling_token(token: CancellationToken) {
    let mut cell = CLOUD_POLLING_TOKEN.write().unwrap();
    // Cancel existing polling task if any
    if let Some(existing_token) = cell.as_ref() {
        existing_token.cancel();
    }
    *cell = Some(token);
}

/// Get the global cloud polling cancellation token
pub fn get_cloud_polling_token() -> Option<CancellationToken> {
    CLOUD_POLLING_TOKEN.read().unwrap().clone()
}

/// Stop cloud polling by cancelling the token
pub fn stop_cloud_polling() {
    let mut cell = CLOUD_POLLING_TOKEN.write().unwrap();
    if let Some(token) = cell.take() {
        token.cancel();
        tracing::info!("Cloud polling stopped");
    }
}

#[cfg(test)]
mod tests;

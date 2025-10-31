use crate::config::config::Config;
use crate::storage::StorageBackend;
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};

/// Global configuration using ArcSwap for lock-free reads
static CONFIG: Lazy<ArcSwap<Option<Config>>> = Lazy::new(|| ArcSwap::from_pointee(None));
static STORAGE_CELL: Lazy<RwLock<Option<Arc<dyn StorageBackend>>>> = Lazy::new(|| RwLock::new(None));

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

#[cfg(test)]
mod tests;

use crate::config::config::Config;
use crate::storage::StorageBackend;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};

static CONFIG_CELL: Lazy<RwLock<Option<Arc<Config>>>> = Lazy::new(|| RwLock::new(None));
static STORAGE_CELL: Lazy<RwLock<Option<Arc<dyn StorageBackend>>>> = Lazy::new(|| RwLock::new(None));

pub fn set_config(config: Arc<Config>) {
    let mut cell = CONFIG_CELL.write().unwrap();
    *cell = Some(config);
}

pub fn get_config() -> Option<Arc<Config>> {
    CONFIG_CELL.read().unwrap().clone()
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

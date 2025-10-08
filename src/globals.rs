use crate::config::config::Config;
use crate::storage::StorageBackend;
use once_cell::sync::OnceCell;
use std::sync::Arc;

static CONFIG_CELL: OnceCell<Arc<Config>> = OnceCell::new();
static STORAGE_CELL: OnceCell<Arc<dyn StorageBackend>> = OnceCell::new();

pub fn set_config(config: Arc<Config>) {
    let _ = CONFIG_CELL.set(config);
}

pub fn get_config() -> Option<Arc<Config>> {
    CONFIG_CELL.get().cloned()
}

pub fn set_storage(storage: Arc<dyn StorageBackend>) {
    let _ = STORAGE_CELL.set(storage);
}

pub fn get_storage() -> Option<Arc<dyn StorageBackend>> {
    STORAGE_CELL.get().map(Arc::clone)
}

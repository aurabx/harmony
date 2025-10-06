use std::sync::Arc;
use once_cell::sync::OnceCell;
use crate::config::config::Config;

static CONFIG_CELL: OnceCell<Arc<Config>> = OnceCell::new();

pub fn set_config(config: Arc<Config>) {
    let _ = CONFIG_CELL.set(config);
}

pub fn get_config() -> Option<Arc<Config>> {
    CONFIG_CELL.get().cloned()
}
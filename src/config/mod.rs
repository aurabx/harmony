#[allow(clippy::module_inception)]
pub mod config;
pub mod config_storage;
mod logging_config;
pub mod proxy_config;
pub mod resolution;
pub mod reload;
mod runbeam_config;
mod tests;
pub mod watcher;

/// Structure representing application startup arguments or metadata.
#[derive(Debug)]
pub struct Cli {
    /// Path to the configuration file.
    pub config_path: String,
}

impl Cli {
    /// Creates a new `Cli` instance with the provided configuration path.
    ///
    /// # Arguments
    /// - `config_path`: The path to the configuration file.
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

#[allow(clippy::module_inception)]
pub mod config;
pub mod config_storage;
pub mod env_substitution;
mod logging_config;
pub mod provider_config;
pub mod proxy_config;
pub mod resolution;
pub mod reload;
pub mod resource_reference;
mod runbeam_config;
mod tests;
pub mod watcher;

/// Structure representing application startup arguments or metadata.
#[derive(Debug)]
pub struct Cli {
    /// Path to the configuration file.
    pub config_path: String,
    /// If true, validate the configuration and exit without starting the server
    pub validate_only: bool,
}

impl Cli {
    /// Creates a new `Cli` instance with the provided configuration path.
    ///
    /// # Arguments
    /// - `config_path`: The path to the configuration file.
    pub fn new(config_path: String) -> Self {
        Self {
            config_path,
            validate_only: false,
        }
    }

    /// Creates a new `Cli` instance with validation-only mode enabled.
    pub fn new_validate_only(config_path: String) -> Self {
        Self {
            config_path,
            validate_only: true,
        }
    }
}

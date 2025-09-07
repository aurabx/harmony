mod tests;
mod proxy_config;
mod logging_config;
pub mod config;


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
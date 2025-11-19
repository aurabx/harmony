use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub log_to_file: bool,
    #[serde(default)]
    pub log_file_path: String,
}

fn default_log_level() -> String {
    "error".to_string()
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            log_to_file: false,
            log_file_path: String::new(),
        }
    }
}

impl LoggingConfig {
    pub fn validate(&self) -> Result<(), String> {
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.log_level.as_str()) {
            return Err(format!(
                "logging.log_level must be one of: trace, debug, info, warn, error. Got: {}",
                self.log_level
            ));
        }

        if self.log_to_file && self.log_file_path.trim().is_empty() {
            return Err("logging.log_file_path is required when log_to_file is true".to_string());
        }

        Ok(())
    }
}

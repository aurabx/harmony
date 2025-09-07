use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct LoggingConfig {
    pub log_to_file: bool,
    pub log_file_path: String,
}
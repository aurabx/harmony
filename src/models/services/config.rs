use serde::Deserialize;

// Example: Service configuration as detailed in `basic.toml`
#[derive(Debug, Deserialize, Default)]
pub struct ServiceConfig {
    pub r#type: String,
}

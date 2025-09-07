use serde::Deserialize;

// Example: Adding Target configuration as detailed in `basic.toml`
#[derive(Debug, Deserialize, Default)]
pub struct TargetConfig {
    pub r#type: String,
    pub url: String,
}

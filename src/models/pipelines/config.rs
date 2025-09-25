use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Pipeline {
    #[serde(default = "default_description")]
    pub description: String,              // Optional description of the pipeline
    #[serde(default)]
    pub networks: Vec<String>,            // Networks the pipeline belongs to
    #[serde(default)]
    pub endpoints: Vec<String>,           // List of endpoints associated with the pipeline
    #[serde(default)]
    pub backends: Vec<String>,            // Backends linked to the pipeline
    #[serde(default)]
    pub middleware: Vec<String>,          // Ordered middleware or services middleware
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            description: default_description(),
            networks: Vec::new(),
            endpoints: Vec::new(),
            backends: Vec::new(),
            middleware: Vec::new(),
        }
    }
}

fn default_description() -> String {
    "Unnamed pipeline".to_string()
}
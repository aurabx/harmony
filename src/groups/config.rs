use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Group {
    pub description: String,
    pub networks: Vec<String>,
    pub middleware: GroupMiddleware,
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub peers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct GroupMiddleware {
    #[serde(default)]
    pub incoming: Vec<String>,
    #[serde(default)]
    pub outgoing: Vec<String>,
}

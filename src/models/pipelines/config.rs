use serde::{Deserialize, Serialize};
use serde::de::{self, Deserializer};

/// Middleware configuration for a pipeline.
/// Can be either a simple list (both left and right use same list)
/// or split into separate left and right chains.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PipelineMiddleware {
    List(Vec<String>),
    Split { left: Vec<String>, right: Vec<String> },
}

impl PipelineMiddleware {
    /// Get the left chain (request to backend)
    pub fn left_chain(&self) -> Vec<String> {
        match self {
            PipelineMiddleware::List(list) => list.clone(),
            PipelineMiddleware::Split { left, .. } => left.clone(),
        }
    }

    /// Get the right chain (response from backend)
    pub fn right_chain(&self) -> Vec<String> {
        match self {
            PipelineMiddleware::List(list) => list.clone(),
            PipelineMiddleware::Split { right, .. } => right.clone(),
        }
    }

    /// Check if the right chain should be reversed during processing
    /// Returns true for List format (where same middleware runs in reverse on right)
    /// Returns false for Split format (where user explicitly specifies right order)
    pub fn should_reverse_right(&self) -> bool {
        matches!(self, PipelineMiddleware::List(_))
    }

    /// Check if both chains are empty
    pub fn is_empty(&self) -> bool {
        match self {
            PipelineMiddleware::List(list) => list.is_empty(),
            PipelineMiddleware::Split { left, right } => left.is_empty() && right.is_empty(),
        }
    }

    /// Check if a middleware name appears in either chain
    pub fn contains(&self, name: &String) -> bool {
        match self {
            PipelineMiddleware::List(list) => list.contains(name),
            PipelineMiddleware::Split { left, right } => {
                left.contains(name) || right.contains(name)
            }
        }
    }

    /// Get the total number of middleware entries (both chains combined for Split)
    pub fn len(&self) -> usize {
        match self {
            PipelineMiddleware::List(list) => list.len(),
            PipelineMiddleware::Split { left, right } => left.len() + right.len(),
        }
    }

    /// Get a middleware by index (returns from combined left+right for Split)
    pub fn get(&self, index: usize) -> Option<&String> {
        match self {
            PipelineMiddleware::List(list) => list.get(index),
            PipelineMiddleware::Split { left, right } => {
                if index < left.len() {
                    left.get(index)
                } else {
                    right.get(index - left.len())
                }
            }
        }
    }

    /// Convert to a single combined vec (left chain + right chain for Split)
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            PipelineMiddleware::List(list) => list.clone(),
            PipelineMiddleware::Split { left, right } => {
                let mut combined = left.clone();
                combined.extend(right.clone());
                combined
            }
        }
    }
}

impl Default for PipelineMiddleware {
    fn default() -> Self {
        PipelineMiddleware::List(Vec::new())
    }
}

impl<'de> Deserialize<'de> for PipelineMiddleware {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde_json::Value;

        let value = Value::deserialize(deserializer)?;

        match &value {
            // Simple array: middleware = ['a', 'b']
            Value::Array(_) => {
                let list: Vec<String> = serde_json::from_value(value)
                    .map_err(|_| de::Error::custom("expected array of middleware names"))?;
                Ok(PipelineMiddleware::List(list))
            }
            // Object: could be split format with left/right
            Value::Object(map) => {
                let left: Vec<String> = map
                    .get("left")
                    .map(|v| serde_json::from_value(v.clone()).ok())
                    .flatten()
                    .unwrap_or_default();

                let right: Vec<String> = map
                    .get("right")
                    .map(|v| serde_json::from_value(v.clone()).ok())
                    .flatten()
                    .unwrap_or_default();

                if map.len() == 2 && map.contains_key("left") && map.contains_key("right") {
                    Ok(PipelineMiddleware::Split { left, right })
                } else if map.len() <= 2 && (map.contains_key("left") || map.contains_key("right")) {
                    Ok(PipelineMiddleware::Split { left, right })
                } else {
                    Err(de::Error::custom(
                        "middleware must be either an array or an object with 'left' and/or 'right' keys",
                    ))
                }
            }
            _ => Err(de::Error::custom(
                "middleware must be an array or an object with 'left' and/or 'right' keys",
            )),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(default)]
pub struct Pipeline {
    #[serde(default = "default_description")]
    pub description: String, // Optional description of the pipeline
    #[serde(default)]
    pub networks: Vec<String>, // Networks the pipeline belongs to
    #[serde(default)]
    pub endpoints: Vec<String>, // List of endpoints associated with the pipeline
    #[serde(default)]
    pub backends: Vec<String>, // Backends linked to the pipeline
    #[serde(default)]
    pub middleware: PipelineMiddleware, // Middleware configuration (list or split)
}

impl Default for Pipeline {
    fn default() -> Self {
        Self {
            description: default_description(),
            networks: Vec::new(),
            endpoints: Vec::new(),
            backends: Vec::new(),
            middleware: PipelineMiddleware::default(),
        }
    }
}

fn default_description() -> String {
    "Unnamed pipeline".to_string()
}

use serde::Deserialize;
use crate::models::endpoints::endpoint_type::EndpointType;
use std::collections::HashMap;
use std::ops::Deref;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub r#type: String, // The type of endpoint, e.g., "http"
    #[serde(flatten)]
    pub kind: EndpointKind, // Updated to a wrapper for deserialization
    // Add an explicit map of options as fallback if required
    #[serde(default)]
    pub options: Option<HashMap<String, serde_json::Value>>, // Maps options into JSON-compatible fields
}

/// Wrapper around dynamic `EndpointType` for (de-)serialization
pub struct EndpointKind(pub Box<dyn EndpointType<ReqBody=Value, ResBody=Value>>);

/// Allows us to directly call methods like on instances without explicitly accessing `.0`. `build_router``EndpointKind`
impl Deref for EndpointKind {
    type Target = dyn EndpointType<ReqBody = Value, ResBody = Value>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl std::fmt::Debug for EndpointKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EndpointKind")
            .field(&"<dyn EndpointType>")
            .finish()
    }
}

impl<'de> Deserialize<'de> for EndpointKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Helper struct to extract "type" field
        #[derive(Deserialize)]
        struct EndpointTypeHelper {
            r#type: String,
        }

        use serde::de::Error;

        // Deserialize into helper to determine the concrete type
        let helper = EndpointTypeHelper::deserialize(deserializer)?;

        match helper.r#type.to_lowercase().as_str() {
            "http" => Ok(EndpointKind(Box::new(crate::models::endpoints::http::HttpEndpoint {}))),
            "jmix" => Ok(EndpointKind(Box::new(crate::models::endpoints::jmix::JmixEndpoint {}))),
            "fhir" => Ok(EndpointKind(Box::new(crate::models::endpoints::fhir::FhirEndpoint {}))),
            // Add additional mappings as necessary
            _ => Err(Error::custom(format!(
                "Unsupported endpoint type: {}",
                helper.r#type
            ))),
        }
    }
}
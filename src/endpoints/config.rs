use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    #[serde(default)]
    pub path_prefix: String,
    #[serde(flatten)]
    pub kind: EndpointKind,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EndpointKind {
    Dicom {
        #[serde(default)]
        aet: Option<String>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
    },
    Fhir {
        #[serde(default)]
        path_prefix: Option<String>,
    },
    Jdx {
        #[serde(default)]
        path_prefix: Option<String>,
    },
    Basic {
        #[serde(default)]
        path_prefix: Option<String>,
    },
    Custom {
        handler_path: String
    },
}


impl Default for EndpointKind {
    fn default() -> Self {
        EndpointKind::Basic {
            path_prefix: None,
        }
    }
}
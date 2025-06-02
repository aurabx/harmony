use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Backend {
    #[serde(flatten,rename = "type")]
    pub kind: BackendKind,
    #[serde(default)]
    pub middleware: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackendKind {
    Dicom {
        aet: String,
        host: String,
        port: u16,
    },
    Fhir {
        url: String,
    },
    DeadLetter
}
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Backend {
    #[serde(rename = "type")]
    pub type_: BackendType, // Renamed to `type_`
    #[serde(default)]
    pub middleware: Option<Vec<String>>,
}


#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackendType {
    Dicom {
        aet: String,
        host: String,
        port: u16,
    },
    Fhir {
        url: String,
    },
    DeadLetter,
    PassThru
}


use serde_json::Value;
use std::path::Path;
use thiserror::Error;

pub mod model {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
    pub struct CommandMeta {
        // Prefer snake_case; accept camelCase via alias for backward or alt formats
        #[serde(rename = "message_id", alias = "messageId")]
        pub message_id: Option<u16>,
        #[serde(rename = "sop_class_uid", alias = "sopClassUid")]
        pub sop_class_uid: Option<String>,
        pub priority: Option<String>,
        pub direction: Option<String>, // REQUEST or RESPONSE
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
    pub struct QueryMetaEntry {
        #[serde(rename = "match_type", alias = "matchType")]
        pub match_type: Option<String>, // EXACT, WILDCARD, RANGE, LIST, RETURN_KEY, SEQUENCE, UNIVERSAL
    }

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
    pub struct QueryMetadata(pub std::collections::HashMap<String, QueryMetaEntry>);

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    pub struct Wrapper {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub command: Option<CommandMeta>,
        pub identifier: Value, // DICOM JSON dataset per Part 18
        #[serde(
            rename = "query_metadata",
            alias = "queryMetadata",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        pub query_metadata: Option<QueryMetadata>,
    }
}

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("DICOM JSON conversion error: {0}")]
    Json(String),
}

pub type Result<T> = std::result::Result<T, ConvertError>;

pub fn identifier_to_json_value(obj: &dicom_object::mem::InMemDicomObject) -> Result<Value> {
    // Use dicom-json to encode dataset to standard DICOM JSON
    let v = dicom_json::to_value(obj).map_err(|e| ConvertError::Json(format!("{}", e)))?;
    Ok(v)
}

pub fn json_value_to_identifier(v: &Value) -> Result<dicom_object::mem::InMemDicomObject> {
    let obj =
        dicom_json::from_value(v.clone()).map_err(|e| ConvertError::Json(format!("{}", e)))?;
    Ok(obj)
}

pub fn wrap_with_command(
    identifier: Value,
    command: Option<model::CommandMeta>,
    query_meta: Option<model::QueryMetadata>,
) -> model::Wrapper {
    model::Wrapper {
        command,
        identifier,
        query_metadata: query_meta,
    }
}

pub fn unwrap_identifier(wrapper: &model::Wrapper) -> &Value {
    &wrapper.identifier
}

pub fn write_part10(path: &Path, obj: &dicom_object::mem::InMemDicomObject) -> Result<()> {
    use dicom_core::Tag;
    use dicom_dictionary_std::uids;
    use dicom_object::meta::FileMetaTableBuilder;

    // Try to obtain SOP Class UID from the object if present
    let ms_sop_uid = obj
        .element(Tag(0x0008, 0x0016)) // SOP Class UID
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uids::SECONDARY_CAPTURE_IMAGE_STORAGE.into());

    let file_obj = obj
        .clone()
        .with_meta(
            FileMetaTableBuilder::new()
                .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN)
                .media_storage_sop_class_uid(ms_sop_uid.as_str()),
        )
        .map_err(|e| ConvertError::Json(e.to_string()))?;

    file_obj
        .write_to_file(path)
        .map_err(|e| ConvertError::Json(e.to_string()))
}

/// Try to parse a wrapper from a JSON value; if it's not a wrapper, treat it as a raw identifier
pub fn parse_wrapper_or_identifier(
    v: &Value,
) -> (
    Option<model::CommandMeta>,
    Value,
    Option<model::QueryMetadata>,
) {
    // Attempt to deserialize as a wrapper first
    if let Ok(w) = serde_json::from_value::<model::Wrapper>(v.clone()) {
        return (w.command, w.identifier, w.query_metadata);
    }
    (None, v.clone(), None)
}

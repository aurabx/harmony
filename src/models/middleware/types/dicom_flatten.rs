use crate::models::envelope::envelope::{RequestEnvelope, ResponseEnvelope};
use crate::models::middleware::middleware::Middleware;
use crate::utils::Error;
use async_trait::async_trait;
use dicom_core::dictionary::{DataDictionary, VirtualVr};
use dicom_core::Tag;
use dicom_dictionary_std::StandardDataDictionary;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Configuration for DICOM flatten middleware
#[derive(Debug, Deserialize, Clone)]
pub struct DicomFlattenConfig {
    /// When to apply: "left", "right", or "both" (default)
    #[serde(default = "default_apply")]
    pub apply: String,
}

fn default_apply() -> String {
    "both".to_string()
}

/// Parses middleware configuration from options
pub fn parse_config(options: &std::collections::HashMap<String, Value>) -> Result<DicomFlattenConfig, String> {
    let apply = options
        .get("apply")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .to_string();

    if !["left", "right", "both"].contains(&apply.as_str()) {
        return Err(format!("Invalid apply value: {}. Must be 'left', 'right', or 'both'", apply));
    }

    Ok(DicomFlattenConfig { apply })
}

/// DICOM JSON flattening middleware
/// Converts between standard DICOM JSON format and flat key-value pairs
pub struct DicomFlattenMiddleware {
    apply: String,
}

impl DicomFlattenMiddleware {
    pub fn new(config: DicomFlattenConfig) -> Self {
        Self { apply: config.apply }
    }

    fn should_apply_left(&self) -> bool {
        self.apply == "left" || self.apply == "both"
    }

    fn should_apply_right(&self) -> bool {
        self.apply == "right" || self.apply == "both"
    }
}

#[async_trait]
impl Middleware for DicomFlattenMiddleware {
    async fn left(
        &self,
        mut envelope: RequestEnvelope<Value>,
    ) -> Result<RequestEnvelope<Value>, Error> {
        if !self.should_apply_left() {
            return Ok(envelope);
        }

        if let Some(ref data) = envelope.normalized_data {
            // Store snapshot before transformation if not already present
            if envelope.normalized_snapshot.is_none() {
                envelope.normalized_snapshot = Some(data.clone());
            }

            match flatten_dicom_json(data) {
                Ok(flattened) => {
                    envelope.normalized_data = Some(flattened);
                    tracing::debug!("Applied DICOM flatten on left side");
                }
                Err(e) => {
                    tracing::error!("DICOM flatten failed on left side: {}", e);
                    return Err(Error::from(format!("DICOM flatten failed: {}", e)));
                }
            }
        }

        Ok(envelope)
    }

    async fn right(
        &self,
        mut envelope: ResponseEnvelope<Value>,
    ) -> Result<ResponseEnvelope<Value>, Error> {
        if !self.should_apply_right() {
            return Ok(envelope);
        }

        if let Some(ref data) = envelope.normalized_data {
            // Store snapshot before transformation if not already present
            if envelope.normalized_snapshot.is_none() {
                envelope.normalized_snapshot = Some(data.clone());
            }

            match unflatten_dicom_json(data) {
                Ok(unflattened) => {
                    envelope.normalized_data = Some(unflattened);
                    tracing::debug!("Applied DICOM unflatten on right side");
                }
                Err(e) => {
                    tracing::error!("DICOM unflatten failed on right side: {}", e);
                    return Err(Error::from(format!("DICOM unflatten failed: {}", e)));
                }
            }
        }

        Ok(envelope)
    }
}

/// Infer VR from StandardDataDictionary, with fallback to stored metadata or "UN"
fn get_vr_for_tag(tag_hex: &str, vr_map: Option<&BTreeMap<String, String>>) -> String {
    // First try to look up in the standard dictionary
    if let Ok(group) = u16::from_str_radix(&tag_hex[0..4], 16) {
        if let Ok(element) = u16::from_str_radix(&tag_hex[4..8], 16) {
            let tag = Tag(group, element);
            if let Some(entry) = StandardDataDictionary.by_tag(tag) {
                if let VirtualVr::Exact(vr) = entry.vr {
                    return format!("{:?}", vr);
                }
            }
        }
    }

    // Fallback to stored metadata if available
    if let Some(map) = vr_map {
        if let Some(vr) = map.get(tag_hex) {
            return vr.clone();
        }
    }

    // Final fallback: unknown VR
    "UN".to_string()
}

/// Flattens DICOM JSON to simple key-value pairs
/// Stores VR metadata only for non-standard/unknown tags as a safety net
fn flatten_dicom_json(data: &Value) -> Result<Value, String> {
    let obj = data.as_object().ok_or("Expected DICOM JSON to be an object")?;

    let mut flat = BTreeMap::new();
    let mut vr_map: BTreeMap<String, String> = BTreeMap::new();

    for (tag, tag_data) in obj {
        if let Some(tag_obj) = tag_data.as_object() {
            let vr = tag_obj
                .get("vr")
                .and_then(|v| v.as_str())
                .unwrap_or("UN")
                .to_string();

            // Only store non-standard VRs that won't be found in the dictionary
            let is_private_tag = if let Ok(group) = u16::from_str_radix(&tag[0..4], 16) {
                group % 2 == 1 // Odd group numbers are private
            } else {
                false
            };
            
            if is_private_tag || !can_lookup_in_dict(tag) {
                vr_map.insert(tag.clone(), vr.clone());
            }

            if let Some(value_array) = tag_obj.get("Value").and_then(|v| v.as_array()) {
                if value_array.is_empty() {
                    // Empty array - return key, store empty string
                    flat.insert(tag.clone(), Value::String(String::new()));
                } else if vr == "SQ" {
                    // Sequence - recursively flatten each item
                    let mut flattened_items = Vec::new();
                    for item in value_array {
                        if let Ok(flat_item) = flatten_dicom_json(item) {
                            flattened_items.push(flat_item);
                        }
                    }
                    flat.insert(tag.clone(), Value::Array(flattened_items));
                } else if vr == "PN" && value_array.len() == 1 {
                    // Person Name - extract Alphabetic field if present
                    if let Some(pn_obj) = value_array[0].as_object() {
                        if let Some(alphabetic) = pn_obj.get("Alphabetic").and_then(|v| v.as_str()) {
                            flat.insert(tag.clone(), Value::String(alphabetic.to_string()));
                        } else {
                            // Fallback: serialize PN object as string
                            flat.insert(tag.clone(), Value::String(value_array[0].to_string()));
                        }
                    }
                } else if value_array.len() == 1 {
                    // Single value - extract from array
                    flat.insert(tag.clone(), value_array[0].clone());
                } else {
                    // Multiple values - keep as array
                    flat.insert(tag.clone(), Value::Array(value_array.clone()));
                }
            } else {
                // Missing Value field - store null
                flat.insert(tag.clone(), Value::Null);
            }
        }
    }

    // Only include VR metadata if we have non-standard tags
    let mut result = serde_json::to_value(&flat).map_err(|e| e.to_string())?;
    if !vr_map.is_empty() {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "__vr_metadata__".to_string(),
                serde_json::to_value(&vr_map).map_err(|e| e.to_string())?,
            );
        }
    }

    Ok(result)
}

/// Check if a tag can be looked up in the standard dictionary
fn can_lookup_in_dict(tag_hex: &str) -> bool {
    if tag_hex.len() != 8 {
        return false;
    }
    if let (Ok(group), Ok(element)) = (
        u16::from_str_radix(&tag_hex[0..4], 16),
        u16::from_str_radix(&tag_hex[4..8], 16),
    ) {
        let tag = Tag(group, element);
        StandardDataDictionary.by_tag(tag).is_some()
    } else {
        false
    }
}

/// Unflattens DICOM JSON from flat key-value pairs back to standard format
fn unflatten_dicom_json(data: &Value) -> Result<Value, String> {
    let obj = data.as_object().ok_or("Expected flat DICOM JSON to be an object")?;

    // Extract VR metadata if present (fallback for non-standard tags)
    let vr_map: BTreeMap<String, String> = obj
        .get("__vr_metadata__")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let mut result = BTreeMap::new();

    for (key, value) in obj {
        // Skip metadata fields
        if key.starts_with("__") {
            continue;
        }

        // Try to infer VR from dictionary first, then fall back to stored metadata
        let vr = get_vr_for_tag(key, if vr_map.is_empty() { None } else { Some(&vr_map) });

        if vr == "SQ" {
            // Sequence - reconstruct from array of flattened items
            if let Some(array) = value.as_array() {
                let mut items = Vec::new();
                for item in array {
                    if let Ok(unflat_item) = unflatten_dicom_json(item) {
                        items.push(unflat_item);
                    }
                }
                result.insert(
                    key.clone(),
                    json!({
                        "vr": vr,
                        "Value": items
                    }),
                );
            }
        } else if vr == "PN" && value.is_string() {
            // Person Name - reconstruct with Alphabetic structure
            let pn_value = value.as_str().unwrap_or("");
            result.insert(
                key.clone(),
                json!({
                    "vr": vr,
                    "Value": [{ "Alphabetic": pn_value }]
                }),
            );
        } else if value.is_array() {
            // Multi-valued attribute
            result.insert(
                key.clone(),
                json!({
                    "vr": vr,
                    "Value": value
                }),
            );
        } else if value.is_null() {
            // Empty Value array (return key)
            result.insert(
                key.clone(),
                json!({
                    "vr": vr,
                    "Value": []
                }),
            );
        } else {
            // Single value
            result.insert(
                key.clone(),
                json!({
                    "vr": vr,
                    "Value": [value]
                }),
            );
        }
    }

    Ok(Value::Object(
        serde_json::Map::from_iter(result.into_iter()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatten_simple_scalars() {
        let input = json!({
            "00100020": {
                "vr": "LO",
                "Value": ["PID156695"]
            },
            "0020000D": {
                "vr": "UI",
                "Value": ["1.2.826.0.1.3680043.8.498.123"]
            }
        });

        let result = flatten_dicom_json(&input).expect("flatten failed");
        assert_eq!(result.get("00100020"), Some(&Value::String("PID156695".to_string())));
        assert_eq!(result.get("0020000D"), Some(&Value::String("1.2.826.0.1.3680043.8.498.123".to_string())));
    }

    #[test]
    fn test_flatten_person_name() {
        let input = json!({
            "00100010": {
                "vr": "PN",
                "Value": [{"Alphabetic": "Doe^John"}]
            }
        });

        let result = flatten_dicom_json(&input).expect("flatten failed");
        assert_eq!(result.get("00100010"), Some(&Value::String("Doe^John".to_string())));
    }

    #[test]
    fn test_flatten_sequence() {
        let input = json!({
            "00400275": {
                "vr": "SQ",
                "Value": [
                    {
                        "00100010": {
                            "vr": "PN",
                            "Value": [{"Alphabetic": "Test^Patient"}]
                        }
                    }
                ]
            }
        });

        let result = flatten_dicom_json(&input).expect("flatten failed");
        let seq_value = result.get("00400275").and_then(|v| v.as_array());
        assert!(seq_value.is_some());
        assert_eq!(seq_value.unwrap().len(), 1);
    }

    #[test]
    fn test_flatten_empty_value() {
        let input = json!({
            "00100030": {
                "vr": "DA",
                "Value": []
            }
        });

        let result = flatten_dicom_json(&input).expect("flatten failed");
        assert_eq!(result.get("00100030"), Some(&Value::String(String::new())));
    }

    #[test]
    fn test_unflatten_simple_scalars() {
        let input = json!({
            "00100020": "PID156695",
            "0020000D": "1.2.826.0.1.3680043.8.498.123"
        });

        let result = unflatten_dicom_json(&input).expect("unflatten failed");
        let obj = result.as_object().expect("result not object");

        let tag1 = obj.get("00100020").expect("tag missing");
        assert_eq!(tag1.get("vr").and_then(|v| v.as_str()), Some("LO"));
        assert_eq!(tag1.get("Value"), Some(&json!(["PID156695"])));
    }

    #[test]
    fn test_unflatten_person_name() {
        let input = json!({
            "00100010": "Doe^John",
            "__vr_metadata__": {
                "00100010": "PN"
            }
        });

        let result = unflatten_dicom_json(&input).expect("unflatten failed");
        let obj = result.as_object().expect("result not object");

        let tag = obj.get("00100010").expect("tag missing");
        assert_eq!(tag.get("vr").and_then(|v| v.as_str()), Some("PN"));
        let value = tag.get("Value").and_then(|v| v.as_array());
        assert!(value.is_some());
    }

    #[test]
    fn test_round_trip_consistency() {
        let original = json!({
            "00100020": {
                "vr": "LO",
                "Value": ["PID156695"]
            },
            "00100010": {
                "vr": "PN",
                "Value": [{"Alphabetic": "Doe^John"}]
            },
            "0020000D": {
                "vr": "UI",
                "Value": ["1.2.826.0.1.3680043.8.498.123"]
            }
        });

        let flattened = flatten_dicom_json(&original).expect("flatten failed");
        let unflattened = unflatten_dicom_json(&flattened).expect("unflatten failed");

        // Remove metadata for comparison
        let original_obj = original.as_object().expect("original not object");
        let unflat_obj = unflattened.as_object().expect("unflattened not object");

        for (key, original_value) in original_obj {
            let unflat_value = unflat_obj.get(key).expect(&format!("key {} missing in unflattened", key));
            assert_eq!(original_value.get("vr"), unflat_value.get("vr"), "VR mismatch for {}", key);
        }
    }
}

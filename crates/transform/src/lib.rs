use fluvio_jolt::{transform, TransformSpec};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("Failed to read JOLT spec file: {0}")]
    FileRead(#[from] std::io::Error),
    #[error("Failed to parse JOLT spec JSON: {0}")]
    SpecParse(#[from] serde_json::Error),
    #[error("JOLT transformation failed: {0}")]
    TransformFailed(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct TransformConfig {
    pub spec_path: String,
    /// Apply transform on which direction: "left", "right", or "both" (default)
    #[serde(default = "default_apply")]
    pub apply: String,
    /// Whether to fail the request on transform errors
    #[serde(default = "default_fail_on_error")]
    pub fail_on_error: bool,
}

fn default_apply() -> String {
    "both".to_string()
}

fn default_fail_on_error() -> bool {
    true
}

pub struct JoltTransformEngine {
    spec: TransformSpec,
    config: TransformConfig,
}

impl JoltTransformEngine {
    /// Create a new transform engine from a config
    pub fn new(config: TransformConfig) -> Result<Self, TransformError> {
        let spec_content = std::fs::read_to_string(&config.spec_path)?;
        let spec: TransformSpec = serde_json::from_str(&spec_content)?;

        tracing::info!("Loaded JOLT transform spec from: {}", config.spec_path);

        Ok(Self { spec, config })
    }

    /// Create a new transform engine from a spec path (for backwards compatibility)
    pub fn from_spec_path<P: AsRef<Path>>(spec_path: P) -> Result<Self, TransformError> {
        let config = TransformConfig {
            spec_path: spec_path.as_ref().to_string_lossy().to_string(),
            apply: default_apply(),
            fail_on_error: default_fail_on_error(),
        };
        Self::new(config)
    }

    /// Apply the JOLT transform to input JSON
    pub fn transform(&self, input: Value) -> Result<Value, TransformError> {
        transform(input, &self.spec).map_err(|e| TransformError::TransformFailed(e.to_string()))
    }

    /// Check if transform should be applied on the left side (request to backend)
    pub fn should_apply_left(&self) -> bool {
        matches!(self.config.apply.as_str(), "left" | "both")
    }

    /// Check if transform should be applied on the right side (response from backend)
    pub fn should_apply_right(&self) -> bool {
        matches!(self.config.apply.as_str(), "right" | "both")
    }

    /// Whether to fail on transform errors
    pub fn should_fail_on_error(&self) -> bool {
        self.config.fail_on_error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_simple_shift_transform() {
        // Create a temporary JOLT spec file
        let spec = json!([{
            "operation": "shift",
            "spec": {
                "name": "data.name",
                "account": "data.account"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let engine = JoltTransformEngine::from_spec_path(temp_file.path()).unwrap();

        let input = json!({
            "id": 1,
            "name": "John Smith",
            "account": {
                "id": 1000,
                "type": "Checking"
            }
        });

        let output = engine.transform(input).unwrap();

        let expected = json!({
            "data": {
                "name": "John Smith",
                "account": {
                    "id": 1000,
                    "type": "Checking"
                }
            }
        });

        assert_eq!(output, expected);
    }

    #[test]
    fn test_wildcard_transform() {
        let spec = json!([{
            "operation": "shift",
            "spec": {
                "*": "data.&0"
            }
        }]);

        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, serde_json::to_string_pretty(&spec).unwrap()).unwrap();

        let engine = JoltTransformEngine::from_spec_path(temp_file.path()).unwrap();

        let input = json!({
            "id": 1,
            "name": "John Smith"
        });

        let output = engine.transform(input).unwrap();

        let expected = json!({
            "data": {
                "id0": 1,
                "name0": "John Smith"
            }
        });

        assert_eq!(output, expected);
    }

    #[test]
    fn test_config_apply_directions() {
        let config = TransformConfig {
            spec_path: "test.json".to_string(),
            apply: "left".to_string(),
            fail_on_error: true,
        };

        assert!(config.apply == "left");

        let config_both = TransformConfig {
            spec_path: "test.json".to_string(),
            apply: "both".to_string(),
            fail_on_error: false,
        };

        assert!(config_both.apply == "both");
    }

    #[test]
    #[ignore] // Spec file moved; no longer in examples
    fn test_parse_real_metadata_set_dimse_op_spec() {
        // Resolve path relative to this crate directory
        let spec_path = format!(
            "{}/../../samples/jolt/metadata_set_dimse_op.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let engine = JoltTransformEngine::from_spec_path(&spec_path)
            .expect("should parse real metadata_set_dimse_op.json");

        let input = json!({});
        let out = engine.transform(input).expect("transform ok");
        assert_eq!(out.get("dimse_op").and_then(|v| v.as_str()), Some("find"));
    }

    #[test]
    fn test_parse_real_fhir_to_dicom_params_spec() {
        let spec_path = format!(
            "{}/../../examples/fhir-to-dicom/transforms/fhir_to_dicom_params.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let engine = JoltTransformEngine::from_spec_path(&spec_path)
            .expect("should parse real fhir_to_dicom_params.json");

        let input = json!({
            "full_path": "/fhir/ImagingStudy?patient=PID156695",
            "path": "ImagingStudy",
            "headers": {},
            "original_data": {}
        });
        let out = engine.transform(input).expect("transform ok");
        // Should produce an object
        assert!(out.is_object());
    }

    #[test]
    fn test_parse_real_dicom_to_imagingstudy_spec() {
        let spec_path = format!(
            "{}/../../examples/fhir-to-dicom/transforms/dicom_to_imagingstudy_simple.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let engine = JoltTransformEngine::from_spec_path(&spec_path)
            .expect("should parse real dicom_to_imagingstudy_simple.json");

        // Build a minimal mock DICOM find response compatible with the spec
        let input = json!({
            "operation": "find",
            "success": true,
            "matches": [
                {
                    "0020000D": {"vr": "UI", "Value": ["1.2.3"]},
                    "00100020": {"vr": "LO", "Value": ["PID156695"]},
                    "00100010": {"vr": "PN", "Value": [{"Alphabetic": "Doe^John"}]},
                    "00080020": {"vr": "DA", "Value": ["20241015"]},
                    "00080030": {"vr": "TM", "Value": ["120000"]},
                    "00081030": {"vr": "LO", "Value": ["Mock CT Study"]},
                    "00200010": {"vr": "SH", "Value": ["1"]}
                }
            ]
        });

        let out = engine.transform(input).expect("transform ok");
        // Should produce a Bundle with an ImagingStudy entry
        assert_eq!(
            out.get("resourceType").and_then(|v| v.as_str()),
            Some("Bundle")
        );
        assert!(out.get("entry").and_then(|v| v.as_array()).is_some());
    }
}

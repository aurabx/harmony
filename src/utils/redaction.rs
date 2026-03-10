//! Sensitive field pattern matching for automatic redaction.
//!
//! This module provides the `SensitiveFieldMatcher` which compiles regex patterns
//! from the global `proxy.sensitive_field_patterns` configuration and provides
//! efficient matching against field names.

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

/// Matches field names against configured sensitive field patterns.
///
/// Patterns are compiled once at construction time and cached for efficient
/// repeated matching. Patterns use standard regex syntax and are matched
/// case-insensitively against field names.
///
/// # Example patterns
/// - `".*patient.*name.*"` - matches "patient_name", "PatientName", "patient.name"
/// - `".*ssn.*"` - matches "ssn", "SSN", "patient_ssn"
/// - `".*password.*"` - matches "password", "user_password", "PASSWORD"
#[derive(Debug, Clone)]
pub struct SensitiveFieldMatcher {
    patterns: Vec<Regex>,
}

impl Default for SensitiveFieldMatcher {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl SensitiveFieldMatcher {
    /// Creates a new matcher from a list of regex pattern strings.
    ///
    /// Invalid patterns are logged and skipped. The matcher will still
    /// function with the valid patterns.
    pub fn new(patterns: &[String]) -> Self {
        let compiled: Vec<Regex> = patterns
            .iter()
            .filter_map(|p| {
                // Compile as case-insensitive regex
                match Regex::new(&format!("(?i){}", p)) {
                    Ok(re) => Some(re),
                    Err(e) => {
                        tracing::warn!("Invalid sensitive field pattern '{}': {}. Skipping.", p, e);
                        None
                    }
                }
            })
            .collect();

        Self { patterns: compiled }
    }

    /// Returns true if the field name matches any of the sensitive patterns.
    pub fn is_sensitive(&self, field_name: &str) -> bool {
        self.patterns.iter().any(|re| re.is_match(field_name))
    }

    /// Returns true if there are no patterns configured.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Returns the number of compiled patterns.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Redacts header values where the header name matches a sensitive pattern.
    ///
    /// Returns a new HashMap with sensitive header values replaced with `<redacted>`.
    pub fn redact_headers(&self, headers: &HashMap<String, String>) -> HashMap<String, String> {
        if self.is_empty() {
            return headers.clone();
        }

        let mut redacted = headers.clone();
        for (key, value) in redacted.iter_mut() {
            if self.is_sensitive(key) {
                *value = "<redacted>".to_string();
            }
        }
        redacted
    }

    /// Redacts metadata values where the key matches a sensitive pattern.
    ///
    /// Returns a new HashMap with sensitive metadata values replaced with `<redacted>`.
    pub fn redact_metadata(&self, metadata: &HashMap<String, String>) -> HashMap<String, String> {
        self.redact_headers(metadata) // Same logic as headers
    }

    /// Recursively redacts JSON values where the key matches a sensitive pattern.
    ///
    /// This traverses the entire JSON structure and redacts any field whose
    /// key matches one of the sensitive patterns.
    pub fn redact_json(&self, data: &mut Value) {
        if self.is_empty() {
            return;
        }
        self.redact_json_recursive(data);
    }

    fn redact_json_recursive(&self, data: &mut Value) {
        match data {
            Value::Object(ref mut map) => {
                for (key, value) in map.iter_mut() {
                    if self.is_sensitive(key) {
                        *value = Value::String("<redacted>".to_string());
                    } else {
                        // Recurse into nested structures
                        self.redact_json_recursive(value);
                    }
                }
            }
            Value::Array(ref mut arr) => {
                for item in arr.iter_mut() {
                    self.redact_json_recursive(item);
                }
            }
            _ => {} // Primitives don't have keys to match
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_matcher() {
        let matcher = SensitiveFieldMatcher::new(&[]);
        assert!(matcher.is_empty());
        assert!(!matcher.is_sensitive("anything"));
    }

    #[test]
    fn test_basic_pattern_matching() {
        let patterns = vec![
            ".*patient.*name.*".to_string(),
            ".*ssn.*".to_string(),
            ".*password.*".to_string(),
        ];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        // Should match
        assert!(matcher.is_sensitive("patient_name"));
        assert!(matcher.is_sensitive("PatientName"));
        assert!(matcher.is_sensitive("patient.name"));
        assert!(matcher.is_sensitive("ssn"));
        assert!(matcher.is_sensitive("SSN"));
        assert!(matcher.is_sensitive("patient_ssn"));
        assert!(matcher.is_sensitive("password"));
        assert!(matcher.is_sensitive("user_password"));
        assert!(matcher.is_sensitive("PASSWORD"));

        // Should not match
        assert!(!matcher.is_sensitive("patient_id"));
        assert!(!matcher.is_sensitive("username"));
        assert!(!matcher.is_sensitive("email"));
    }

    #[test]
    fn test_medical_record_number_pattern() {
        let patterns = vec![".*medical.*record.*number.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        assert!(matcher.is_sensitive("medical_record_number"));
        assert!(matcher.is_sensitive("MedicalRecordNumber"));
        assert!(matcher.is_sensitive("patient_medical_record_number"));
        assert!(!matcher.is_sensitive("record_id"));
    }

    #[test]
    fn test_case_insensitive_matching() {
        let patterns = vec![".*secret.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        assert!(matcher.is_sensitive("secret"));
        assert!(matcher.is_sensitive("SECRET"));
        assert!(matcher.is_sensitive("Secret"));
        assert!(matcher.is_sensitive("api_secret"));
        assert!(matcher.is_sensitive("API_SECRET"));
    }

    #[test]
    fn test_invalid_pattern_skipped() {
        let patterns = vec![
            ".*valid.*".to_string(),
            "[invalid".to_string(), // Invalid regex (unclosed bracket)
            ".*also_valid.*".to_string(),
        ];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        // Should have 2 valid patterns
        assert_eq!(matcher.len(), 2);
        assert!(matcher.is_sensitive("valid_field"));
        assert!(matcher.is_sensitive("also_valid_field"));
    }

    #[test]
    fn test_redact_headers() {
        let patterns = vec![".*authorization.*".to_string(), ".*secret.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());
        headers.insert("X-Api-Secret".to_string(), "secret-value".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let redacted = matcher.redact_headers(&headers);

        assert_eq!(redacted.get("Authorization").unwrap(), "<redacted>");
        assert_eq!(redacted.get("X-Api-Secret").unwrap(), "<redacted>");
        assert_eq!(redacted.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn test_redact_headers_empty_matcher() {
        let matcher = SensitiveFieldMatcher::new(&[]);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token123".to_string());

        let redacted = matcher.redact_headers(&headers);

        // Nothing should be redacted
        assert_eq!(redacted.get("Authorization").unwrap(), "Bearer token123");
    }

    #[test]
    fn test_redact_json_flat() {
        let patterns = vec![".*ssn.*".to_string(), ".*password.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        let mut data = json!({
            "name": "John Doe",
            "ssn": "123-45-6789",
            "password": "secret123",
            "email": "john@example.com"
        });

        matcher.redact_json(&mut data);

        assert_eq!(data["name"], "John Doe");
        assert_eq!(data["ssn"], "<redacted>");
        assert_eq!(data["password"], "<redacted>");
        assert_eq!(data["email"], "john@example.com");
    }

    #[test]
    fn test_redact_json_nested() {
        let patterns = vec![".*patient.*name.*".to_string(), ".*ssn.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        let mut data = json!({
            "patient": {
                "patient_name": "John Doe",
                "ssn": "123-45-6789",
                "address": {
                    "city": "Boston"
                }
            },
            "metadata": {
                "created_by": "admin"
            }
        });

        matcher.redact_json(&mut data);

        assert_eq!(data["patient"]["patient_name"], "<redacted>");
        assert_eq!(data["patient"]["ssn"], "<redacted>");
        assert_eq!(data["patient"]["address"]["city"], "Boston");
        assert_eq!(data["metadata"]["created_by"], "admin");
    }

    #[test]
    fn test_redact_json_arrays() {
        let patterns = vec![".*secret.*".to_string()];
        let matcher = SensitiveFieldMatcher::new(&patterns);

        let mut data = json!({
            "items": [
                {"id": 1, "secret_key": "abc"},
                {"id": 2, "secret_key": "def"},
                {"id": 3, "value": "public"}
            ]
        });

        matcher.redact_json(&mut data);

        assert_eq!(data["items"][0]["id"], 1);
        assert_eq!(data["items"][0]["secret_key"], "<redacted>");
        assert_eq!(data["items"][1]["secret_key"], "<redacted>");
        assert_eq!(data["items"][2]["value"], "public");
    }

    #[test]
    fn test_redact_json_empty_matcher() {
        let matcher = SensitiveFieldMatcher::new(&[]);

        let mut data = json!({
            "ssn": "123-45-6789",
            "password": "secret"
        });

        let original = data.clone();
        matcher.redact_json(&mut data);

        // Nothing should be redacted
        assert_eq!(data, original);
    }

    #[test]
    fn test_default_matcher() {
        let matcher = SensitiveFieldMatcher::default();
        assert!(matcher.is_empty());
        assert!(!matcher.is_sensitive("anything"));
    }
}

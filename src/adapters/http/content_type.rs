///! Content-Type parsing utilities for HTTP requests
///!
///! This module provides functions to parse various content types into normalized
///! JSON structures for pipeline processing. Supports:
///! - JSON (application/json)
///! - XML (application/xml, text/xml)
///! - CSV (text/csv)
///! - Form URL-encoded (application/x-www-form-urlencoded)
///! - Multipart form data (multipart/form-data)
///! - Binary content (with metadata extraction)
use crate::utils::Error;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Maximum CSV rows to parse (default limit)
const DEFAULT_MAX_CSV_ROWS: usize = 10_000;

/// Maximum XML depth to prevent XML bomb attacks
const DEFAULT_MAX_XML_DEPTH: usize = 100;

/// Maximum form fields to parse
const DEFAULT_MAX_FORM_FIELDS: usize = 1_000;

/// Maximum multipart files
const DEFAULT_MAX_MULTIPART_FILES: usize = 10;

/// Content type parsing result with media type and parameters
#[derive(Debug, Clone)]
pub struct ContentType {
    pub media_type: String,
    pub charset: Option<String>,
    pub boundary: Option<String>,
}

/// Parse status for content parsing operations
#[derive(Debug, Clone, PartialEq)]
pub enum ParseStatus {
    Success,
    Failed,
    NotAttempted,
    Unsupported,
}

/// Result of content parsing operation
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub normalized_data: Option<Value>,
    pub status: ParseStatus,
    pub error_message: Option<String>,
}

impl ParseResult {
    pub fn success(data: Value) -> Self {
        Self {
            normalized_data: Some(data),
            status: ParseStatus::Success,
            error_message: None,
        }
    }

    pub fn failed(error: String) -> Self {
        Self {
            normalized_data: None,
            status: ParseStatus::Failed,
            error_message: Some(error),
        }
    }

    pub fn not_attempted() -> Self {
        Self {
            normalized_data: None,
            status: ParseStatus::NotAttempted,
            error_message: None,
        }
    }

    pub fn unsupported(media_type: &str) -> Self {
        Self {
            normalized_data: None,
            status: ParseStatus::Unsupported,
            error_message: Some(format!("Unsupported content type: {}", media_type)),
        }
    }
}

/// Parse Content-Type header into components
///
/// # Example
/// ```ignore
/// let ct = parse_content_type("application/json; charset=utf-8")?;
/// assert_eq!(ct.media_type, "application/json");
/// assert_eq!(ct.charset, Some("utf-8".to_string()));
/// ```
pub fn parse_content_type(header: &str) -> Result<ContentType, Error> {
    let parts: Vec<&str> = header.split(';').map(|s| s.trim()).collect();

    if parts.is_empty() {
        return Err(Error::from("Empty Content-Type header"));
    }

    let media_type = parts[0].to_lowercase();
    let mut charset = None;
    let mut boundary = None;

    for part in parts.iter().skip(1) {
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"').to_string();

            match key.as_str() {
                "charset" => charset = Some(value),
                "boundary" => boundary = Some(value),
                _ => {}
            }
        }
    }

    Ok(ContentType {
        media_type,
        charset,
        boundary,
    })
}

/// Parse application/x-www-form-urlencoded body
///
/// Handles array notation (field[], field[0], field[key]) and converts
/// to JSON structure.
pub fn parse_form_urlencoded(body: &[u8]) -> Result<Value, Error> {
    parse_form_urlencoded_with_limit(body, DEFAULT_MAX_FORM_FIELDS)
}

/// Parse form data with configurable field limit
pub fn parse_form_urlencoded_with_limit(body: &[u8], max_fields: usize) -> Result<Value, Error> {
    let body_str = std::str::from_utf8(body)
        .map_err(|e| Error::from(format!("Invalid UTF-8 in form data: {}", e)))?;

    let mut result = serde_json::Map::new();
    let mut field_count = 0;

    for (key, value) in url::form_urlencoded::parse(body_str.as_bytes()) {
        if field_count >= max_fields {
            return Err(Error::from(format!(
                "Form field count exceeds limit of {}",
                max_fields
            )));
        }
        field_count += 1;

        // Handle array notation: field[] or field[0]
        if key.ends_with("[]") {
            let key_base = key.trim_end_matches("[]");
            result
                .entry(key_base)
                .or_insert_with(|| Value::Array(vec![]))
                .as_array_mut()
                .ok_or_else(|| Error::from(format!("Field '{}' is not an array", key_base)))?
                .push(Value::String(value.to_string()));
        } else if let Some((key_base, _)) = key.split_once('[') {
            // Handle field[key] notation
            result
                .entry(key_base)
                .or_insert_with(|| Value::Array(vec![]))
                .as_array_mut()
                .ok_or_else(|| Error::from(format!("Field '{}' is not an array", key_base)))?
                .push(Value::String(value.to_string()));
        } else {
            result.insert(key.to_string(), Value::String(value.to_string()));
        }
    }

    Ok(Value::Object(result))
}

/// Parse XML to JSON structure
///
/// Implements XXE prevention by disabling external entities.
/// Converts XML structure to JSON-compatible format.
pub fn parse_xml(body: &[u8]) -> Result<Value, Error> {
    parse_xml_with_limit(body, DEFAULT_MAX_XML_DEPTH)
}

/// Parse XML with configurable depth limit
pub fn parse_xml_with_limit(body: &[u8], max_depth: usize) -> Result<Value, Error> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(body);
    // Note: quick-xml doesn't support external entities by default, providing inherent XXE protection

    let mut stack: Vec<(String, serde_json::Map<String, Value>)> = Vec::new();
    let mut current_text = String::new();
    let mut depth = 0;

    loop {
        if depth > max_depth {
            return Err(Error::from(format!(
                "XML depth exceeds limit of {}",
                max_depth
            )));
        }

        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let mut attrs = serde_json::Map::new();

                // Capture attributes
                for attr in e.attributes() {
                    if let Ok(attr) = attr {
                        let key = format!("@{}", String::from_utf8_lossy(attr.key.as_ref()));
                        let value = String::from_utf8_lossy(&attr.value).to_string();
                        attrs.insert(key, Value::String(value));
                    }
                }

                stack.push((name, attrs));
                current_text.clear();
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if let Some((name, mut attrs)) = stack.pop() {
                    let element_value = if !current_text.trim().is_empty() && attrs.is_empty() {
                        // Text-only element with no attributes -> simple string
                        Value::String(current_text.trim().to_string())
                    } else if !current_text.trim().is_empty() {
                        // Text + attributes -> object with #text
                        attrs.insert(
                            "#text".to_string(),
                            Value::String(current_text.trim().to_string()),
                        );
                        Value::Object(attrs)
                    } else if !attrs.is_empty() {
                        // Attributes only, no text
                        Value::Object(attrs)
                    } else {
                        // Empty element
                        Value::String("".to_string())
                    };

                    if let Some((_, parent_attrs)) = stack.last_mut() {
                        // Add to parent
                        parent_attrs
                            .entry(name.clone())
                            .and_modify(|v| {
                                // Convert to array if multiple elements with same name
                                if let Value::Array(arr) = v {
                                    arr.push(element_value.clone());
                                } else {
                                    *v = Value::Array(vec![v.clone(), element_value.clone()]);
                                }
                            })
                            .or_insert(element_value);
                    } else {
                        // Root element
                        let mut root = serde_json::Map::new();
                        root.insert(name, element_value);
                        return Ok(Value::Object(root));
                    }

                    current_text.clear();
                }
            }
            Ok(Event::Text(e)) => {
                current_text.push_str(&e.unescape().unwrap_or_default());
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::from(format!("XML parsing error: {}", e))),
            _ => {}
        }
    }

    Err(Error::from("Invalid XML: no root element found"))
}

/// Parse CSV to JSON array of objects
///
/// Sanitizes fields to prevent formula injection.
/// First row is treated as header.
pub fn parse_csv(body: &[u8]) -> Result<Value, Error> {
    parse_csv_with_limit(body, DEFAULT_MAX_CSV_ROWS)
}

/// Parse CSV with configurable row limit
pub fn parse_csv_with_limit(body: &[u8], max_rows: usize) -> Result<Value, Error> {
    use csv::ReaderBuilder;

    let mut reader = ReaderBuilder::new().has_headers(true).from_reader(body);

    let headers = reader
        .headers()
        .map_err(|e| Error::from(format!("CSV header error: {}", e)))?
        .iter()
        .map(|h| sanitize_csv_field(h))
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    let mut row_count = 0;

    for result in reader.records() {
        if row_count >= max_rows {
            return Err(Error::from(format!(
                "CSV row count exceeds limit of {}",
                max_rows
            )));
        }
        row_count += 1;

        let record = result.map_err(|e| Error::from(format!("CSV record error: {}", e)))?;

        let mut row_obj = serde_json::Map::new();
        for (i, field) in record.iter().enumerate() {
            if let Some(header) = headers.get(i) {
                row_obj.insert(header.clone(), Value::String(sanitize_csv_field(field)));
            }
        }

        rows.push(Value::Object(row_obj));
    }

    Ok(json!({ "rows": rows, "row_count": rows.len() }))
}

/// Sanitize CSV field to prevent formula injection
///
/// Strips leading =, +, -, @ characters that could be interpreted
/// as formulas in spreadsheet applications.
fn sanitize_csv_field(field: &str) -> String {
    let trimmed = field.trim();
    if trimmed.starts_with('=')
        || trimmed.starts_with('+')
        || trimmed.starts_with('-')
        || trimmed.starts_with('@')
    {
        format!("'{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// Parse multipart/form-data body
///
/// Extracts form fields and file metadata. Files are not stored to disk
/// in this implementation; instead, metadata is captured for pipeline processing.
pub async fn parse_multipart(body: &[u8], boundary: Option<String>) -> Result<Value, Error> {
    parse_multipart_with_limit(body, boundary, DEFAULT_MAX_MULTIPART_FILES).await
}

/// Parse multipart with configurable file limit
pub async fn parse_multipart_with_limit(
    body: &[u8],
    boundary: Option<String>,
    max_files: usize,
) -> Result<Value, Error> {
    use bytes::Bytes;
    use futures_util::stream;
    use multer::Multipart;

    let boundary = boundary.ok_or_else(|| Error::from("Missing boundary in multipart data"))?;

    // Create a stream from the body bytes
    let body_bytes = Bytes::from(body.to_vec());
    let stream = stream::once(async move { Ok::<Bytes, std::io::Error>(body_bytes) });

    let mut multipart = Multipart::new(stream, boundary);

    let mut fields = serde_json::Map::new();
    let mut files = Vec::new();
    let mut file_count = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::from(format!("Multipart parsing error: {}", e)))?
    {
        let name = field
            .name()
            .ok_or_else(|| Error::from("Field missing name"))?
            .to_string();

        let filename = field.file_name().map(|s| s.to_string());

        if let Some(filename) = filename {
            // File upload
            if file_count >= max_files {
                return Err(Error::from(format!(
                    "Multipart file count exceeds limit of {}",
                    max_files
                )));
            }
            file_count += 1;

            let content_type = field.content_type().map(|m| m.to_string());
            let data = field
                .bytes()
                .await
                .map_err(|e| Error::from(format!("Failed to read file data: {}", e)))?;

            files.push(json!({
                "name": name,
                "filename": filename,
                "content_type": content_type,
                "size": data.len(),
                "checksum": calculate_checksum(&data),
            }));
        } else {
            // Regular field
            let value = field
                .text()
                .await
                .map_err(|e| Error::from(format!("Failed to read field text: {}", e)))?;
            fields.insert(name, Value::String(value));
        }
    }

    Ok(json!({
        "fields": Value::Object(fields),
        "files": files,
    }))
}

/// Calculate SHA256 checksum for binary data
///
/// Returns hex-encoded string.
pub fn calculate_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Detect if content is likely binary based on content-type
pub fn is_binary_content(media_type: &str) -> bool {
    media_type.starts_with("image/")
        || media_type.starts_with("video/")
        || media_type.starts_with("audio/")
        || media_type.starts_with("application/octet-stream")
        || media_type.starts_with("application/pdf")
        || media_type.starts_with("application/zip")
}

/// Create metadata for binary content
pub fn create_binary_metadata(media_type: &str, data: &[u8]) -> Value {
    json!({
        "format": "binary",
        "content_type": media_type,
        "size": data.len(),
        "checksum": calculate_checksum(data),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_type() {
        let ct = parse_content_type("application/json; charset=utf-8").unwrap();
        assert_eq!(ct.media_type, "application/json");
        assert_eq!(ct.charset, Some("utf-8".to_string()));
        assert_eq!(ct.boundary, None);
    }

    #[test]
    fn test_parse_content_type_with_boundary() {
        let ct =
            parse_content_type("multipart/form-data; boundary=----WebKitFormBoundary").unwrap();
        assert_eq!(ct.media_type, "multipart/form-data");
        assert_eq!(ct.boundary, Some("----WebKitFormBoundary".to_string()));
    }

    #[test]
    fn test_parse_form_urlencoded() {
        let body = b"name=Alice&age=30&city=NYC";
        let result = parse_form_urlencoded(body).unwrap();

        assert_eq!(result["name"], "Alice");
        assert_eq!(result["age"], "30");
        assert_eq!(result["city"], "NYC");
    }

    #[test]
    fn test_parse_form_urlencoded_arrays() {
        let body = b"name=Alice&interests[]=coding&interests[]=music";
        let result = parse_form_urlencoded(body).unwrap();

        assert_eq!(result["name"], "Alice");
        assert!(result["interests"].is_array());
        let interests = result["interests"].as_array().unwrap();
        assert_eq!(interests.len(), 2);
        assert_eq!(interests[0], "coding");
        assert_eq!(interests[1], "music");
    }

    #[test]
    fn test_sanitize_csv_field() {
        assert_eq!(sanitize_csv_field("normal"), "normal");
        assert_eq!(sanitize_csv_field("=SUM(A1:A10)"), "'=SUM(A1:A10)");
        assert_eq!(sanitize_csv_field("+1234"), "'+1234");
        assert_eq!(sanitize_csv_field("-1234"), "'-1234");
        assert_eq!(sanitize_csv_field("@import"), "'@import");
    }

    #[test]
    fn test_parse_csv() {
        let csv_data = b"name,age,city\nAlice,30,NYC\nBob,25,LA";
        let result = parse_csv(csv_data).unwrap();

        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Alice");
        assert_eq!(rows[0]["age"], "30");
        assert_eq!(rows[1]["name"], "Bob");
    }

    #[test]
    fn test_parse_xml_simple() {
        let xml_data = b"<person><name>Alice</name><age>30</age></person>";
        let result = parse_xml(xml_data).unwrap();

        assert!(result["person"].is_object());
        let person = result["person"].as_object().unwrap();
        assert_eq!(person["name"], "Alice");
        assert_eq!(person["age"], "30");
    }

    #[test]
    fn test_calculate_checksum() {
        let data = b"Hello, World!";
        let checksum = calculate_checksum(data);
        assert_eq!(
            checksum,
            "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"
        );
    }

    #[test]
    fn test_is_binary_content() {
        assert!(is_binary_content("image/jpeg"));
        assert!(is_binary_content("video/mp4"));
        assert!(is_binary_content("application/octet-stream"));
        assert!(!is_binary_content("application/json"));
        assert!(!is_binary_content("text/html"));
    }
}

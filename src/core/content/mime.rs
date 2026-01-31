//! MIME type classification for content detection.
//!
//! This module provides functions to classify MIME types as definitively
//! text or binary. When a MIME type is not definitively classified,
//! byte-level sniffing should be used as a fallback.

/// Returns true if the MIME type is definitively text-based.
///
/// Text-based content can be safely parsed, transformed, and re-encoded
/// without risk of corruption.
///
/// # Arguments
///
/// * `mime` - The MIME type string, may include parameters like charset.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_definitely_text("text/plain"));
/// assert!(is_definitely_text("application/json"));
/// assert!(is_definitely_text("application/fhir+json"));
/// assert!(is_definitely_text("text/html; charset=utf-8"));
/// ```
pub fn is_definitely_text(mime: &str) -> bool {
    let mime = mime.to_lowercase();
    let base = mime.split(';').next().unwrap_or(&mime).trim();

    // Text types
    base.starts_with("text/")
        // JSON types (including FHIR, DICOM-JSON, etc.)
        || base == "application/json"
        || base.ends_with("+json")
        // XML types
        || base == "application/xml"
        || base.ends_with("+xml")
        // JavaScript
        || base == "application/javascript"
        || base == "application/x-javascript"
        || base == "text/javascript"
        // Form data
        || base == "application/x-www-form-urlencoded"
        // GraphQL
        || base == "application/graphql"
        || base == "application/graphql+json"
        // YAML
        || base == "application/x-yaml"
        || base == "application/yaml"
        // LD+JSON (JSON-LD)
        || base == "application/ld+json"
}

/// Returns true if the MIME type is definitively binary.
///
/// Binary content should be passed through untouched. Attempting to parse
/// or transform binary content as text will cause corruption.
///
/// # Arguments
///
/// * `mime` - The MIME type string, may include parameters.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_definitely_binary("image/png"));
/// assert!(is_definitely_binary("application/octet-stream"));
/// assert!(is_definitely_binary("application/dicom"));
/// ```
pub fn is_definitely_binary(mime: &str) -> bool {
    let mime = mime.to_lowercase();
    let base = mime.split(';').next().unwrap_or(&mime).trim();

    // Media types
    base.starts_with("image/")
        || base.starts_with("video/")
        || base.starts_with("audio/")
        // Font types
        || base.starts_with("font/")
        || base == "application/font-woff"
        || base == "application/font-woff2"
        || base == "application/x-font-ttf"
        || base == "application/x-font-otf"
        // 3D model types
        || base.starts_with("model/")
        // Generic binary
        || base == "application/octet-stream"
        // Documents
        || base == "application/pdf"
        // Archives
        || base == "application/zip"
        || base == "application/gzip"
        || base == "application/x-gzip"
        || base == "application/x-tar"
        || base == "application/x-7z-compressed"
        || base == "application/x-rar-compressed"
        || base == "application/x-bzip"
        || base == "application/x-bzip2"
        || base == "application/x-xz"
        // WebAssembly
        || base == "application/wasm"
        // Microsoft Office
        || base == "application/vnd.ms-excel"
        || base == "application/vnd.ms-powerpoint"
        || base == "application/msword"
        || base == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || base == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || base == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        // OpenDocument
        || base == "application/vnd.oasis.opendocument.text"
        || base == "application/vnd.oasis.opendocument.spreadsheet"
        || base == "application/vnd.oasis.opendocument.presentation"
        // DICOM (binary format, not DICOM+JSON which is text)
        || base == "application/dicom"
        // Multipart (typically contains binary parts)
        || base == "multipart/related"
        // Java
        || base == "application/java-archive"
        // Executables
        || base == "application/x-executable"
        || base == "application/x-sharedlib"
        || base == "application/x-mach-binary"
        || base == "application/vnd.microsoft.portable-executable"
        // Protobuf (binary by default)
        || base == "application/x-protobuf"
        || base == "application/protobuf"
        // MessagePack
        || base == "application/msgpack"
        || base == "application/x-msgpack"
        // CBOR
        || base == "application/cbor"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_types() {
        assert!(is_definitely_text("text/plain"));
        assert!(is_definitely_text("text/html"));
        assert!(is_definitely_text("text/css"));
        assert!(is_definitely_text("text/csv"));
        assert!(is_definitely_text("application/json"));
        assert!(is_definitely_text("application/fhir+json"));
        assert!(is_definitely_text("application/dicom+json"));
        assert!(is_definitely_text("application/xml"));
        assert!(is_definitely_text("application/soap+xml"));
        assert!(is_definitely_text("text/html; charset=utf-8"));
        assert!(is_definitely_text("application/javascript"));
        assert!(is_definitely_text("application/x-www-form-urlencoded"));
    }

    #[test]
    fn test_binary_types() {
        assert!(is_definitely_binary("image/png"));
        assert!(is_definitely_binary("image/jpeg"));
        assert!(is_definitely_binary("image/gif"));
        assert!(is_definitely_binary("image/webp"));
        assert!(is_definitely_binary("video/mp4"));
        assert!(is_definitely_binary("audio/mpeg"));
        assert!(is_definitely_binary("application/octet-stream"));
        assert!(is_definitely_binary("application/pdf"));
        assert!(is_definitely_binary("application/zip"));
        assert!(is_definitely_binary("application/gzip"));
        assert!(is_definitely_binary("application/x-tar"));
        assert!(is_definitely_binary("application/wasm"));
        assert!(is_definitely_binary("application/dicom"));
        assert!(is_definitely_binary("multipart/related"));
        assert!(is_definitely_binary("font/woff2"));
        assert!(is_definitely_binary("model/gltf-binary"));
    }

    #[test]
    fn test_ambiguous_types() {
        // These are NOT definitively text or binary - they require sniffing
        assert!(!is_definitely_text("application/octet-stream"));
        assert!(!is_definitely_text("application/dicom")); // Not JSON variant
        assert!(!is_definitely_binary("text/plain")); // text/* is always text
    }

    #[test]
    fn test_case_insensitive() {
        assert!(is_definitely_text("Application/JSON"));
        assert!(is_definitely_text("TEXT/HTML"));
        assert!(is_definitely_binary("IMAGE/PNG"));
        assert!(is_definitely_binary("APPLICATION/OCTET-STREAM"));
    }

    #[test]
    fn test_with_parameters() {
        assert!(is_definitely_text("application/json; charset=utf-8"));
        assert!(is_definitely_binary("image/png; name=test.png"));
    }
}

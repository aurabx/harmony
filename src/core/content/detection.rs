//! Byte-level content inspection for binary vs text detection.
//!
//! This module provides functions to sniff byte content when MIME type
//! information is missing or unreliable. Uses the `content_inspector` crate
//! for robust detection.

use super::analysis::ContentDisposition;
use content_inspector::{inspect, ContentType};

/// Sniff bytes to determine if content is binary or text.
///
/// Uses the `content_inspector` crate for robust detection based on:
/// - NUL byte presence
/// - BOM (Byte Order Mark) markers
/// - Byte distribution analysis
///
/// # Arguments
///
/// * `sample` - A sample of bytes from the content (first 512-2048 bytes recommended)
///
/// # Returns
///
/// `ContentDisposition::Text` for UTF-8 content, `ContentDisposition::Binary` otherwise.
/// UTF-16/32 are treated as binary because naive string operations could corrupt them.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(sniff(b"Hello, World!"), ContentDisposition::Text);
/// assert_eq!(sniff(&[0x00, 0x01, 0x02]), ContentDisposition::Binary);
/// ```
pub fn sniff(sample: &[u8]) -> ContentDisposition {
    if sample.is_empty() {
        // Empty content is safe to treat as text
        return ContentDisposition::Text;
    }

    match inspect(sample) {
        ContentType::BINARY => ContentDisposition::Binary,
        ContentType::UTF_8 | ContentType::UTF_8_BOM => ContentDisposition::Text,
        // UTF-16/32 are technically text but require special handling
        // Treat as binary to avoid corruption from naive string operations
        ContentType::UTF_16LE | ContentType::UTF_16BE => ContentDisposition::Binary,
        ContentType::UTF_32LE | ContentType::UTF_32BE => ContentDisposition::Binary,
    }
}

/// Conservative binary detection for cases where content_inspector might be too lenient.
///
/// Uses simple heuristics:
/// - Contains NUL (0x00) → binary
/// - >30% non-printable bytes → binary
///
/// This is useful as a fallback or validation check.
///
/// # Arguments
///
/// * `sample` - A sample of bytes from the content
///
/// # Returns
///
/// `ContentDisposition::Text` if the sample appears to be printable text,
/// `ContentDisposition::Binary` otherwise.
pub fn sniff_conservative(sample: &[u8]) -> ContentDisposition {
    if sample.is_empty() {
        return ContentDisposition::Text;
    }

    // NUL byte is strong binary indicator
    if sample.iter().any(|&b| b == 0) {
        return ContentDisposition::Binary;
    }

    // Count non-text bytes (not printable ASCII, tab, newline, carriage return)
    let non_text = sample
        .iter()
        .filter(|&&b| !(b == b'\n' || b == b'\r' || b == b'\t' || (0x20..=0x7E).contains(&b)))
        .count();

    let ratio = non_text as f32 / sample.len() as f32;

    if ratio > 0.30 {
        ContentDisposition::Binary
    } else {
        ContentDisposition::Text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sniff_plain_text() {
        assert_eq!(sniff(b"Hello, World!"), ContentDisposition::Text);
        assert_eq!(sniff(b"The quick brown fox"), ContentDisposition::Text);
    }

    #[test]
    fn test_sniff_json() {
        assert_eq!(
            sniff(b"{\"key\": \"value\", \"number\": 42}"),
            ContentDisposition::Text
        );
    }

    #[test]
    fn test_sniff_xml() {
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><root><item>test</item></root>"),
            ContentDisposition::Text
        );
    }

    #[test]
    fn test_sniff_html() {
        assert_eq!(
            sniff(b"<!DOCTYPE html><html><body>Hello</body></html>"),
            ContentDisposition::Text
        );
    }

    #[test]
    fn test_sniff_binary_with_nul() {
        assert_eq!(sniff(&[0x00, 0x01, 0x02, 0x03]), ContentDisposition::Binary);
        assert_eq!(sniff(b"text\x00with\x00nulls"), ContentDisposition::Binary);
    }

    #[test]
    fn test_sniff_png_header() {
        // PNG magic bytes
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(sniff(&png_header), ContentDisposition::Binary);
    }

    #[test]
    fn test_sniff_gzip_header() {
        // Gzip magic bytes
        let gzip_header = [0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(sniff(&gzip_header), ContentDisposition::Binary);
    }

    #[test]
    fn test_sniff_empty() {
        assert_eq!(sniff(&[]), ContentDisposition::Text);
    }

    #[test]
    fn test_sniff_utf8_bom() {
        // UTF-8 BOM followed by text
        let with_bom = [0xEF, 0xBB, 0xBF, b'H', b'e', b'l', b'l', b'o'];
        assert_eq!(sniff(&with_bom), ContentDisposition::Text);
    }

    #[test]
    fn test_sniff_conservative_plain_text() {
        assert_eq!(
            sniff_conservative(b"Hello, World!"),
            ContentDisposition::Text
        );
    }

    #[test]
    fn test_sniff_conservative_with_nul() {
        assert_eq!(
            sniff_conservative(&[0x00, 0x01, 0x02]),
            ContentDisposition::Binary
        );
    }

    #[test]
    fn test_sniff_conservative_high_non_printable() {
        // >30% non-printable
        let mostly_binary = [0x80, 0x81, 0x82, b'a', b'b', b'c'];
        assert_eq!(
            sniff_conservative(&mostly_binary),
            ContentDisposition::Binary
        );
    }

    #[test]
    fn test_sniff_conservative_low_non_printable() {
        // <30% non-printable (high-bit chars in otherwise text)
        let mostly_text = b"Hello World with some extended chars";
        assert_eq!(sniff_conservative(mostly_text), ContentDisposition::Text);
    }

    #[test]
    fn test_sniff_dicom_file() {
        // Read a real DICOM file from the samples directory
        let dicom_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm");

        let dicom_bytes = std::fs::read(&dicom_path).expect("Failed to read DICOM sample file");

        // DICOM files are binary - they contain pixel data, binary headers, and NUL bytes
        assert_eq!(sniff(&dicom_bytes), ContentDisposition::Binary);
        assert_eq!(sniff_conservative(&dicom_bytes), ContentDisposition::Binary);

        // Also verify that just the first 1024 bytes (typical sniff sample) detect as binary
        let sample = &dicom_bytes[..std::cmp::min(1024, dicom_bytes.len())];
        assert_eq!(sniff(sample), ContentDisposition::Binary);
    }

    #[test]
    fn test_sniff_dicom_preamble() {
        // DICOM files have a 128-byte preamble (often zeros) followed by "DICM" magic
        // The preamble is typically all zeros or contains file meta info
        let mut dicom_header = vec![0u8; 128]; // 128-byte preamble (zeros)
        dicom_header.extend_from_slice(b"DICM"); // DICOM magic bytes

        assert_eq!(sniff(&dicom_header), ContentDisposition::Binary);
        assert_eq!(
            sniff_conservative(&dicom_header),
            ContentDisposition::Binary
        );
    }
}

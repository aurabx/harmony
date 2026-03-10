//! Content analysis for determining binary vs text disposition.

use super::{detection, mime};

/// Protocol-agnostic content metadata for detection decisions.
///
/// This struct collects hints from various sources (protocol headers, encodings,
/// body samples) to make an informed decision about content disposition.
#[derive(Debug, Clone, Default)]
pub struct ContentAnalysis<'a> {
    /// MIME type hint from protocol (e.g., Content-Type header value).
    /// Should be the base MIME type without parameters (e.g., "application/json"
    /// not "application/json; charset=utf-8").
    pub mime_type: Option<&'a str>,

    /// Compression encoding (e.g., gzip, br, deflate, zstd).
    /// If present, content should be treated as binary unless decoded first.
    pub encoding: Option<&'a str>,

    /// Sample of body bytes for sniffing (first 512-2048 bytes recommended).
    /// Used when MIME type is missing or generic (like application/octet-stream).
    pub body_sample: Option<&'a [u8]>,
}

/// Result of content analysis.
///
/// Indicates whether content should be treated as transformable text
/// or opaque binary data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentDisposition {
    /// Content is text-based and can be parsed/transformed.
    Text,
    /// Content is binary and should be passed through untouched.
    Binary,
}

impl ContentAnalysis<'_> {
    /// Determine whether content should be transformed or passed through.
    ///
    /// Decision order:
    /// 1. Compressed content → Binary (unless you decode it first)
    /// 2. MIME type clearly indicates text → Text
    /// 3. MIME type clearly indicates binary → Binary
    /// 4. MIME missing or generic → Sniff bytes
    /// 5. Still uncertain → Binary (safe default)
    ///
    /// Returns `ContentDisposition::Binary` when uncertain, as corrupting
    /// binary data is catastrophic while corrupting text is merely annoying.
    pub fn disposition(&self) -> ContentDisposition {
        // 1. Compressed content - don't touch unless decoding
        if self.is_compressed() {
            return ContentDisposition::Binary;
        }

        // 2. Check MIME type hint
        if let Some(mime) = self.mime_type {
            if mime::is_definitely_text(mime) {
                return ContentDisposition::Text;
            }
            if mime::is_definitely_binary(mime) {
                return ContentDisposition::Binary;
            }
        }

        // 3. MIME missing or generic - sniff bytes
        if let Some(sample) = self.body_sample {
            return detection::sniff(sample);
        }

        // 4. No information - default to binary (safe)
        ContentDisposition::Binary
    }

    /// Check if content has compression encoding applied.
    fn is_compressed(&self) -> bool {
        self.encoding.is_some_and(|e| {
            let e = e.to_lowercase();
            e.contains("gzip")
                || e.contains("deflate")
                || e.contains("br")
                || e.contains("zstd")
                || e.contains("compress")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_is_text() {
        let analysis = ContentAnalysis {
            mime_type: Some("application/json"),
            encoding: None,
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Text);
    }

    #[test]
    fn test_compressed_json_is_binary() {
        let analysis = ContentAnalysis {
            mime_type: Some("application/json"),
            encoding: Some("gzip"),
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_image_is_binary() {
        let analysis = ContentAnalysis {
            mime_type: Some("image/png"),
            encoding: None,
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_sniff_text_sample() {
        let analysis = ContentAnalysis {
            mime_type: None,
            encoding: None,
            body_sample: Some(b"Hello, World!"),
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Text);
    }

    #[test]
    fn test_sniff_binary_sample() {
        let analysis = ContentAnalysis {
            mime_type: None,
            encoding: None,
            body_sample: Some(&[0x00, 0x01, 0x02, 0x03]),
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_no_info_defaults_to_binary() {
        let analysis = ContentAnalysis::default();
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_octet_stream_with_text_sample() {
        // Generic MIME but body is clearly text
        let analysis = ContentAnalysis {
            mime_type: Some("application/octet-stream"),
            encoding: None,
            body_sample: Some(b"{\"key\": \"value\"}"),
        };
        // octet-stream is definitely binary, so we trust MIME over sniffing
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_brotli_compression() {
        let analysis = ContentAnalysis {
            mime_type: Some("text/html"),
            encoding: Some("br"),
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_dicom_mime_type() {
        // DICOM binary format (not DICOM+JSON)
        let analysis = ContentAnalysis {
            mime_type: Some("application/dicom"),
            encoding: None,
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }

    #[test]
    fn test_dicom_json_is_text() {
        // DICOM-JSON is a text format
        let analysis = ContentAnalysis {
            mime_type: Some("application/dicom+json"),
            encoding: None,
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Text);
    }

    #[test]
    fn test_dicom_file_with_full_analysis() {
        // Read a real DICOM file and test with ContentAnalysis
        let dicom_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/dicom/study_1/series_1/CT.1.1.dcm");

        let dicom_bytes = std::fs::read(&dicom_path).expect("Failed to read DICOM sample file");
        let sample = &dicom_bytes[..std::cmp::min(1024, dicom_bytes.len())];

        // Test 1: With correct MIME type - should be binary from MIME alone
        let analysis_with_mime = ContentAnalysis {
            mime_type: Some("application/dicom"),
            encoding: None,
            body_sample: Some(sample),
        };
        assert_eq!(analysis_with_mime.disposition(), ContentDisposition::Binary);

        // Test 2: Without MIME type - should detect binary from byte sniffing
        let analysis_sniff_only = ContentAnalysis {
            mime_type: None,
            encoding: None,
            body_sample: Some(sample),
        };
        assert_eq!(
            analysis_sniff_only.disposition(),
            ContentDisposition::Binary
        );

        // Test 3: With unknown MIME type - should fall back to sniffing
        let analysis_unknown_mime = ContentAnalysis {
            mime_type: Some("application/x-unknown"),
            encoding: None,
            body_sample: Some(sample),
        };
        assert_eq!(
            analysis_unknown_mime.disposition(),
            ContentDisposition::Binary
        );
    }

    #[test]
    fn test_multipart_related_is_binary() {
        // multipart/related is commonly used for DICOM WADO-RS responses
        let analysis = ContentAnalysis {
            mime_type: Some("multipart/related"),
            encoding: None,
            body_sample: None,
        };
        assert_eq!(analysis.disposition(), ContentDisposition::Binary);
    }
}

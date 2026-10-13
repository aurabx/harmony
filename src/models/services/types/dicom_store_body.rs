//! Finding the instance in the body of a C-STORE request.
//!
//! The DICOM SCU backends share this so the rule below has one implementation:
//! a path in a request is never read. A request can come from any HTTP client,
//! and a path it names would be opened with this process's rights.

use std::borrow::Cow;
use std::fmt;

/// The body forms a C-STORE request may use to carry its instance.
const ACCEPTED: &str = "Send the DICOM Part 10 file as the request body, \
     or as base64 in the 'part10_b64' key of a JSON body";

/// Why a C-STORE request body yields no DICOM Part 10 file.
#[derive(Debug)]
pub(crate) enum StoreBodyError {
    /// The body has no bytes.
    Empty,
    /// The body is neither a Part 10 file nor JSON.
    NotPart10OrJson {
        body_len: usize,
        json_error: serde_json::Error,
    },
    /// The JSON body names a path in `file` and carries no instance.
    NamesFile,
    /// The JSON body has no `part10_b64` string.
    NoPart10Key,
    /// `part10_b64` is not base64.
    InvalidBase64(base64::DecodeError),
    /// `part10_b64` decodes to bytes that are not a Part 10 file.
    DecodedNotPart10 { decoded_len: usize },
}

impl fmt::Display for StoreBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "envelope has no original_data. {}", ACCEPTED),
            Self::NotPart10OrJson {
                body_len,
                json_error,
            } => write!(
                f,
                "the {}-byte body is neither a DICOM Part 10 file nor JSON ({}). {}",
                body_len, json_error, ACCEPTED
            ),
            Self::NamesFile => write!(
                f,
                "the body names a path in 'file', and this backend does not read files named by a request. {}",
                ACCEPTED
            ),
            Self::NoPart10Key => {
                write!(f, "the JSON body has no 'part10_b64' string. {}", ACCEPTED)
            }
            Self::InvalidBase64(e) => write!(f, "'part10_b64' is not valid base64: {}", e),
            Self::DecodedNotPart10 { decoded_len } => write!(
                f,
                "'part10_b64' decodes to {} bytes that are not a DICOM Part 10 file",
                decoded_len
            ),
        }
    }
}

/// Detect whether `bytes` looks like a DICOM Part 10 file (DICM magic at offset 128).
pub(crate) fn is_part10(bytes: &[u8]) -> bool {
    bytes.len() > 132 && &bytes[128..132] == b"DICM"
}

/// Find the Part 10 bytes in a C-STORE request body.
///
/// The body is either the Part 10 file itself or a JSON object with the file
/// as base64 in `part10_b64`. The first is what `dicom_to_http` sends from
/// another Harmony. The second is what the DIMSE adapter builds when a DICOM
/// SCU backend is in the same pipeline.
///
/// The `file` key of a JSON body is never read.
pub(crate) fn part10_from_store_body(body: &[u8]) -> Result<Cow<'_, [u8]>, StoreBodyError> {
    use base64::Engine;

    if body.is_empty() {
        return Err(StoreBodyError::Empty);
    }
    if is_part10(body) {
        return Ok(Cow::Borrowed(body));
    }

    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|json_error| StoreBodyError::NotPart10OrJson {
            body_len: body.len(),
            json_error,
        })?;
    let Some(part10_b64) = json.get("part10_b64").and_then(|v| v.as_str()) else {
        return Err(if json.get("file").is_some() {
            StoreBodyError::NamesFile
        } else {
            StoreBodyError::NoPart10Key
        });
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(part10_b64)
        .map_err(StoreBodyError::InvalidBase64)?;
    if !is_part10(&decoded) {
        return Err(StoreBodyError::DecodedNotPart10 {
            decoded_len: decoded.len(),
        });
    }
    Ok(Cow::Owned(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_part10_detects_dicm_magic() {
        let mut buf = vec![0u8; 128];
        buf.extend_from_slice(b"DICM");
        buf.extend_from_slice(&[0u8; 100]);
        assert!(is_part10(&buf));
    }

    #[test]
    fn test_is_part10_rejects_short_buffer() {
        assert!(!is_part10(&[0u8; 100]));
    }

    #[test]
    fn test_is_part10_rejects_wrong_magic() {
        let mut buf = vec![0u8; 128];
        buf.extend_from_slice(b"XXXX");
        buf.extend_from_slice(&[0u8; 100]);
        assert!(!is_part10(&buf));
    }
}

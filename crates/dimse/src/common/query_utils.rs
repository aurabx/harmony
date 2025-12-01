//! Query parameter utilities shared between SCP and SCU

use crate::types::QueryLevel;

/// Normalize a DICOM tag from 8-character format to (gggg,eeee) format
///
/// # Examples
/// - "00100020" -> "0010,0020"
/// - "0010,0020" -> "0010,0020" (already normalized)
pub fn normalize_tag(tag: &str) -> String {
    if tag.len() == 8 {
        format!("{},{}", &tag[0..4], &tag[4..8])
    } else {
        tag.to_string()
    }
}

/// Convert QueryLevel enum to DICOM string representation
///
/// # Examples
/// - QueryLevel::Patient -> "PATIENT"
/// - QueryLevel::Study -> "STUDY"
pub fn query_level_to_string(level: QueryLevel) -> &'static str {
    match level {
        QueryLevel::Patient => "PATIENT",
        QueryLevel::Study => "STUDY",
        QueryLevel::Series => "SERIES",
        QueryLevel::Image => "IMAGE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_tag() {
        assert_eq!(normalize_tag("00100020"), "0010,0020");
        assert_eq!(normalize_tag("0010,0020"), "0010,0020");
        assert_eq!(normalize_tag("PatientID"), "PatientID");
    }

    #[test]
    fn test_query_level_to_string() {
        assert_eq!(query_level_to_string(QueryLevel::Patient), "PATIENT");
        assert_eq!(query_level_to_string(QueryLevel::Study), "STUDY");
        assert_eq!(query_level_to_string(QueryLevel::Series), "SERIES");
        assert_eq!(query_level_to_string(QueryLevel::Image), "IMAGE");
    }
}

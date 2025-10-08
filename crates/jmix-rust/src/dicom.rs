use crate::config::Config;
use crate::error::{JmixError, JmixResult};
use crate::types::Series;
use dicom_object::{open_file, DefaultDicomObject};
use dicom_dictionary_std::tags;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// DICOM metadata extracted from files
#[derive(Debug, Clone, Default)]
pub struct DicomMetadata {
    pub patient_name: Option<String>,
    pub patient_id: Option<String>,
    pub patient_dob: Option<String>,
    pub patient_sex: Option<String>,
    pub study_description: Option<String>,
    pub study_uid: Option<String>,
    pub series: Vec<Series>,
    pub modalities: Vec<String>,
    pub instance_count: usize,
}

/// DICOM processor for extracting metadata
pub struct DicomProcessor;

impl DicomProcessor {
    /// Create a new DICOM processor
    pub fn new() -> Self {
        Self
    }

    /// Process a directory of DICOM files and extract metadata
    pub fn process_dicom_folder<P: AsRef<Path>>(
        &self,
        dicom_path: P,
        config: Option<&Config>,
    ) -> JmixResult<DicomMetadata> {
        let dicom_files = self.find_dicom_files(&dicom_path)?;
        
        if dicom_files.is_empty() {
            return Err(JmixError::Dicom(
                format!("No DICOM files found in: {}", dicom_path.as_ref().display())
            ));
        }

        let mut metadata = DicomMetadata::default();
        let mut series_map: HashMap<String, Series> = HashMap::new();

        for file_path in &dicom_files {
            match self.process_dicom_file(file_path, config) {
                Ok(file_metadata) => {
                    metadata = self.merge_metadata(metadata, file_metadata);
                    
                    // Update series information
                    if let Some(series_uid) = &metadata.study_uid {
                        let series_key = format!("{}_{}", 
                            series_uid,
                            metadata.series.last().map(|s| &s.modality).unwrap_or(&"UNKNOWN".to_string())
                        );
                        
                        if let Some(existing_series) = series_map.get_mut(&series_key) {
                            if let Some(instance_count) = existing_series.instance_count.as_mut() {
                                *instance_count += 1;
                            }
                        } else if let Some(last_series) = metadata.series.last() {
                            series_map.insert(series_key, last_series.clone());
                        }
                    }
                }
                Err(e) => {
                    // Log error but continue processing other files
                    eprintln!("Warning: Failed to process DICOM file {}: {}", file_path.display(), e);
                }
            }
        }

        // Update series with correct instance counts
        metadata.series = series_map.into_values().collect();
        metadata.instance_count = dicom_files.len();

        Ok(metadata)
    }

    /// Find all DICOM files in a directory recursively
    fn find_dicom_files<P: AsRef<Path>>(&self, path: P) -> JmixResult<Vec<std::path::PathBuf>> {
        let mut dicom_files = Vec::new();

        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let file_path = entry.path();
                if self.is_dicom_file(file_path)? {
                    dicom_files.push(file_path.to_path_buf());
                }
            }
        }

        Ok(dicom_files)
    }

    /// Check if a file is a DICOM file
    fn is_dicom_file<P: AsRef<Path>>(&self, file_path: P) -> JmixResult<bool> {
        // First try to parse with dicom-rs
        if let Ok(_) = open_file(&file_path) {
            return Ok(true);
        }

        // Fallback: check for DICOM magic number at offset 128
        let file_path = file_path.as_ref();
        if let Ok(bytes) = fs::read(file_path) {
            if bytes.len() >= 132 && &bytes[128..132] == b"DICM" {
                return Ok(true);
            }
        }

        // Check common DICOM file extensions
        if let Some(extension) = file_path.extension().and_then(|e| e.to_str()) {
            match extension.to_lowercase().as_str() {
                "dcm" | "dicom" | "ima" | "img" => return Ok(true),
                _ => {}
            }
        }

        Ok(false)
    }

    /// Process a single DICOM file
    fn process_dicom_file<P: AsRef<Path>>(
        &self,
        file_path: P,
        config: Option<&Config>,
    ) -> JmixResult<DicomMetadata> {
        match open_file(&file_path) {
            Ok(dicom_obj) => self.extract_dicom_metadata(&dicom_obj),
            Err(_) => {
                // Fallback to config data if DICOM parsing fails
                Ok(self.fallback_metadata_from_config(config))
            }
        }
    }

    /// Extract metadata from a DICOM object
    fn extract_dicom_metadata(&self, dicom_obj: &DefaultDicomObject) -> JmixResult<DicomMetadata> {
        let mut metadata = DicomMetadata::default();

        // Patient Name (0010,0010)
        if let Ok(element) = dicom_obj.element(tags::PATIENT_NAME) {
            if let Ok(value) = element.to_str() {
                metadata.patient_name = Some(self.format_dicom_person_name(&value));
            }
        }

        // Patient ID (0010,0020)
        if let Ok(element) = dicom_obj.element(tags::PATIENT_ID) {
            if let Ok(value) = element.to_str() {
                metadata.patient_id = Some(value.trim().to_string());
            }
        }

        // Patient Birth Date (0010,0030)
        if let Ok(element) = dicom_obj.element(tags::PATIENT_BIRTH_DATE) {
            if let Ok(value) = element.to_str() {
                metadata.patient_dob = Some(self.format_dicom_date(&value));
            }
        }

        // Patient Sex (0010,0040)
        if let Ok(element) = dicom_obj.element(tags::PATIENT_SEX) {
            if let Ok(value) = element.to_str() {
                metadata.patient_sex = Some(value.trim().to_string());
            }
        }

        // Study Description (0008,1030)
        if let Ok(element) = dicom_obj.element(tags::STUDY_DESCRIPTION) {
            if let Ok(value) = element.to_str() {
                metadata.study_description = Some(value.trim().to_string());
            }
        }

        // Study Instance UID (0020,000D)
        if let Ok(element) = dicom_obj.element(tags::STUDY_INSTANCE_UID) {
            if let Ok(value) = element.to_str() {
                metadata.study_uid = Some(value.trim().to_string());
            }
        }

        // Series information
        let series_uid = if let Ok(element) = dicom_obj.element(tags::SERIES_INSTANCE_UID) {
            element.to_str().ok().map(|s| s.trim().to_string())
        } else {
            None
        };

        let modality = if let Ok(element) = dicom_obj.element(tags::MODALITY) {
            element.to_str().ok().map(|s| s.trim().to_string())
        } else {
            None
        };

        let body_part = if let Ok(element) = dicom_obj.element(tags::BODY_PART_EXAMINED) {
            element.to_str().ok().map(|s| s.trim().to_string())
        } else {
            None
        };

        if let Some(uid) = series_uid {
            let modality_name = modality.unwrap_or_else(|| "UNKNOWN".to_string());
            let series = Series {
                series_uid: uid,
                modality: modality_name.clone(),
                body_part,
                instance_count: Some(1),
            };
            metadata.series.push(series);
            
            if modality_name != "UNKNOWN" && !metadata.modalities.contains(&modality_name) {
                metadata.modalities.push(modality_name);
            }
        }

        Ok(metadata)
    }

    /// Format DICOM person name (Family^Given^Middle^Prefix^Suffix) to readable format
    fn format_dicom_person_name(&self, dicom_name: &str) -> String {
        let parts: Vec<&str> = dicom_name.split('^').collect();
        if parts.is_empty() {
            return dicom_name.to_string();
        }

        let family = parts.get(0).unwrap_or(&"").trim();
        let given = parts.get(1).unwrap_or(&"").trim();
        
        if family.is_empty() && given.is_empty() {
            return dicom_name.to_string();
        }

        if given.is_empty() {
            family.to_string()
        } else if family.is_empty() {
            given.to_string()
        } else {
            format!("{}, {}", family, given)
        }
    }

    /// Format DICOM date (YYYYMMDD) to ISO format (YYYY-MM-DD)
    fn format_dicom_date(&self, dicom_date: &str) -> String {
        let date = dicom_date.trim();
        if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) {
            format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
        } else {
            date.to_string()
        }
    }

    /// Merge metadata from multiple DICOM files
    fn merge_metadata(&self, mut existing: DicomMetadata, new: DicomMetadata) -> DicomMetadata {
        // Take first non-empty value for patient data
        existing.patient_name = existing.patient_name.or(new.patient_name);
        existing.patient_id = existing.patient_id.or(new.patient_id);
        existing.patient_dob = existing.patient_dob.or(new.patient_dob);
        existing.patient_sex = existing.patient_sex.or(new.patient_sex);
        existing.study_description = existing.study_description.or(new.study_description);
        existing.study_uid = existing.study_uid.or(new.study_uid);

        // Merge modalities (unique only)
        for modality in new.modalities {
            if !existing.modalities.contains(&modality) {
                existing.modalities.push(modality);
            }
        }

        // Merge series information
        for new_series in new.series {
            let mut found = false;
            for existing_series in &mut existing.series {
                if existing_series.series_uid == new_series.series_uid {
                    // Increment instance count
                    if let Some(count) = existing_series.instance_count.as_mut() {
                        *count += new_series.instance_count.unwrap_or(1);
                    } else {
                        existing_series.instance_count = Some(new_series.instance_count.unwrap_or(1));
                    }
                    
                    // Update other fields if they were empty
                    if existing_series.modality.is_empty() || existing_series.modality == "UNKNOWN" {
                        existing_series.modality = new_series.modality.clone();
                    }
                    if existing_series.body_part.is_none() {
                        existing_series.body_part = new_series.body_part.clone();
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                existing.series.push(new_series);
            }
        }

        existing
    }

    /// Create fallback metadata from configuration when DICOM parsing fails
    fn fallback_metadata_from_config(&self, config: Option<&Config>) -> DicomMetadata {
        let mut metadata = DicomMetadata::default();

        if let Some(config) = config {
            metadata.patient_name = config.patient.name.clone();
            metadata.patient_id = config.patient.id.clone();
            metadata.patient_dob = config.patient.dob.clone();
            metadata.patient_sex = config.patient.sex.clone();

            // Create a placeholder series if no DICOM data available
            metadata.series.push(Series {
                series_uid: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
                modality: "UNKNOWN".to_string(),
                body_part: None,
                instance_count: Some(1),
            });
            metadata.modalities.push("UNKNOWN".to_string());
        }

        metadata
    }
}

impl Default for DicomProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_dicom_processor_creation() {
let _processor = DicomProcessor::new();
        // Just test that it creates successfully
    }

    #[test]
    fn test_is_dicom_file_with_magic_number() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("test.dcm");
        
        // Create a file with DICOM magic number at offset 128
        let mut file = File::create(&file_path).expect("Failed to create file");
        let mut buffer = vec![0u8; 132];
        buffer[128..132].copy_from_slice(b"DICM");
        file.write_all(&buffer).expect("Failed to write file");
        drop(file);

        let processor = DicomProcessor::new();
        let result = processor.is_dicom_file(&file_path).expect("Failed to check file");
        assert!(result, "File with DICM magic should be detected as DICOM");
    }

    #[test]
    fn test_is_dicom_file_by_extension() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let file_path = temp_dir.path().join("test.dcm");
        
        // Create a file without DICM magic but with .dcm extension
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"not a real dicom file").expect("Failed to write file");
        drop(file);

        let processor = DicomProcessor::new();
        let result = processor.is_dicom_file(&file_path).expect("Failed to check file");
        assert!(result, "File with .dcm extension should be detected as DICOM");
    }

    #[test]
    fn test_format_dicom_person_name() {
        let processor = DicomProcessor::new();
        
        // Test standard DICOM format: Family^Given
        assert_eq!(
            processor.format_dicom_person_name("Smith^John"),
            "Smith, John"
        );
        
        // Test with only family name
        assert_eq!(
            processor.format_dicom_person_name("Smith^"),
            "Smith"
        );
        
        // Test with only given name
        assert_eq!(
            processor.format_dicom_person_name("^John"),
            "John"
        );
        
        // Test with no separators
        assert_eq!(
            processor.format_dicom_person_name("John Smith"),
            "John Smith"
        );
    }

    #[test]
    fn test_format_dicom_date() {
        let processor = DicomProcessor::new();
        
        // Test valid DICOM date
        assert_eq!(
            processor.format_dicom_date("19850214"),
            "1985-02-14"
        );
        
        // Test invalid date (not 8 digits)
        assert_eq!(
            processor.format_dicom_date("1985-02-14"),
            "1985-02-14"
        );
        
        // Test with whitespace
        assert_eq!(
            processor.format_dicom_date(" 19850214 "),
            "1985-02-14"
        );
    }

    #[test]
    fn test_fallback_metadata_from_config() {
        let processor = DicomProcessor::new();
        
        let mut config = Config::default();
        config.patient.name = Some("Test Patient".to_string());
        config.patient.id = Some("PAT123".to_string());
        config.patient.dob = Some("1985-02-14".to_string());
        config.patient.sex = Some("M".to_string());

        let metadata = processor.fallback_metadata_from_config(Some(&config));
        
        assert_eq!(metadata.patient_name, Some("Test Patient".to_string()));
        assert_eq!(metadata.patient_id, Some("PAT123".to_string()));
        assert_eq!(metadata.patient_dob, Some("1985-02-14".to_string()));
        assert_eq!(metadata.patient_sex, Some("M".to_string()));
        assert_eq!(metadata.series.len(), 1);
        assert_eq!(metadata.modalities.len(), 1);
        assert_eq!(metadata.modalities[0], "UNKNOWN");
    }

    #[test]
    fn test_process_empty_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let processor = DicomProcessor::new();
        
        let result = processor.process_dicom_folder(temp_dir.path(), None);
        assert!(result.is_err());
        
        match result.unwrap_err() {
            JmixError::Dicom(msg) => assert!(msg.contains("No DICOM files found")),
            other => panic!("Expected DICOM error, got: {:?}", other),
        }
    }
}
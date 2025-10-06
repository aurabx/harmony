//! Common types for DIMSE operations

use bytes::Bytes;
use dicom_object::InMemDicomObject;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Represents a DICOM dataset as either in-memory bytes or a file path
#[derive(Debug, Clone)]
pub enum DatasetStream {
    /// DICOM object in memory
    Memory {
        /// Raw DICOM bytes
        data: Bytes,
        /// Associated metadata
        metadata: DatasetMetadata,
    },
    /// DICOM object stored as temporary file
    File {
        /// Path to the temporary file
        path: PathBuf,
        /// Associated metadata
        metadata: DatasetMetadata,
        /// Whether to delete the file when dropped
        delete_on_drop: bool,
    },
    /// DICOM object already parsed
    Object {
        /// Parsed DICOM object
        object: InMemDicomObject,
        /// Associated metadata
        metadata: DatasetMetadata,
    },
}

/// Metadata associated with a DICOM dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadata {
    /// Unique identifier for this dataset
    pub id: Uuid,
    
    /// Transfer syntax UID
    pub transfer_syntax: Option<String>,
    
    /// SOP Class UID
    pub sop_class_uid: Option<String>,
    
    /// SOP Instance UID
    pub sop_instance_uid: Option<String>,
    
    /// Study Instance UID
    pub study_instance_uid: Option<String>,
    
    /// Series Instance UID
    pub series_instance_uid: Option<String>,
    
    /// Patient ID
    pub patient_id: Option<String>,
    
    /// Timestamp when dataset was received/created
    pub timestamp: chrono::DateTime<chrono::Utc>,
    
    /// Size of the dataset in bytes
    pub size_bytes: Option<u64>,
}

/// DIMSE command types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimseCommand {
    /// C-ECHO command
    Echo,
    /// C-FIND command
    Find,
    /// C-MOVE command
    Move,
    /// C-STORE command (for future use)
    Store,
}

/// Query parameters for C-FIND operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindQuery {
    /// Query level (PATIENT, STUDY, SERIES, IMAGE)
    pub query_level: QueryLevel,
    
    /// Query parameters as DICOM tags and values
    pub parameters: std::collections::HashMap<String, String>,
    
    /// Maximum number of results to return (0 = unlimited)
    pub max_results: u32,
}

/// Query parameters for C-MOVE operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveQuery {
    /// Query level (PATIENT, STUDY, SERIES, IMAGE)
    pub query_level: QueryLevel,
    
    /// Query parameters as DICOM tags and values
    pub parameters: std::collections::HashMap<String, String>,
    
    /// Destination AE Title for the move operation
    pub destination_aet: String,
    
    /// Priority of the move operation
    pub priority: MovePriority,
}

/// DICOM query/retrieve levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryLevel {
    /// Patient level
    Patient,
    /// Study level
    Study,
    /// Series level
    Series,
    /// Image level
    Image,
}

/// Priority levels for C-MOVE operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MovePriority {
    /// Low priority
    Low,
    /// Medium priority (default)
    Medium,
    /// High priority
    High,
}

/// DIMSE operation status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimseStatus {
    /// Operation completed successfully
    Success,
    /// Operation is pending (more responses to follow)
    Pending,
    /// Operation cancelled by user
    Cancel,
    /// Operation failed with error
    Failure(u16), // DICOM status code
    /// Warning occurred during operation
    Warning(u16), // DICOM status code
}

impl DatasetStream {
    /// Create a new in-memory dataset
    pub fn from_bytes(data: Bytes) -> Self {
        Self::Memory {
            data,
            metadata: DatasetMetadata::new(),
        }
    }
    
    /// Create a new file-based dataset
    pub fn from_file(path: PathBuf, delete_on_drop: bool) -> Self {
        Self::File {
            path,
            metadata: DatasetMetadata::new(),
            delete_on_drop,
        }
    }
    
    /// Create a new dataset from a parsed DICOM object
    pub fn from_object(object: InMemDicomObject) -> Self {
        let mut metadata = DatasetMetadata::new();
        
        // Extract metadata from DICOM object
        if let Ok(sop_class) = object.element_by_name("SOPClassUID") {
            if let Ok(value) = sop_class.to_str() {
                metadata.sop_class_uid = Some(value.to_string());
            }
        }
        
        if let Ok(sop_instance) = object.element_by_name("SOPInstanceUID") {
            if let Ok(value) = sop_instance.to_str() {
                metadata.sop_instance_uid = Some(value.to_string());
            }
        }
        
        Self::Object { object, metadata }
    }
    
    /// Get the metadata for this dataset
    pub fn metadata(&self) -> &DatasetMetadata {
        match self {
            Self::Memory { metadata, .. } => metadata,
            Self::File { metadata, .. } => metadata,
            Self::Object { metadata, .. } => metadata,
        }
    }
    
    /// Get mutable metadata for this dataset
    pub fn metadata_mut(&mut self) -> &mut DatasetMetadata {
        match self {
            Self::Memory { metadata, .. } => metadata,
            Self::File { metadata, .. } => metadata,
            Self::Object { metadata, .. } => metadata,
        }
    }
    
    /// Convert to bytes (loading from file if necessary)
    pub async fn to_bytes(&self) -> crate::error::Result<Bytes> {
        match self {
            Self::Memory { data, .. } => Ok(data.clone()),
            Self::File { path, .. } => {
                let bytes = tokio::fs::read(path).await?;
                Ok(Bytes::from(bytes))
            },
            Self::Object { .. } => {
                // TODO: Implement proper DICOM object serialization
                // For now, return empty bytes as placeholder
                Ok(Bytes::new())
            },
        }
    }
    
    /// Convert to a parsed DICOM object
    pub async fn to_object(&self) -> crate::error::Result<InMemDicomObject> {
        match self {
            Self::Object { object, .. } => Ok(object.clone()),
            _ => {
                // TODO: Implement proper DICOM object parsing
                // For now, return empty object as placeholder
                Ok(dicom_object::InMemDicomObject::new_empty())
            },
        }
    }
    
    /// Write to a temporary file in the specified directory
    pub async fn to_temp_file(&self, temp_dir: &std::path::Path) -> crate::error::Result<PathBuf> {
        let temp_file = temp_dir.join(format!("{}.dcm", self.metadata().id));
        
        match self {
            Self::File { path, .. } => {
                tokio::fs::copy(path, &temp_file).await?;
            },
            _ => {
                let bytes = self.to_bytes().await?;
                tokio::fs::write(&temp_file, &bytes).await?;
            },
        }
        
        Ok(temp_file)
    }
}

impl DatasetMetadata {
    /// Create new metadata with a unique ID and current timestamp
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            transfer_syntax: None,
            sop_class_uid: None,
            sop_instance_uid: None,
            study_instance_uid: None,
            series_instance_uid: None,
            patient_id: None,
            timestamp: chrono::Utc::now(),
            size_bytes: None,
        }
    }
}

impl Default for DatasetMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl FindQuery {
    /// Create a new patient-level query
    pub fn patient(patient_id: Option<String>) -> Self {
        let mut parameters = std::collections::HashMap::new();
        if let Some(id) = patient_id {
            parameters.insert("PatientID".to_string(), id);
        }
        
        Self {
            query_level: QueryLevel::Patient,
            parameters,
            max_results: 0,
        }
    }
    
    /// Create a new study-level query
    pub fn study(study_instance_uid: Option<String>) -> Self {
        let mut parameters = std::collections::HashMap::new();
        if let Some(uid) = study_instance_uid {
            parameters.insert("StudyInstanceUID".to_string(), uid);
        }
        
        Self {
            query_level: QueryLevel::Study,
            parameters,
            max_results: 0,
        }
    }
    
    /// Add a query parameter
    pub fn with_parameter(mut self, tag: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(tag.into(), value.into());
        self
    }
    
    /// Set maximum number of results
    pub fn with_max_results(mut self, max: u32) -> Self {
        self.max_results = max;
        self
    }
}

impl MoveQuery {
    /// Create a new move query
    pub fn new(query_level: QueryLevel, destination_aet: impl Into<String>) -> Self {
        Self {
            query_level,
            parameters: std::collections::HashMap::new(),
            destination_aet: destination_aet.into(),
            priority: MovePriority::Medium,
        }
    }
    
    /// Add a query parameter
    pub fn with_parameter(mut self, tag: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(tag.into(), value.into());
        self
    }
    
    /// Set the priority
    pub fn with_priority(mut self, priority: MovePriority) -> Self {
        self.priority = priority;
        self
    }
}

impl std::fmt::Display for QueryLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryLevel::Patient => write!(f, "PATIENT"),
            QueryLevel::Study => write!(f, "STUDY"),
            QueryLevel::Series => write!(f, "SERIES"),
            QueryLevel::Image => write!(f, "IMAGE"),
        }
    }
}

impl std::str::FromStr for QueryLevel {
    type Err = crate::error::DimseError;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "PATIENT" => Ok(QueryLevel::Patient),
            "STUDY" => Ok(QueryLevel::Study),
            "SERIES" => Ok(QueryLevel::Series),
            "IMAGE" => Ok(QueryLevel::Image),
            _ => Err(crate::error::DimseError::config(format!("Invalid query level: {}", s))),
        }
    }
}

// Implement Drop for DatasetStream to handle file cleanup
impl Drop for DatasetStream {
    fn drop(&mut self) {
        if let DatasetStream::File { path, delete_on_drop, .. } = self {
            if *delete_on_drop {
                let path_clone = path.clone();
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::warn!("Failed to delete temporary DICOM file {:?}: {}", path_clone, e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataset_metadata() {
        let metadata = DatasetMetadata::new();
        assert!(metadata.timestamp <= chrono::Utc::now());
        assert!(!metadata.id.is_nil());
    }

    #[test]
    fn test_find_query_builder() {
        let query = FindQuery::patient(Some("12345".to_string()))
            .with_parameter("PatientName", "DOE^JOHN")
            .with_max_results(100);
            
        assert_eq!(query.query_level, QueryLevel::Patient);
        assert_eq!(query.parameters.get("PatientID"), Some(&"12345".to_string()));
        assert_eq!(query.parameters.get("PatientName"), Some(&"DOE^JOHN".to_string()));
        assert_eq!(query.max_results, 100);
    }

    #[test]
    fn test_query_level_parsing() {
        assert_eq!("PATIENT".parse::<QueryLevel>().unwrap(), QueryLevel::Patient);
        assert_eq!("study".parse::<QueryLevel>().unwrap(), QueryLevel::Study);
        assert!("INVALID".parse::<QueryLevel>().is_err());
    }
}
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Re-export filesystem types
pub use harmony_filesystem::{
    FilesystemStorage, StorageBackend, StorageError, StorageResult,
};

// Re-export database types
pub use harmony_database::{
    DatabaseBackend, DatabaseManager, DatabaseOperation, DatabaseStats,
};

/// Configuration for storage backend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default)]
    pub options: std::collections::HashMap<String, serde_json::Value>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "path".to_string(),
            serde_json::Value::String("./tmp".to_string()),
        );

        Self {
            backend: default_backend(),
            options,
        }
    }
}

fn default_backend() -> String {
    "filesystem".to_string()
}

/// Create a storage backend from configuration
pub fn create_storage_backend(config: &StorageConfig) -> StorageResult<Arc<dyn StorageBackend>> {
    match config.backend.as_str() {
        "filesystem" => {
            let path = config
                .options
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("./tmp");

            let storage = FilesystemStorage::new(path)?;
            Ok(Arc::new(storage))
        }
        _ => Err(StorageError::Config(format!(
            "Unknown storage backend: {}",
            config.backend
        ))),
    }
}

#[cfg(test)]
mod tests;

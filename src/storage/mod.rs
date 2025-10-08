use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod filesystem;

pub use filesystem::FilesystemStorage;

/// Error type for storage operations
#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Path(String),
    Config(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "IO error: {}", e),
            StorageError::Path(e) => write!(f, "Path error: {}", e),
            StorageError::Config(e) => write!(f, "Configuration error: {}", e),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(err: std::io::Error) -> Self {
        StorageError::Io(err)
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Storage backend trait for abstracting temporary file operations
///
/// This trait provides a consistent interface for storage operations that can be
/// implemented by different backends (filesystem, cloud storage, etc.)
#[async_trait]
pub trait StorageBackend: Send + Sync + std::fmt::Debug {
    /// Get the base path for this storage backend
    fn base_path(&self) -> &Path;

    /// Whether this backend represents a writable local filesystem root
    /// Defaults to false; concrete backends can override to opt-in.
    fn is_filesystem(&self) -> bool {
        false
    }

    /// Create a subpath relative to the storage root
    fn subpath_str(&self, path: &str) -> PathBuf {
        self.base_path().join(path)
    }

    /// Create a subpath relative to the storage root  
    fn subpath_path(&self, path: &Path) -> PathBuf {
        self.base_path().join(path)
    }

    /// Ensure a directory exists under the storage root, creating it if necessary
    fn ensure_dir_str(&self, path: &str) -> StorageResult<PathBuf> {
        let full_path = self.subpath_str(path);
        std::fs::create_dir_all(&full_path)?;
        Ok(full_path)
    }

    /// Create a temporary directory with a given prefix within a subdirectory
    fn tempdir_in_str(&self, subdir: &str, prefix: &str) -> StorageResult<tempfile::TempDir> {
        let parent = self.ensure_dir_str(subdir)?;
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(&parent)
            .map_err(StorageError::from)
    }

    /// Write bytes to a file at the given relative path
    async fn write_file_str(&self, path: &str, contents: &[u8]) -> StorageResult<PathBuf> {
        let full_path = self.subpath_str(path);

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        tokio::fs::write(&full_path, contents).await?;
        Ok(full_path)
    }

    /// Read bytes from a file at the given relative path
    async fn read_file_str(&self, path: &str) -> StorageResult<Vec<u8>> {
        let full_path = self.subpath_str(path);
        tokio::fs::read(&full_path)
            .await
            .map_err(StorageError::from)
    }

    /// Check if a file exists at the given relative path
    fn exists_str(&self, path: &str) -> bool {
        self.subpath_str(path).exists()
    }

    /// Remove a file or directory at the given relative path
    async fn remove_str(&self, path: &str) -> StorageResult<()> {
        let full_path = self.subpath_str(path);
        if full_path.is_dir() {
            tokio::fs::remove_dir_all(&full_path).await?;
        } else {
            tokio::fs::remove_file(&full_path).await?
        }
        Ok(())
    }
}

/// Configuration for storage backend
#[derive(Debug, Clone, Serialize, Deserialize)]
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

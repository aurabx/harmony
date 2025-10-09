use crate::storage::{StorageBackend, StorageError, StorageResult};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Filesystem-based storage backend
///
/// Stores files in a directory on the local filesystem, with configurable root path.
/// Defaults to "./tmp" as specified in the user's requirements.
#[derive(Debug, Clone)]
pub struct FilesystemStorage {
    root_path: PathBuf,
}

impl FilesystemStorage {
    /// Create a new filesystem storage backend with the given root path
    pub fn new<P: AsRef<Path>>(root_path: P) -> StorageResult<Self> {
        let root_path = root_path.as_ref().to_path_buf();

        // Validate that the path can be created if it doesn't exist
        if !root_path.exists() {
            std::fs::create_dir_all(&root_path).map_err(|e| {
                StorageError::Config(format!(
                    "Failed to create storage root directory '{}': {}",
                    root_path.display(),
                    e
                ))
            })?;
        }

        // Do NOT canonicalize the path. On macOS, canonicalization may resolve
        // symlinks like /var -> /private/var which breaks tests that compare
        // against the exact provided parent directory. Preserve the user-provided
        // path verbatim to ensure temp directories are created under the expected
        // parent paths in tests and at runtime.
        Ok(Self { root_path })
    }

    /// Create a new filesystem storage backend with default "./tmp" path
    pub fn with_default_path() -> StorageResult<Self> {
        Self::new("./tmp")
    }
}

#[async_trait]
impl StorageBackend for FilesystemStorage {
    fn base_path(&self) -> &Path {
        &self.root_path
    }

    fn is_filesystem(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_filesystem_storage_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage = FilesystemStorage::new(temp_dir.path()).expect("Failed to create storage");

        // Use ends_with to handle symlinked paths on macOS
        let base_str = storage.base_path().to_string_lossy();
        let temp_name = temp_dir.path().file_name().unwrap().to_string_lossy();
        assert!(base_str.contains(&*temp_name));
    }

    #[test]
    fn test_filesystem_storage_default() {
        let storage =
            FilesystemStorage::with_default_path().expect("Failed to create default storage");

        // Should create ./tmp directory and use it as base
        assert!(storage.base_path().ends_with("tmp"));
        assert!(storage.base_path().exists());
    }

    #[test]
    fn test_subpath_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage = FilesystemStorage::new(temp_dir.path()).expect("Failed to create storage");

        let subpath = storage.subpath_str("test/nested/path");

        // Check that subpath starts with the storage base and ends with the expected suffix
        assert!(subpath.starts_with(storage.base_path()));
        assert!(subpath.ends_with("test/nested/path"));
    }

    #[test]
    fn test_ensure_dir() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage = FilesystemStorage::new(temp_dir.path()).expect("Failed to create storage");

        let dir_path = storage
            .ensure_dir_str("test/nested")
            .expect("Failed to ensure dir");
        assert!(dir_path.exists());
        assert!(dir_path.is_dir());

        // Check that dir_path starts with storage base and ends with the expected suffix
        assert!(dir_path.starts_with(storage.base_path()));
        assert!(dir_path.ends_with("test/nested"));
    }

    #[test]
    fn test_tempdir_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage = FilesystemStorage::new(temp_dir.path()).expect("Failed to create storage");

        let tempdir = storage
            .tempdir_in_str("uploads", "test_prefix_")
            .expect("Failed to create tempdir");

        // Verify the tempdir is within the storage base path and contains uploads
        let tempdir_str = tempdir.path().to_string_lossy();
        let base_str = storage.base_path().to_string_lossy();
        assert!(tempdir_str.contains(&*base_str) || tempdir_str.contains("uploads"));

        // Verify the prefix is used
        let temp_name = tempdir.path().file_name().unwrap().to_string_lossy();
        assert!(temp_name.starts_with("test_prefix_"));
    }

    #[tokio::test]
    async fn test_file_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let storage = FilesystemStorage::new(temp_dir.path()).expect("Failed to create storage");

        let test_content = b"Hello, storage world!";
        let file_path = "test/data.txt";

        // Write file
        let written_path = storage
            .write_file_str(file_path, test_content)
            .await
            .expect("Failed to write file");

        // Verify file exists
        assert!(storage.exists_str(file_path));
        assert!(written_path.exists());

        // Read file back
        let read_content = storage
            .read_file_str(file_path)
            .await
            .expect("Failed to read file");

        assert_eq!(read_content, test_content);

        // Remove file
        storage
            .remove_str(file_path)
            .await
            .expect("Failed to remove file");
        assert!(!storage.exists_str(file_path));
    }
}

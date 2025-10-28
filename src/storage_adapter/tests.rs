#[cfg(test)]
mod tests {
    use super::super::*;
    use harmony_filesystem::FilesystemStorage;
    use runbeam_sdk::storage::StorageBackend as SdkStorage;
    use tempfile::TempDir;

    async fn create_test_storage() -> (FilesystemStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path().to_str().unwrap()).unwrap();
        (storage, temp_dir)
    }

    #[tokio::test]
    async fn test_storage_adapter_write_and_read() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_data = b"Hello, World!";
        let test_path = "test_file.txt";

        // Write data
        let write_result = adapter.write_file_str(test_path, test_data).await;
        assert!(write_result.is_ok(), "Failed to write file");

        // Read data back
        let read_result = adapter.read_file_str(test_path).await;
        assert!(read_result.is_ok(), "Failed to read file");
        assert_eq!(read_result.unwrap(), test_data);
    }

    #[tokio::test]
    async fn test_storage_adapter_exists() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_path = "exists_test.txt";

        // File should not exist initially
        assert!(!adapter.exists_str(test_path));

        // Write file
        adapter
            .write_file_str(test_path, b"test data")
            .await
            .unwrap();

        // File should now exist
        assert!(adapter.exists_str(test_path));
    }

    #[tokio::test]
    async fn test_storage_adapter_remove() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_path = "remove_test.txt";

        // Write file
        adapter
            .write_file_str(test_path, b"test data")
            .await
            .unwrap();
        assert!(adapter.exists_str(test_path));

        // Remove file
        let remove_result = adapter.remove_str(test_path).await;
        assert!(remove_result.is_ok(), "Failed to remove file");

        // File should no longer exist
        assert!(!adapter.exists_str(test_path));
    }

    #[tokio::test]
    async fn test_storage_adapter_read_nonexistent() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let result = adapter.read_file_str("nonexistent.txt").await;
        assert!(result.is_err(), "Should fail to read nonexistent file");
    }

    #[tokio::test]
    async fn test_storage_adapter_remove_nonexistent() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let result = adapter.remove_str("nonexistent.txt").await;
        assert!(result.is_err(), "Should fail to remove nonexistent file");
    }

    #[tokio::test]
    async fn test_storage_adapter_write_empty_data() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_path = "empty_file.txt";
        let write_result = adapter.write_file_str(test_path, b"").await;
        assert!(write_result.is_ok(), "Should be able to write empty file");

        let read_result = adapter.read_file_str(test_path).await;
        assert!(read_result.is_ok(), "Should be able to read empty file");
        assert_eq!(read_result.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_storage_adapter_overwrite() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_path = "overwrite_test.txt";

        // Write initial data
        adapter
            .write_file_str(test_path, b"initial data")
            .await
            .unwrap();

        // Overwrite with new data
        adapter
            .write_file_str(test_path, b"new data")
            .await
            .unwrap();

        // Read and verify
        let data = adapter.read_file_str(test_path).await.unwrap();
        assert_eq!(data, b"new data");
    }

    #[test]
    fn test_storage_adapter_creation() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilesystemStorage::new(temp_dir.path().to_str().unwrap()).unwrap();
        let _adapter = StorageAdapter::new(&storage);
        // If we get here, construction succeeded
    }

    #[tokio::test]
    async fn test_storage_adapter_with_subdirectory() {
        let (storage, _temp_dir) = create_test_storage().await;
        let adapter = StorageAdapter::new(&storage);

        let test_path = "subdir/nested/file.txt";
        let test_data = b"nested file data";

        // Write to nested path (should create directories)
        let write_result = adapter.write_file_str(test_path, test_data).await;
        assert!(write_result.is_ok(), "Should create nested directories");

        // Read back
        let read_result = adapter.read_file_str(test_path).await;
        assert!(read_result.is_ok());
        assert_eq!(read_result.unwrap(), test_data);
    }
}

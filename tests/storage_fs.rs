use harmony::config::config::Config;
use harmony::storage::{create_storage_backend, StorageConfig};
use tempfile::TempDir;

#[tokio::test]
async fn test_storage_configuration_parsing() {
    let toml = r#"
        [proxy]
        id = "test"
        
        [storage]
        backend = "filesystem"
        
        [storage.options]
        path = "./tmp/test"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [services.http]
        module = ""
    "#;

    let config: Config = toml::from_str(toml).expect("Failed to parse config");
    config.validate().expect("Config validation failed");

    assert_eq!(config.storage.backend, "filesystem");
    let path = config
        .storage
        .options
        .get("path")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(path, "./tmp/test");

    // Test storage backend creation
    let storage =
        create_storage_backend(&config.storage).expect("Failed to create storage backend");
    assert!(storage.base_path().ends_with("test"));
}

#[tokio::test]
async fn test_default_storage_configuration() {
    let toml = r#"
        [proxy]
        id = "test"
        
        [network.default]
        enable_wireguard = false
        interface = "wg0"
        
        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8080
        
        [services.http]
        module = ""
    "#;

    let config: Config = toml::from_str(toml).expect("Failed to parse config");
    config.validate().expect("Config validation failed");

    // Should use default storage config
    assert_eq!(config.storage.backend, "filesystem");
    let path = config
        .storage
        .options
        .get("path")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(path, "./tmp");
}

#[tokio::test]
async fn test_filesystem_storage_operations() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage_config = StorageConfig {
        backend: "filesystem".to_string(),
        options: {
            let mut options = std::collections::HashMap::new();
            options.insert(
                "path".to_string(),
                serde_json::Value::String(temp_dir.path().to_string_lossy().to_string()),
            );
            options
        },
    };

    let storage = create_storage_backend(&storage_config).expect("Failed to create storage");

    // Test subpath creation
    let subpath = storage.subpath_str("test/nested/file.txt");
    assert!(subpath.starts_with(storage.base_path()));
    assert!(subpath.ends_with("test/nested/file.txt"));

    // Test directory creation
    let dir_path = storage
        .ensure_dir_str("test/nested")
        .expect("Failed to ensure dir");
    assert!(dir_path.exists());
    assert!(dir_path.is_dir());

    // Check that dir_path starts with storage base and ends with the expected suffix
    assert!(dir_path.starts_with(storage.base_path()));
    assert!(dir_path.ends_with("test/nested"));

    // Test file operations
    let test_content = b"Hello, storage world!";
    let file_path = "test/data.txt";

    let written_path = storage
        .write_file_str(file_path, test_content)
        .await
        .expect("Failed to write file");

    assert!(storage.exists_str(file_path));
    assert!(written_path.exists());

    let read_content = storage
        .read_file_str(file_path)
        .await
        .expect("Failed to read file");
    assert_eq!(read_content, test_content);

    // Test removal
    storage
        .remove_str(file_path)
        .await
        .expect("Failed to remove file");
    assert!(!storage.exists_str(file_path));

    // Test directory removal
    storage
        .remove_str("test")
        .await
        .expect("Failed to remove directory");
    assert!(!storage.exists_str("test"));
}

#[test]
fn test_tempdir_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage_config = StorageConfig {
        backend: "filesystem".to_string(),
        options: {
            let mut options = std::collections::HashMap::new();
            options.insert(
                "path".to_string(),
                serde_json::Value::String(temp_dir.path().to_string_lossy().to_string()),
            );
            options
        },
    };

    let storage = create_storage_backend(&storage_config).expect("Failed to create storage");

    // Test tempdir creation
    let tempdir = storage
        .tempdir_in_str("uploads", "test_prefix_")
        .expect("Failed to create tempdir");

    // Verify the tempdir is within our expected parent
    let parent = temp_dir.path().join("uploads");
    assert!(tempdir.path().starts_with(&parent));

    // Verify the prefix is used
    let temp_name = tempdir.path().file_name().unwrap().to_string_lossy();
    assert!(temp_name.starts_with("test_prefix_"));
}

#[tokio::test]
async fn test_storage_validation_errors() {
    // Test invalid backend
    let invalid_config = StorageConfig {
        backend: "invalid_backend".to_string(),
        options: std::collections::HashMap::new(),
    };

    let result = create_storage_backend(&invalid_config);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("Unknown storage backend"));

    // Test invalid path type
    let mut invalid_path_config = StorageConfig {
        backend: "filesystem".to_string(),
        options: std::collections::HashMap::new(),
    };
    invalid_path_config.options.insert(
        "path".to_string(),
        serde_json::Value::Number(serde_json::Number::from(123)),
    );

    // This should still work as we fall back to default path
    let storage =
        create_storage_backend(&invalid_path_config).expect("Should fall back to default path");
    assert!(storage.base_path().ends_with("tmp"));
}

#[tokio::test]
async fn test_jmix_and_dimse_subdirectories() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage_config = StorageConfig {
        backend: "filesystem".to_string(),
        options: {
            let mut options = std::collections::HashMap::new();
            options.insert(
                "path".to_string(),
                serde_json::Value::String(temp_dir.path().to_string_lossy().to_string()),
            );
            options
        },
    };

    let storage = create_storage_backend(&storage_config).expect("Failed to create storage");

    // Test JMIX subdirectories
    let jmix_store = storage
        .ensure_dir_str("jmix-store")
        .expect("Failed to create jmix-store");
    assert!(jmix_store.exists());
    assert!(jmix_store.ends_with("jmix-store"));

    let jmix_upload = storage
        .ensure_dir_str("jmix-upload")
        .expect("Failed to create jmix-upload");
    assert!(jmix_upload.exists());
    assert!(jmix_upload.ends_with("jmix-upload"));

    // Test DIMSE subdirectories
    let dimse_dir = storage
        .ensure_dir_str("dimse")
        .expect("Failed to create dimse");
    assert!(dimse_dir.exists());
    assert!(dimse_dir.ends_with("dimse"));

    let dimse_uuid = storage
        .ensure_dir_str("dimse/test-uuid-123")
        .expect("Failed to create dimse/uuid");
    assert!(dimse_uuid.exists());
    assert!(dimse_uuid.ends_with("test-uuid-123"));

    // Test temp directory creation within subdirectories
    let temp_upload = storage
        .tempdir_in_str("jmix-upload", "jmix_upload_")
        .expect("Failed to create tempdir");
    let expected_parent = temp_dir.path().join("jmix-upload");
    assert!(temp_upload.path().starts_with(&expected_parent));
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::collections::HashMap;

    #[test]
    fn test_storage_config_default() {
        let config = StorageConfig::default();
        assert_eq!(config.backend, "filesystem");
        assert!(config.options.contains_key("path"));
        assert_eq!(
            config.options.get("path").unwrap().as_str().unwrap(),
            "./tmp"
        );
    }

    #[test]
    fn test_storage_config_serialization() {
        let config = StorageConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: StorageConfig = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(config.backend, deserialized.backend);
        assert_eq!(
            config.options.get("path").unwrap().as_str().unwrap(),
            deserialized.options.get("path").unwrap().as_str().unwrap()
        );
    }

    #[test]
    fn test_create_filesystem_backend() {
        let config = StorageConfig {
            backend: "filesystem".to_string(),
            options: {
                let mut map = HashMap::new();
                map.insert(
                    "path".to_string(),
                    serde_json::Value::String("./tmp/test".to_string()),
                );
                map
            },
        };

        let result = create_storage_backend(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_filesystem_backend_default_path() {
        let config = StorageConfig {
            backend: "filesystem".to_string(),
            options: HashMap::new(),
        };

        let result = create_storage_backend(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_unknown_backend() {
        let config = StorageConfig {
            backend: "unknown".to_string(),
            options: HashMap::new(),
        };

        let result = create_storage_backend(&config);
        assert!(result.is_err());
        
        if let Err(StorageError::Config(msg)) = result {
            assert!(msg.contains("Unknown storage backend"));
            assert!(msg.contains("unknown"));
        } else {
            panic!("Expected StorageError::Config");
        }
    }

    #[test]
    fn test_storage_config_with_custom_options() {
        let mut options = HashMap::new();
        options.insert(
            "path".to_string(),
            serde_json::Value::String("/custom/path".to_string()),
        );
        options.insert(
            "create_dirs".to_string(),
            serde_json::Value::Bool(true),
        );

        let config = StorageConfig {
            backend: "filesystem".to_string(),
            options,
        };

        assert_eq!(config.backend, "filesystem");
        assert_eq!(
            config.options.get("path").unwrap().as_str().unwrap(),
            "/custom/path"
        );
        assert_eq!(
            config.options.get("create_dirs").unwrap().as_bool().unwrap(),
            true
        );
    }

    #[test]
    fn test_default_backend_function() {
        let backend = default_backend();
        assert_eq!(backend, "filesystem");
    }
}

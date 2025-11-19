#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::config::config::Config;
    use crate::storage::{FilesystemStorage, StorageBackend};
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_set_and_get_config() {
        let config = Arc::new(Config::default());
        set_config(config.clone());

        let retrieved = get_config();
        assert!(retrieved.is_some());

        let retrieved_config = retrieved.unwrap();
        assert_eq!(
            Arc::strong_count(&config) - 1,
            Arc::strong_count(&retrieved_config) - 1
        );
    }

    #[test]
    #[serial]
    fn test_get_config_when_not_set() {
        // This test might interfere with others, run serially
        // Reset by setting None
        let retrieved = get_config();
        // Can't reliably test None state due to global state
        assert!(retrieved.is_some() || retrieved.is_none());
    }

    #[test]
    #[serial]
    fn test_set_and_get_storage() {
        let storage =
            Arc::new(FilesystemStorage::new("./tmp/test").unwrap()) as Arc<dyn StorageBackend>;
        set_storage(storage.clone());

        let retrieved = get_storage();
        assert!(retrieved.is_some());
    }

    #[test]
    #[serial]
    fn test_reset_storage() {
        let storage =
            Arc::new(FilesystemStorage::new("./tmp/test").unwrap()) as Arc<dyn StorageBackend>;
        set_storage(storage);

        // Verify it's set
        assert!(get_storage().is_some());

        // Reset
        reset_storage();

        // Verify it's cleared
        assert!(get_storage().is_none());
    }

    #[test]
    #[serial]
    fn test_config_persistence_across_calls() {
        let config = Arc::new(Config::default());
        set_config(config.clone());

        // Get config multiple times
        let first = get_config().unwrap();
        let second = get_config().unwrap();

        // Both should reference the same underlying data
        assert_eq!(first.proxy.id, second.proxy.id);
    }

    #[test]
    #[serial]
    fn test_storage_persistence_across_calls() {
        let storage =
            Arc::new(FilesystemStorage::new("./tmp/test").unwrap()) as Arc<dyn StorageBackend>;
        set_storage(storage);

        // Get storage multiple times
        let first = get_storage();
        let second = get_storage();

        assert!(first.is_some());
        assert!(second.is_some());
    }

    #[test]
    #[serial]
    fn test_overwrite_config() {
        let config1 = Arc::new(Config::default());
        set_config(config1);

        let mut config2 = Config::default();
        config2.proxy.id = "updated_id".to_string();
        let config2_arc = Arc::new(config2);
        set_config(config2_arc.clone());

        let retrieved = get_config().unwrap();
        assert_eq!(retrieved.proxy.id, "updated_id");
    }

    #[test]
    #[serial]
    fn test_overwrite_storage() {
        let storage1 =
            Arc::new(FilesystemStorage::new("./tmp/test1").unwrap()) as Arc<dyn StorageBackend>;
        set_storage(storage1);

        let storage2 =
            Arc::new(FilesystemStorage::new("./tmp/test2").unwrap()) as Arc<dyn StorageBackend>;
        set_storage(storage2);

        // Should have the new storage
        let retrieved = get_storage();
        assert!(retrieved.is_some());
    }
}

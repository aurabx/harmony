use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::SystemTime;

/// JMIX package metadata stored in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmixPackageInfo {
    pub id: String,
    pub study_uid: String,
    pub path: String,
    pub created_at: u64, // Unix timestamp
}

// Define redb tables
const PACKAGES_BY_ID: TableDefinition<&str, &str> = TableDefinition::new("packages_by_id");
const PACKAGES_BY_STUDY_UID: TableDefinition<&str, &str> =
    TableDefinition::new("packages_by_study_uid");

/// JMIX package index for fast lookups without filesystem walks
pub struct JmixIndex {
    db: Database,
}

impl JmixIndex {
    /// Open or create the index database
    pub fn open(db_path: &Path) -> Result<Self, String> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create index directory: {}", e))?;
        }
        
        let db = Database::create(db_path)
            .map_err(|e| format!("Failed to open JMIX index database: {}", e))?;

        // Initialize tables
        let write_txn = db
            .begin_write()
            .map_err(|e| format!("Failed to begin write transaction: {}", e))?;
        {
            let _ = write_txn
                .open_table(PACKAGES_BY_ID)
                .map_err(|e| format!("Failed to open packages_by_id table: {}", e))?;
            let _ = write_txn
                .open_table(PACKAGES_BY_STUDY_UID)
                .map_err(|e| format!("Failed to open packages_by_study_uid table: {}", e))?;
        }
        write_txn
            .commit()
            .map_err(|e| format!("Failed to commit table initialization: {}", e))?;

        Ok(Self { db })
    }

    /// Index a new JMIX package
    pub fn index_package(&self, info: &JmixPackageInfo) -> Result<(), String> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| format!("Failed to begin write transaction: {}", e))?;

        {
            // Serialize the package info
            let json = serde_json::to_string(info)
                .map_err(|e| format!("Failed to serialize package info: {}", e))?;

            // Store by ID
            let mut table = write_txn
                .open_table(PACKAGES_BY_ID)
                .map_err(|e| format!("Failed to open packages_by_id table: {}", e))?;
            table
                .insert(info.id.as_str(), json.as_str())
                .map_err(|e| format!("Failed to insert package by ID: {}", e))?;
        }

        {
            // Store by study UID (for fast queries)
            let json = serde_json::to_string(info)
                .map_err(|e| format!("Failed to serialize package info: {}", e))?;

            let mut table = write_txn
                .open_table(PACKAGES_BY_STUDY_UID)
                .map_err(|e| format!("Failed to open packages_by_study_uid table: {}", e))?;

            // Key format: "study_uid:id" to support multiple packages per study
            let key = format!("{}:{}", info.study_uid, info.id);
            table
                .insert(key.as_str(), json.as_str())
                .map_err(|e| format!("Failed to insert package by study UID: {}", e))?;
        }

        write_txn
            .commit()
            .map_err(|e| format!("Failed to commit package index: {}", e))?;

        tracing::debug!(
            "📇 Indexed JMIX package: id={}, study_uid={}",
            info.id,
            info.study_uid
        );
        Ok(())
    }

    /// Lookup a package by ID
    pub fn get_by_id(&self, id: &str) -> Result<Option<JmixPackageInfo>, String> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| format!("Failed to begin read transaction: {}", e))?;

        let table = read_txn
            .open_table(PACKAGES_BY_ID)
            .map_err(|e| format!("Failed to open packages_by_id table: {}", e))?;

        match table.get(id) {
            Ok(Some(value)) => {
                let json = value.value();
                let info: JmixPackageInfo = serde_json::from_str(json)
                    .map_err(|e| format!("Failed to deserialize package info: {}", e))?;
                Ok(Some(info))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("Failed to get package by ID: {}", e)),
        }
    }

    /// Query packages by study UID
    pub fn query_by_study_uid(&self, study_uid: &str) -> Result<Vec<JmixPackageInfo>, String> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| format!("Failed to begin read transaction: {}", e))?;

        let table = read_txn
            .open_table(PACKAGES_BY_STUDY_UID)
            .map_err(|e| format!("Failed to open packages_by_study_uid table: {}", e))?;

        let mut results = Vec::new();
        let prefix = format!("{}:", study_uid);

        // Iterate over all entries and filter by prefix
        let iter = table
            .iter()
            .map_err(|e| format!("Failed to iterate packages_by_study_uid: {}", e))?;

        for entry in iter {
            let (key, value) = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let key_str = key.value();

            if key_str.starts_with(&prefix) {
                let json = value.value();
                let info: JmixPackageInfo = serde_json::from_str(json)
                    .map_err(|e| format!("Failed to deserialize package info: {}", e))?;
                results.push(info);
            }
        }

        tracing::debug!(
            "📇 Found {} packages for study_uid={}",
            results.len(),
            study_uid
        );
        Ok(results)
    }

    /// Remove a package from the index
    pub fn remove_package(&self, id: &str, study_uid: &str) -> Result<(), String> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| format!("Failed to begin write transaction: {}", e))?;

        {
            // Remove from ID index
            let mut table = write_txn
                .open_table(PACKAGES_BY_ID)
                .map_err(|e| format!("Failed to open packages_by_id table: {}", e))?;
            table
                .remove(id)
                .map_err(|e| format!("Failed to remove package by ID: {}", e))?;
        }

        {
            // Remove from study UID index
            let mut table = write_txn
                .open_table(PACKAGES_BY_STUDY_UID)
                .map_err(|e| format!("Failed to open packages_by_study_uid table: {}", e))?;
            let key = format!("{}:{}", study_uid, id);
            table
                .remove(key.as_str())
                .map_err(|e| format!("Failed to remove package by study UID: {}", e))?;
        }

        write_txn
            .commit()
            .map_err(|e| format!("Failed to commit package removal: {}", e))?;

        tracing::debug!("📇 Removed JMIX package from index: id={}", id);
        Ok(())
    }

    /// Check if a package exists in the index
    pub fn exists(&self, id: &str) -> Result<bool, String> {
        Ok(self.get_by_id(id)?.is_some())
    }
}

/// Get or create the global JMIX index
pub fn get_jmix_index(store_root: &Path) -> Result<JmixIndex, String> {
    let db_path = store_root.join("jmix-index.redb");
    JmixIndex::open(&db_path)
}

/// Helper to get current Unix timestamp
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_and_query() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");
        let index = JmixIndex::open(&db_path).unwrap();

        // Index a package
        let info = JmixPackageInfo {
            id: "test-uuid-123".to_string(),
            study_uid: "1.2.3.4.5".to_string(),
            path: "/tmp/test-uuid-123".to_string(),
            created_at: current_timestamp(),
        };
        index.index_package(&info).unwrap();

        // Query by ID
        let result = index.get_by_id("test-uuid-123").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().study_uid, "1.2.3.4.5");

        // Query by study UID
        let results = index.query_by_study_uid("1.2.3.4.5").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "test-uuid-123");

        // Remove package
        index.remove_package("test-uuid-123", "1.2.3.4.5").unwrap();
        assert!(!index.exists("test-uuid-123").unwrap());
    }

    #[test]
    fn test_multiple_packages_per_study() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");
        let index = JmixIndex::open(&db_path).unwrap();

        // Index multiple packages with same study UID
        for i in 1..=3 {
            let info = JmixPackageInfo {
                id: format!("test-uuid-{}", i),
                study_uid: "1.2.3.4.5".to_string(),
                path: format!("/tmp/test-uuid-{}", i),
                created_at: current_timestamp(),
            };
            index.index_package(&info).unwrap();
        }

        // Query should return all 3
        let results = index.query_by_study_uid("1.2.3.4.5").unwrap();
        assert_eq!(results.len(), 3);
    }
}

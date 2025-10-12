use once_cell::sync::OnceCell;
use redb::{Database, TableDefinition};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Global database manager for handling shared database instances
/// This prevents database lock contention by reusing database instances per path
static GLOBAL_DB_MANAGER: OnceCell<DatabaseManager> = OnceCell::new();

/// Database manager that maintains shared database instances
/// Maps database file paths to their respective Arc<Database> instances
pub struct DatabaseManager {
    databases: Mutex<HashMap<PathBuf, Arc<Database>>>,
}

impl DatabaseManager {
    /// Create a new database manager
    fn new() -> Self {
        Self {
            databases: Mutex::new(HashMap::new()),
        }
    }

    /// Get the global database manager instance
    pub fn global() -> &'static DatabaseManager {
        GLOBAL_DB_MANAGER.get_or_init(DatabaseManager::new)
    }

    /// Get or create a shared database instance for a specific path
    /// This is the main method to prevent database lock contention
    pub fn get_or_create_database(&self, db_path: &Path) -> Result<Arc<Database>, String> {
        let db_path_buf = db_path.to_path_buf();

        // Lock the map and check if we already have a database for this path
        let mut map = self
            .databases
            .lock()
            .map_err(|e| format!("Failed to lock database map: {}", e))?;

        if let Some(existing_db) = map.get(&db_path_buf) {
            // Return existing database instance for this path
            tracing::debug!(
                "🔄 Reusing existing database instance for: {}",
                db_path_buf.display()
            );
            Ok(existing_db.clone())
        } else {
            // Create new database instance for this path
            tracing::debug!(
                "🆕 Creating new database instance for: {}",
                db_path_buf.display()
            );
            let db = self.create_database(&db_path_buf)?;
            map.insert(db_path_buf, db.clone());
            Ok(db)
        }
    }

    /// Create a new database instance with proper initialization
    fn create_database(&self, db_path: &Path) -> Result<Arc<Database>, String> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create database directory: {}", e))?;
        }

        tracing::info!("🗄️  Initializing shared database: {}", db_path.display());

        let db =
            Database::create(db_path).map_err(|e| format!("Failed to create database: {}", e))?;

        tracing::info!("✅ Database initialized successfully");
        Ok(Arc::new(db))
    }

    /// Initialize tables in a database (helper method for specific implementations)
    pub fn initialize_tables<'a>(
        &self,
        db: &Database,
        table_definitions: &[&'a TableDefinition<&'static str, &'static str>],
    ) -> Result<(), String> {
        let write_txn = db
            .begin_write()
            .map_err(|e| format!("Failed to begin write transaction: {}", e))?;

        // Initialize all provided table definitions
        for (i, table_def) in table_definitions.iter().enumerate() {
            let _ = write_txn
                .open_table(**table_def)
                .map_err(|e| format!("Failed to open table {}: {}", i, e))?;
        }

        write_txn
            .commit()
            .map_err(|e| format!("Failed to commit table initialization: {}", e))?;

        tracing::debug!("✅ Initialized {} tables", table_definitions.len());
        Ok(())
    }

    /// Get database statistics for monitoring/debugging
    pub fn get_database_stats(&self) -> Result<DatabaseStats, String> {
        let map = self
            .databases
            .lock()
            .map_err(|e| format!("Failed to lock database map: {}", e))?;

        Ok(DatabaseStats {
            total_databases: map.len(),
            database_paths: map.keys().cloned().collect(),
        })
    }

    /// Close a specific database (useful for cleanup or testing)
    pub fn close_database(&self, db_path: &Path) -> Result<bool, String> {
        let mut map = self
            .databases
            .lock()
            .map_err(|e| format!("Failed to lock database map: {}", e))?;

        let removed = map.remove(&db_path.to_path_buf()).is_some();
        if removed {
            tracing::info!("🗑️  Closed database: {}", db_path.display());
        }
        Ok(removed)
    }

    /// Clear all databases (primarily for testing)
    #[cfg(test)]
    pub fn clear_all_databases(&self) -> Result<usize, String> {
        let mut map = self
            .databases
            .lock()
            .map_err(|e| format!("Failed to lock database map: {}", e))?;

        let count = map.len();
        map.clear();
        tracing::info!("🧹 Cleared {} database instances", count);
        Ok(count)
    }
}

/// Database statistics for monitoring
#[derive(Debug, Clone)]
pub struct DatabaseStats {
    pub total_databases: usize,
    pub database_paths: Vec<PathBuf>,
}

/// Trait for components that use database functionality
/// This provides a standard interface for database-backed services
pub trait DatabaseBackend {
    /// Get the database path this backend uses
    fn database_path(&self) -> PathBuf;

    /// Get the shared database instance
    fn get_database(&self) -> Result<Arc<Database>, String> {
        DatabaseManager::global().get_or_create_database(&self.database_path())
    }

    /// Initialize any required tables (should be implemented by each backend)
    fn initialize_tables(&self, db: &Database) -> Result<(), String>;
}

/// Wrapper for database operations with error handling
pub struct DatabaseOperation;

impl DatabaseOperation {
    /// Execute a read operation with proper error handling
    pub fn read<F, R>(db: &Database, operation: F) -> Result<R, String>
    where
        F: FnOnce(&redb::ReadTransaction) -> Result<R, String>,
    {
        let read_txn = db
            .begin_read()
            .map_err(|e| format!("Failed to begin read transaction: {}", e))?;

        operation(&read_txn)
    }

    /// Execute a write operation with proper error handling
    pub fn write<F, R>(db: &Database, operation: F) -> Result<R, String>
    where
        F: FnOnce(&redb::WriteTransaction) -> Result<R, String>,
    {
        let write_txn = db
            .begin_write()
            .map_err(|e| format!("Failed to begin write transaction: {}", e))?;

        let result = operation(&write_txn)?;

        write_txn
            .commit()
            .map_err(|e| format!("Failed to commit write transaction: {}", e))?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_database_manager_singleton() {
        let manager1 = DatabaseManager::global();
        let manager2 = DatabaseManager::global();

        // Should be the same instance (singleton pattern)
        assert!(std::ptr::eq(manager1, manager2));
    }

    #[test]
    fn test_database_creation_and_reuse() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");

        let manager = DatabaseManager::global();

        // First call should create a new database
        let db1 = manager.get_or_create_database(&db_path).unwrap();

        // Second call should return the same instance
        let db2 = manager.get_or_create_database(&db_path).unwrap();

        // Should be the same Arc instance
        assert!(Arc::ptr_eq(&db1, &db2));

        // Clean up for other tests
        let _ = manager.close_database(&db_path);
    }

    #[test]
    fn test_multiple_database_paths() {
        let temp_dir = TempDir::new().unwrap();
        let db_path1 = temp_dir.path().join("test1.redb");
        let db_path2 = temp_dir.path().join("test2.redb");

        let manager = DatabaseManager::global();

        let db1 = manager.get_or_create_database(&db_path1).unwrap();
        let db2 = manager.get_or_create_database(&db_path2).unwrap();

        // Should be different instances for different paths
        assert!(!Arc::ptr_eq(&db1, &db2));

        // But same path should return same instance
        let db1_again = manager.get_or_create_database(&db_path1).unwrap();
        assert!(Arc::ptr_eq(&db1, &db1_again));

        // Clean up
        let _ = manager.close_database(&db_path1);
        let _ = manager.close_database(&db_path2);
    }

    #[test]
    fn test_database_stats() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("stats_test.redb");

        let manager = DatabaseManager::global();
        let initial_stats = manager.get_database_stats().unwrap();
        let initial_count = initial_stats.total_databases;

        // Create a database
        let _db = manager.get_or_create_database(&db_path).unwrap();

        let stats = manager.get_database_stats().unwrap();
        assert_eq!(stats.total_databases, initial_count + 1);
        assert!(stats.database_paths.contains(&db_path));

        // Close the database
        manager.close_database(&db_path).unwrap();

        let final_stats = manager.get_database_stats().unwrap();
        assert_eq!(final_stats.total_databases, initial_count);
    }

    #[test]
    #[cfg(test)]
    fn test_clear_all_databases() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("clear_test.redb");

        let manager = DatabaseManager::global();

        // Create a database
        let _db = manager.get_or_create_database(&db_path).unwrap();

        // Clear all databases
        let cleared_count = manager.clear_all_databases().unwrap();
        assert!(cleared_count > 0);

        // Stats should show no databases
        let stats = manager.get_database_stats().unwrap();
        assert_eq!(stats.total_databases, 0);
    }
}

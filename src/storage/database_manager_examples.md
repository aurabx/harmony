# Database Manager - Usage Examples

The generalized `DatabaseManager` provides a clean abstraction for managing shared redb database instances across the application, preventing database lock contention issues.

## Key Features

- **Shared Database Instances**: Reuses database instances per file path to prevent lock contention
- **Thread-Safe**: Uses `Mutex` and `Arc` for safe concurrent access
- **Path-Based Isolation**: Different database files get their own instances
- **Automatic Cleanup**: Provides methods for database lifecycle management
- **Monitoring**: Built-in statistics and debugging capabilities

## Basic Usage

### 1. Simple Database Access

```rust
use crate::storage::DatabaseManager;
use std::path::PathBuf;

// Get a shared database instance
let db_path = PathBuf::from("./data/my-database.redb");
let db = DatabaseManager::global().get_or_create_database(&db_path)?;

// Use the database for operations
// Multiple calls with the same path will return the same Arc<Database>
```

### 2. Using the DatabaseBackend Trait

```rust
use crate::storage::{DatabaseBackend, DatabaseManager, DatabaseOperation};
use redb::{Database, TableDefinition};
use std::path::PathBuf;
use std::sync::Arc;

// Define your table schemas
const USERS: TableDefinition<&str, &str> = TableDefinition::new("users");
const SESSIONS: TableDefinition<&str, &str> = TableDefinition::new("sessions");

struct UserDatabase {
    db: Arc<Database>,
    db_path: PathBuf,
}

impl UserDatabase {
    pub fn new(db_path: PathBuf) -> Result<Self, String> {
        let db = DatabaseManager::global().get_or_create_database(&db_path)?;
        let instance = Self { db: db.clone(), db_path };
        instance.initialize_tables(&db)?;
        Ok(instance)
    }
}

impl DatabaseBackend for UserDatabase {
    fn database_path(&self) -> PathBuf {
        self.db_path.clone()
    }
    
    fn initialize_tables(&self, db: &Database) -> Result<(), String> {
        let table_definitions = &[&USERS, &SESSIONS];
        DatabaseManager::global().initialize_tables(db, table_definitions)
    }
}

// Usage
let user_db = UserDatabase::new(PathBuf::from("./data/users.redb"))?;
let db = user_db.get_database()?; // Get shared database instance
```

### 3. Using DatabaseOperation for Safe Transactions

```rust
use crate::storage::DatabaseOperation;

// Safe read operation
let user_data = DatabaseOperation::read(&db, |read_txn| {
    let table = read_txn.open_table(USERS)?;
    match table.get("user123")? {
        Some(value) => Ok(Some(value.value().to_string())),
        None => Ok(None),
    }
})?;

// Safe write operation
DatabaseOperation::write(&db, |write_txn| {
    let mut table = write_txn.open_table(USERS)?;
    table.insert("user123", "{\"name\":\"John\",\"email\":\"john@example.com\"}")?;
    Ok(())
})?;
```

## Advanced Usage

### 4. Database Statistics and Monitoring

```rust
let manager = DatabaseManager::global();

// Get statistics about all databases
let stats = manager.get_database_stats()?;
println!("Total databases: {}", stats.total_databases);
println!("Database paths: {:?}", stats.database_paths);

// Close a specific database when no longer needed
manager.close_database(&db_path)?;
```

### 5. Complete Example: Session Store

```rust
use crate::storage::{DatabaseBackend, DatabaseManager, DatabaseOperation};
use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

const SESSIONS: TableDefinition<&str, &str> = TableDefinition::new("sessions");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub created_at: u64,
    pub expires_at: u64,
}

pub struct SessionStore {
    db: Arc<Database>,
    db_path: PathBuf,
}

impl SessionStore {
    pub fn new(store_root: &Path) -> Result<Self, String> {
        let db_path = store_root.join("sessions.redb");
        let db = DatabaseManager::global().get_or_create_database(&db_path)?;
        
        let instance = Self { db: db.clone(), db_path };
        instance.initialize_tables(&db)?;
        
        Ok(instance)
    }
    
    pub fn create_session(&self, session: &Session) -> Result<(), String> {
        let session_json = serde_json::to_string(session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;
            
        DatabaseOperation::write(&self.db, |write_txn| {
            let mut table = write_txn.open_table(SESSIONS)
                .map_err(|e| format!("Failed to open sessions table: {}", e))?;
            table.insert(session.id.as_str(), session_json.as_str())
                .map_err(|e| format!("Failed to insert session: {}", e))?;
            Ok(())
        })
    }
    
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>, String> {
        DatabaseOperation::read(&self.db, |read_txn| {
            let table = read_txn.open_table(SESSIONS)
                .map_err(|e| format!("Failed to open sessions table: {}", e))?;
                
            match table.get(session_id) {
                Ok(Some(value)) => {
                    let session: Session = serde_json::from_str(value.value())
                        .map_err(|e| format!("Failed to deserialize session: {}", e))?;
                    Ok(Some(session))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(format!("Failed to get session: {}", e)),
            }
        })
    }
    
    pub fn delete_session(&self, session_id: &str) -> Result<bool, String> {
        DatabaseOperation::write(&self.db, |write_txn| {
            let mut table = write_txn.open_table(SESSIONS)
                .map_err(|e| format!("Failed to open sessions table: {}", e))?;
            match table.remove(session_id) {
                Ok(Some(_)) => Ok(true),
                Ok(None) => Ok(false),
                Err(e) => Err(format!("Failed to delete session: {}", e)),
            }
        })
    }
}

impl DatabaseBackend for SessionStore {
    fn database_path(&self) -> PathBuf {
        self.db_path.clone()
    }
    
    fn initialize_tables(&self, db: &Database) -> Result<(), String> {
        let table_definitions = &[&SESSIONS];
        DatabaseManager::global().initialize_tables(db, table_definitions)
    }
}

// Usage
let session_store = SessionStore::new(Path::new("./data"))?;
let session = Session {
    id: "sess_123".to_string(),
    user_id: "user_456".to_string(),
    created_at: 1640995200, // timestamp
    expires_at: 1641081600, // timestamp + 24h
};

session_store.create_session(&session)?;
let retrieved = session_store.get_session("sess_123")?;
println!("Retrieved session: {:?}", retrieved);
```

## Testing with the Database Manager

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_session_store() {
        let temp_dir = TempDir::new().unwrap();
        let store = SessionStore::new(temp_dir.path()).unwrap();
        
        let session = Session {
            id: "test_session".to_string(),
            user_id: "test_user".to_string(),
            created_at: 1640995200,
            expires_at: 1641081600,
        };
        
        // Create session
        store.create_session(&session).unwrap();
        
        // Retrieve session
        let retrieved = store.get_session("test_session").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().user_id, "test_user");
        
        // Delete session
        let deleted = store.delete_session("test_session").unwrap();
        assert!(deleted);
        
        // Verify deletion
        let not_found = store.get_session("test_session").unwrap();
        assert!(not_found.is_none());
    }
}
```

## Benefits

1. **No Database Lock Contention**: Multiple threads can safely access the same database
2. **Resource Efficiency**: Database instances are shared and reused
3. **Clean API**: Simple, consistent interface for database operations
4. **Error Handling**: Proper error propagation and transaction management
5. **Testability**: Easy to test with temporary directories and isolated instances
6. **Monitoring**: Built-in statistics for debugging and performance monitoring

## Migration from Direct redb Usage

If you have existing code that uses redb directly:

**Before:**
```rust
let db = Database::create("./data/my-db.redb")?;
let write_txn = db.begin_write()?;
// ... database operations
write_txn.commit()?;
```

**After:**
```rust
let db = DatabaseManager::global().get_or_create_database(Path::new("./data/my-db.redb"))?;
DatabaseOperation::write(&db, |write_txn| {
    // ... same database operations
    Ok(())
})?;
```

The new approach provides better error handling, automatic transaction management, and prevents database lock contention issues.
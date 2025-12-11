//! Reader connection pool using try_clone().
//!
//! DuckDB connections created via `try_clone()` share the same underlying database instance,
//! enabling readers to see writes immediately without waiting for WAL checkpoint.

use std::sync::{Arc, Mutex};

use duckdb::Connection;

use crate::storage::StorageError;

/// Connection pool for concurrent read operations.
///
/// Uses `try_clone()` from a parent connection to ensure all connections share
/// the same DuckDB database instance, providing immediate write visibility.
pub struct ReadPool {
    /// Parent connection used for cloning.
    parent: Mutex<Connection>,
}

impl ReadPool {
    /// Create a new read pool from a parent connection.
    ///
    /// The parent connection should be cloned from the writer's connection
    /// to ensure they share the same database instance.
    pub fn new(parent_conn: Connection) -> Arc<Self> {
        Arc::new(Self {
            parent: Mutex::new(parent_conn),
        })
    }

    /// Get a connection from the pool.
    ///
    /// Each call creates a new cloned connection that shares the same
    /// underlying database instance as the writer.
    pub fn get(&self) -> Result<Connection, StorageError> {
        let parent = self.parent.lock().map_err(|e| {
            StorageError::Internal(format!("Failed to lock parent connection: {}", e))
        })?;
        parent.try_clone().map_err(StorageError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::init_schema;
    use tempfile::tempdir;

    #[test]
    fn test_pool_creation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create the database file first with schema
        let parent_conn = Connection::open(&db_path).unwrap();
        init_schema(&parent_conn).unwrap();

        // Create pool from parent connection
        let pool = ReadPool::new(parent_conn);
        let conn = pool.get().unwrap();

        // Verify we can execute queries
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'metric_series'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}

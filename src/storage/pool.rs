//! Reader connection pool using r2d2.

use std::path::Path;
use std::sync::Arc;

use duckdb::DuckdbConnectionManager;
use r2d2::{Pool, PooledConnection};

use crate::storage::StorageError;

/// Connection pool for concurrent read operations.
pub struct ReadPool {
    pool: Pool<DuckdbConnectionManager>,
}

impl ReadPool {
    /// Create a new read pool.
    ///
    /// Note: Schema is expected to be initialized by the writer actor before this is called.
    pub fn new(db_path: &Path, size: u32) -> Result<Arc<Self>, StorageError> {
        let manager = DuckdbConnectionManager::file(db_path)?;
        let pool = Pool::builder().max_size(size).build(manager)?;

        Ok(Arc::new(Self { pool }))
    }

    /// Get a connection from the pool.
    pub fn get(&self) -> Result<PooledConnection<DuckdbConnectionManager>, StorageError> {
        Ok(self.pool.get()?)
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
        let conn = duckdb::Connection::open(&db_path).unwrap();
        init_schema(&conn).unwrap();
        drop(conn);

        let pool = ReadPool::new(&db_path, 4).unwrap();
        let conn = pool.get().unwrap();

        // Verify we can execute queries
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'metrics'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}

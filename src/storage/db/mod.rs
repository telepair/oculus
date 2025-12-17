//! Database abstraction layer for multi-backend support.
//!
//! Currently supports SQLite. Designed for future PostgreSQL extension.
//!
//! # Architecture
//!
//! The abstraction is intentionally minimal to avoid over-engineering:
//! - `SqlitePool`: Connection pool wrapper for SQLite
//! - Future: `PostgresPool` with the same interface pattern
//!
//! # Example
//!
//! ```ignore
//! let pool = SqlitePool::connect("sqlite:data/oculus.db?mode=rwc").await?;
//! let row = sqlx::query("SELECT 1").fetch_one(pool.inner()).await?;
//! ```

mod sqlite;

pub use sqlite::SqlitePool;

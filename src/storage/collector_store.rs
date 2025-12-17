//! Collector storage and synchronization.
//!
//! Provides CRUD operations for collector configurations and
//! sync logic for config-file collectors on startup.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::Row;
use strum_macros::{AsRefStr, Display, EnumString};

use crate::storage::StorageError;
use crate::storage::db::SqlitePool;

// =============================================================================
// Types
// =============================================================================

/// Collector type classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum CollectorType {
    /// TCP port probe collector.
    Tcp,
    /// ICMP ping probe collector.
    Ping,
    /// HTTP endpoint probe collector.
    Http,
}

/// Collector source classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, Display, AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum CollectorSource {
    /// From configuration file.
    Config,
    /// Created via API.
    Api,
}

/// Collector record stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorRecord {
    /// Database ID (None for new records).
    pub id: Option<i64>,
    /// Collector type.
    pub collector_type: CollectorType,
    /// Unique name within type.
    pub name: String,
    /// Source of this collector.
    pub source: CollectorSource,
    /// Whether the collector is enabled.
    pub enabled: bool,
    /// Collector group name.
    #[serde(rename = "group")]
    pub group_name: String,
    /// JSON-serialized configuration.
    pub config: serde_json::Value,
    /// Creation timestamp (Unix millis).
    pub created_at: i64,
    /// Last update timestamp (Unix millis).
    pub updated_at: i64,
}

impl CollectorRecord {
    /// Create a new collector record from config.
    pub fn from_config(
        collector_type: CollectorType,
        name: impl Into<String>,
        enabled: bool,
        group_name: impl Into<String>,
        config: serde_json::Value,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: None,
            collector_type,
            name: name.into(),
            source: CollectorSource::Config,
            enabled,
            group_name: group_name.into(),
            config,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new collector record from API.
    pub fn from_api(
        collector_type: CollectorType,
        name: impl Into<String>,
        enabled: bool,
        group_name: impl Into<String>,
        config: serde_json::Value,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: None,
            collector_type,
            name: name.into(),
            source: CollectorSource::Api,
            enabled,
            group_name: group_name.into(),
            config,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Sync result for config-file collectors.
#[derive(Debug, Default)]
pub struct SyncResult {
    /// Number of collectors added.
    pub added: usize,
    /// Number of collectors updated.
    pub updated: usize,
    /// Number of stale collectors deleted.
    pub deleted: usize,
}

// =============================================================================
// Collector Store
// =============================================================================

/// Collector storage facade for CRUD operations.
#[derive(Clone)]
pub struct CollectorStore {
    pool: Arc<SqlitePool>,
}

impl CollectorStore {
    /// Create a new collector store.
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Upsert a collector record.
    ///
    /// Returns the record ID.
    pub async fn upsert(&self, record: &CollectorRecord) -> Result<i64, StorageError> {
        let now = chrono::Utc::now().timestamp_millis();
        let config_json = record.config.to_string();

        // Use INSERT OR REPLACE for upsert
        // First try to get existing ID
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM collectors WHERE type = ? AND name = ?")
                .bind(record.collector_type.as_ref())
                .bind(&record.name)
                .fetch_optional(self.pool.inner())
                .await?;

        if let Some((id,)) = existing {
            // Update existing
            sqlx::query(
                "UPDATE collectors SET enabled = ?, group_name = ?, config = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(record.enabled)
            .bind(&record.group_name)
            .bind(&config_json)
            .bind(now)
            .bind(id)
            .execute(self.pool.inner())
            .await?;
            Ok(id)
        } else {
            // Insert new
            let result = sqlx::query(
                "INSERT INTO collectors (type, name, source, enabled, group_name, config, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(record.collector_type.as_ref())
            .bind(&record.name)
            .bind(record.source.as_ref())
            .bind(record.enabled)
            .bind(&record.group_name)
            .bind(&config_json)
            .bind(now)
            .bind(now)
            .execute(self.pool.inner())
            .await?;
            Ok(result.last_insert_rowid())
        }
    }

    /// Insert a collector record only if it doesn't already exist.
    ///
    /// Returns Some(id) if inserted, None if already exists.
    /// Used for startup sync from include directory.
    pub async fn insert_if_not_exists(
        &self,
        record: &CollectorRecord,
    ) -> Result<Option<i64>, StorageError> {
        // Check if exists
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM collectors WHERE type = ? AND name = ?")
                .bind(record.collector_type.as_ref())
                .bind(&record.name)
                .fetch_optional(self.pool.inner())
                .await?;

        if existing.is_some() {
            // Already exists, skip
            return Ok(None);
        }

        // Insert new
        let now = chrono::Utc::now().timestamp_millis();
        let config_json = record.config.to_string();

        let result = sqlx::query(
            "INSERT INTO collectors (type, name, source, enabled, group_name, config, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(record.collector_type.as_ref())
        .bind(&record.name)
        .bind(record.source.as_ref())
        .bind(record.enabled)
        .bind(&record.group_name)
        .bind(&config_json)
        .bind(now)
        .bind(now)
        .execute(self.pool.inner())
        .await?;

        Ok(Some(result.last_insert_rowid()))
    }

    /// Delete a collector by type and name.
    pub async fn delete(
        &self,
        collector_type: CollectorType,
        name: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM collectors WHERE type = ? AND name = ?")
            .bind(collector_type.as_ref())
            .bind(name)
            .execute(self.pool.inner())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List collectors by source.
    pub async fn list_by_source(
        &self,
        source: CollectorSource,
    ) -> Result<Vec<CollectorRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, type, name, source, enabled, group_name, config, created_at, updated_at
             FROM collectors WHERE source = ? ORDER BY type, name",
        )
        .bind(source.as_ref())
        .fetch_all(self.pool.inner())
        .await?;

        let records = rows
            .into_iter()
            .map(|row| {
                let type_str: String = row.get(1);
                let source_str: String = row.get(3);
                let config_str: String = row.get(6);

                CollectorRecord {
                    id: Some(row.get(0)),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2),
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get::<i32, _>(4) != 0,
                    group_name: row.get(5),
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7),
                    updated_at: row.get(8),
                }
            })
            .collect();

        Ok(records)
    }

    /// List all collectors.
    pub async fn list_all(&self) -> Result<Vec<CollectorRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, type, name, source, enabled, group_name, config, created_at, updated_at
             FROM collectors ORDER BY type, name",
        )
        .fetch_all(self.pool.inner())
        .await?;

        let records = rows
            .into_iter()
            .map(|row| {
                let type_str: String = row.get(1);
                let source_str: String = row.get(3);
                let config_str: String = row.get(6);

                CollectorRecord {
                    id: Some(row.get(0)),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2),
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get::<i32, _>(4) != 0,
                    group_name: row.get(5),
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7),
                    updated_at: row.get(8),
                }
            })
            .collect();

        Ok(records)
    }

    /// List collectors with pagination.
    ///
    /// Returns (collectors, total_count).
    pub async fn list_paginated(
        &self,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<CollectorRecord>, u64), StorageError> {
        let offset = (page.saturating_sub(1)) * page_size;

        // Get total count
        let count_row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM collectors")
            .fetch_one(self.pool.inner())
            .await?;
        let total_count = count_row.0 as u64;

        // Get paginated records
        let rows = sqlx::query(
            "SELECT id, type, name, source, enabled, group_name, config, created_at, updated_at
             FROM collectors ORDER BY type, name LIMIT ? OFFSET ?",
        )
        .bind(page_size as i64)
        .bind(offset as i64)
        .fetch_all(self.pool.inner())
        .await?;

        let records = rows
            .into_iter()
            .map(|row| {
                let type_str: String = row.get(1);
                let source_str: String = row.get(3);
                let config_str: String = row.get(6);

                CollectorRecord {
                    id: Some(row.get(0)),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2),
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get::<i32, _>(4) != 0,
                    group_name: row.get(5),
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7),
                    updated_at: row.get(8),
                }
            })
            .collect();

        Ok((records, total_count))
    }

    /// Get a collector by type and name.
    pub async fn get(
        &self,
        collector_type: CollectorType,
        name: &str,
    ) -> Result<Option<CollectorRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id, type, name, source, enabled, group_name, config, created_at, updated_at
             FROM collectors WHERE type = ? AND name = ?",
        )
        .bind(collector_type.as_ref())
        .bind(name)
        .fetch_optional(self.pool.inner())
        .await?;

        Ok(row.map(|row| {
            let type_str: String = row.get(1);
            let source_str: String = row.get(3);
            let config_str: String = row.get(6);

            CollectorRecord {
                id: Some(row.get(0)),
                collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                name: row.get(2),
                source: source_str.parse().unwrap_or(CollectorSource::Config),
                enabled: row.get::<i32, _>(4) != 0,
                group_name: row.get(5),
                config: serde_json::from_str(&config_str).unwrap_or_default(),
                created_at: row.get(7),
                updated_at: row.get(8),
            }
        }))
    }

    /// Sync config-file collectors.
    ///
    /// - Upserts all provided config collectors
    /// - Deletes stale config collectors not in the provided list
    pub async fn sync_from_config(
        &self,
        configs: Vec<CollectorRecord>,
    ) -> Result<SyncResult, StorageError> {
        let mut result = SyncResult::default();

        // Get existing config collectors
        let existing = self.list_by_source(CollectorSource::Config).await?;
        let existing_keys: std::collections::HashSet<_> = existing
            .iter()
            .map(|r| (r.collector_type, r.name.clone()))
            .collect();

        // Build set of new config keys
        let new_keys: std::collections::HashSet<_> = configs
            .iter()
            .map(|r| (r.collector_type, r.name.clone()))
            .collect();

        // Upsert all new configs
        for config in &configs {
            let existed = existing_keys.contains(&(config.collector_type, config.name.clone()));
            self.upsert(config).await?;
            if existed {
                result.updated += 1;
            } else {
                result.added += 1;
            }
        }

        // Delete stale configs
        for (collector_type, name) in existing_keys.difference(&new_keys) {
            self.delete(*collector_type, name).await?;
            result.deleted += 1;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::SqlitePool;
    use crate::storage::schema::init_schema;
    use std::sync::Arc;

    async fn create_test_pool() -> Arc<SqlitePool> {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init_schema(pool.inner()).await.unwrap();
        Arc::new(pool)
    }

    #[tokio::test]
    async fn test_collector_crud() {
        let pool = create_test_pool().await;
        let store = CollectorStore::new(pool);

        // Create
        let record = CollectorRecord::from_config(
            CollectorType::Tcp,
            "test-tcp",
            true,
            "default",
            serde_json::json!({"host": "127.0.0.1", "port": 6379}),
        );
        let id = store.upsert(&record).await.unwrap();
        assert!(id > 0);

        // Read
        let fetched = store.get(CollectorType::Tcp, "test-tcp").await.unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.name, "test-tcp");
        assert!(fetched.enabled);

        // Update
        let mut updated = record.clone();
        updated.enabled = false;
        store.upsert(&updated).await.unwrap();
        let fetched = store
            .get(CollectorType::Tcp, "test-tcp")
            .await
            .unwrap()
            .unwrap();
        assert!(!fetched.enabled);

        // Delete
        let deleted = store.delete(CollectorType::Tcp, "test-tcp").await.unwrap();
        assert!(deleted);
        let fetched = store.get(CollectorType::Tcp, "test-tcp").await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_collector_sync() {
        let pool = create_test_pool().await;
        let store = CollectorStore::new(pool);

        // Initial sync with 2 collectors
        let configs = vec![
            CollectorRecord::from_config(
                CollectorType::Tcp,
                "tcp-1",
                true,
                "default",
                serde_json::json!({}),
            ),
            CollectorRecord::from_config(
                CollectorType::Ping,
                "ping-1",
                true,
                "default",
                serde_json::json!({}),
            ),
        ];
        let result = store.sync_from_config(configs).await.unwrap();
        assert_eq!(result.added, 2);
        assert_eq!(result.updated, 0);
        assert_eq!(result.deleted, 0);

        // Sync again with 1 collector removed and 1 added
        let configs = vec![
            CollectorRecord::from_config(
                CollectorType::Tcp,
                "tcp-1",
                true,
                "default",
                serde_json::json!({}),
            ),
            CollectorRecord::from_config(
                CollectorType::Http,
                "http-1",
                true,
                "default",
                serde_json::json!({}),
            ),
        ];
        let result = store.sync_from_config(configs).await.unwrap();
        assert_eq!(result.added, 1);
        assert_eq!(result.updated, 1);
        assert_eq!(result.deleted, 1);

        // Verify final state
        let all = store.list_all().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_collector_type_enum() {
        use std::str::FromStr;

        assert_eq!(CollectorType::from_str("tcp").unwrap(), CollectorType::Tcp);
        assert_eq!(
            CollectorType::from_str("PING").unwrap(),
            CollectorType::Ping
        );
        let http_ref: &str = CollectorType::Http.as_ref();
        assert_eq!(http_ref, "http");
    }
}

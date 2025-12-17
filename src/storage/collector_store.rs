//! Collector storage and synchronization.
//!
//! Provides CRUD operations for collector configurations and
//! sync logic for config-file collectors on startup.

use std::sync::Arc;

use duckdb::params;
use serde::{Deserialize, Serialize};
use strum_macros::{AsRefStr, Display, EnumString};

use crate::storage::StorageError;
use crate::storage::pool::ReadPool;

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
    pool: Arc<ReadPool>,
}

impl CollectorStore {
    /// Create a new collector store.
    pub fn new(pool: Arc<ReadPool>) -> Self {
        Self { pool }
    }

    /// Upsert a collector record.
    ///
    /// Returns the record ID.
    pub fn upsert(&self, record: &CollectorRecord) -> Result<i64, StorageError> {
        let conn = self.pool.get()?;
        let now = chrono::Utc::now().timestamp_millis();
        let config_json = record.config.to_string();

        // Use INSERT ... ON CONFLICT for upsert
        let sql = r#"
            INSERT INTO collectors (type, name, source, enabled, group_name, config, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)
            ON CONFLICT (type, name) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                group_name = EXCLUDED.group_name,
                config = EXCLUDED.config,
                updated_at = EXCLUDED.updated_at
            RETURNING id
        "#;

        let id: i64 = conn.query_row(
            sql,
            params![
                record.collector_type.as_ref(),
                record.name.as_str(),
                record.source.as_ref(),
                record.enabled,
                record.group_name.as_str(),
                config_json.as_str(),
                now,
            ],
            |row: &duckdb::Row<'_>| row.get(0),
        )?;

        Ok(id)
    }

    /// Delete a collector by type and name.
    pub fn delete(&self, collector_type: CollectorType, name: &str) -> Result<bool, StorageError> {
        let conn = self.pool.get()?;

        let rows = conn.execute(
            "DELETE FROM collectors WHERE type = $1 AND name = $2",
            params![collector_type.as_ref(), name],
        )?;

        Ok(rows > 0)
    }

    /// List collectors by source.
    pub fn list_by_source(
        &self,
        source: CollectorSource,
    ) -> Result<Vec<CollectorRecord>, StorageError> {
        let conn = self.pool.get()?;

        let mut stmt = conn.prepare(
            "SELECT id, type::VARCHAR, name, source::VARCHAR, enabled, group_name, config, created_at, updated_at
             FROM collectors WHERE source = $1 ORDER BY type, name"
        )?;

        let records = stmt
            .query_map(params![source.as_ref()], |row: &duckdb::Row<'_>| {
                let type_str: String = row.get(1)?;
                let source_str: String = row.get(3)?;
                let config_str: String = row.get(6)?;

                Ok(CollectorRecord {
                    id: Some(row.get(0)?),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2)?,
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get(4)?,
                    group_name: row.get(5)?,
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// List all collectors.
    pub fn list_all(&self) -> Result<Vec<CollectorRecord>, StorageError> {
        let conn = self.pool.get()?;

        let mut stmt = conn.prepare(
            "SELECT id, type::VARCHAR, name, source::VARCHAR, enabled, group_name, config, created_at, updated_at
             FROM collectors ORDER BY type, name"
        )?;

        let records = stmt
            .query_map([], |row: &duckdb::Row<'_>| {
                let type_str: String = row.get(1)?;
                let source_str: String = row.get(3)?;
                let config_str: String = row.get(6)?;

                Ok(CollectorRecord {
                    id: Some(row.get(0)?),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2)?,
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get(4)?,
                    group_name: row.get(5)?,
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Get a collector by type and name.
    pub fn get(
        &self,
        collector_type: CollectorType,
        name: &str,
    ) -> Result<Option<CollectorRecord>, StorageError> {
        let conn = self.pool.get()?;

        let result: Result<CollectorRecord, duckdb::Error> = conn.query_row(
            "SELECT id, type::VARCHAR, name, source::VARCHAR, enabled, group_name, config, created_at, updated_at
             FROM collectors WHERE type = $1 AND name = $2",
            params![collector_type.as_ref(), name],
            |row: &duckdb::Row<'_>| {
                let type_str: String = row.get(1)?;
                let source_str: String = row.get(3)?;
                let config_str: String = row.get(6)?;

                Ok(CollectorRecord {
                    id: Some(row.get(0)?),
                    collector_type: type_str.parse().unwrap_or(CollectorType::Tcp),
                    name: row.get(2)?,
                    source: source_str.parse().unwrap_or(CollectorSource::Config),
                    enabled: row.get(4)?,
                    group_name: row.get(5)?,
                    config: serde_json::from_str(&config_str).unwrap_or_default(),
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::from(e)),
        }
    }

    /// Sync config-file collectors.
    ///
    /// - Upserts all provided config collectors
    /// - Deletes stale config collectors not in the provided list
    pub fn sync_from_config(
        &self,
        configs: Vec<CollectorRecord>,
    ) -> Result<SyncResult, StorageError> {
        let mut result = SyncResult::default();

        // Get existing config collectors
        let existing = self.list_by_source(CollectorSource::Config)?;
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
            self.upsert(config)?;
            if existed {
                result.updated += 1;
            } else {
                result.added += 1;
            }
        }

        // Delete stale configs
        for (collector_type, name) in existing_keys.difference(&new_keys) {
            self.delete(*collector_type, name)?;
            result.deleted += 1;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pool::ReadPool;
    use crate::storage::schema::init_schema;
    use duckdb::Connection;
    use std::sync::Arc;

    fn create_test_pool() -> Arc<ReadPool> {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        ReadPool::new(conn)
    }

    #[test]
    fn test_collector_crud() {
        let pool = create_test_pool();
        let store = CollectorStore::new(pool);

        // Create
        let record = CollectorRecord::from_config(
            CollectorType::Tcp,
            "test-tcp",
            true,
            "default",
            serde_json::json!({"host": "127.0.0.1", "port": 6379}),
        );
        let id = store.upsert(&record).unwrap();
        assert!(id > 0);

        // Read
        let fetched = store.get(CollectorType::Tcp, "test-tcp").unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.name, "test-tcp");
        assert!(fetched.enabled);

        // Update
        let mut updated = record.clone();
        updated.enabled = false;
        store.upsert(&updated).unwrap();
        let fetched = store.get(CollectorType::Tcp, "test-tcp").unwrap().unwrap();
        assert!(!fetched.enabled);

        // Delete
        let deleted = store.delete(CollectorType::Tcp, "test-tcp").unwrap();
        assert!(deleted);
        let fetched = store.get(CollectorType::Tcp, "test-tcp").unwrap();
        assert!(fetched.is_none());
    }

    #[test]
    fn test_collector_sync() {
        let pool = create_test_pool();
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
        let result = store.sync_from_config(configs).unwrap();
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
        let result = store.sync_from_config(configs).unwrap();
        assert_eq!(result.added, 1);
        assert_eq!(result.updated, 1);
        assert_eq!(result.deleted, 1);

        // Verify final state
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_collector_type_enum() {
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

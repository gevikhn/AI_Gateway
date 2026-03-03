use crate::api_keys::ban::BanMetricsSnapshot;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Row, Sqlite};
use std::str::FromStr;
use thiserror::Error;

/// 封禁日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanLogEntry {
    /// 日志ID
    pub id: String,
    /// API Key ID
    pub api_key_id: String,
    /// 触发封禁的规则ID
    pub rule_id: String,
    /// 封禁原因
    pub reason: String,
    /// 封禁开始时间（Unix秒）
    pub banned_at: u64,
    /// 封禁结束时间（Unix秒）
    pub banned_until: u64,
    /// 实际解封时间（Unix秒，未解封为null）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unbanned_at: Option<u64>,
    /// 触发时的指标快照
    pub metrics_snapshot: BanMetricsSnapshot,
}

/// 封禁日志错误
#[derive(Debug, Error)]
pub enum BanLogError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
}

/// 封禁日志存储 trait
#[async_trait]
pub trait BanLogStore: Send + Sync {
    /// 写入封禁记录
    async fn insert(&self, entry: BanLogEntry) -> Result<(), BanLogError>;

    /// 更新解封时间
    async fn mark_unbanned(&self, entry_id: &str, unbanned_at: u64) -> Result<(), BanLogError>;

    /// 查询 API Key 的封禁历史
    async fn query_by_api_key(
        &self,
        api_key_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError>;

    /// 查询最近的封禁日志（所有 API Keys）
    async fn query_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError>;

    /// 查询活跃的封禁（未解封且未过期）
    async fn query_active_bans(
        &self,
        before: u64,
    ) -> Result<Vec<BanLogEntry>, BanLogError>;

    /// 清理过期日志
    async fn cleanup_old_entries(&self, before: u64) -> Result<u64, BanLogError>;
}

/// SQLite 封禁日志存储实现
pub struct SqliteBanLogStore {
    pool: Pool<Sqlite>,
}

impl SqliteBanLogStore {
    /// 创建新的 SQLite 存储实例
    pub async fn new(database_path: &str) -> Result<Self, BanLogError> {
        tracing::info!("Creating ban log store, database_path: {}", database_path);

        // 确保目录存在
        if let Some(parent) = std::path::Path::new(database_path).parent() {
            tracing::info!("Creating parent directory: {}", parent.display());
            match std::fs::create_dir_all(parent) {
                Ok(_) => {
                    tracing::info!("Parent directory created or already exists");
                    // 验证目录是否真的存在
                    if !parent.exists() {
                        tracing::error!("Parent directory still does not exist after creation attempt");
                        return Err(BanLogError::InvalidData(
                            format!("Failed to create directory: {} - still does not exist", parent.display())
                        ));
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create directory {}: {}", parent.display(), e);
                    return Err(BanLogError::InvalidData(format!("Failed to create directory: {}", e)));
                }
            }
        } else {
            tracing::warn!("No parent directory for database path");
        }

        tracing::info!("Connecting to SQLite database...");

        // 使用 SqliteConnectOptions 并设置 create_if_missing(true)
        // 这样如果数据库文件不存在会自动创建
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(database_path)
            .map_err(|e| {
                tracing::error!("Invalid SQLite connection string: {}", e);
                BanLogError::InvalidData(format!("Invalid connection string: {}", e))
            })?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| {
                tracing::error!("Failed to connect to SQLite database {}: {}", database_path, e);
                BanLogError::Database(e)
            })?;
        tracing::info!("SQLite connection established");

        let store = Self { pool };
        store.init_tables().await?;

        Ok(store)
    }

    /// 初始化数据库表
    async fn init_tables(&self) -> Result<(), BanLogError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS ban_logs (
                id TEXT PRIMARY KEY,
                api_key_id TEXT NOT NULL,
                rule_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                banned_at INTEGER NOT NULL,
                banned_until INTEGER NOT NULL,
                unbanned_at INTEGER,
                metrics_requests INTEGER NOT NULL,
                metrics_errors INTEGER NOT NULL,
                metrics_error_rate REAL NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // 创建索引
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_ban_logs_api_key_id ON ban_logs(api_key_id)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_ban_logs_banned_at ON ban_logs(banned_at)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_ban_logs_unbanned_at ON ban_logs(unbanned_at)
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// 将 BanLogEntry 转换为数据库行
    fn entry_to_row(entry: &BanLogEntry) -> (String, String, String, String, i64, i64, Option<i64>, i64, i64, f64) {
        (
            entry.id.clone(),
            entry.api_key_id.clone(),
            entry.rule_id.clone(),
            entry.reason.clone(),
            entry.banned_at as i64,
            entry.banned_until as i64,
            entry.unbanned_at.map(|t| t as i64),
            entry.metrics_snapshot.requests as i64,
            entry.metrics_snapshot.errors as i64,
            entry.metrics_snapshot.error_rate,
        )
    }

    /// 将数据库行转换为 BanLogEntry
    fn row_to_entry(row: &sqlx::sqlite::SqliteRow) -> Result<BanLogEntry, BanLogError> {
        Ok(BanLogEntry {
            id: row.try_get("id")?,
            api_key_id: row.try_get("api_key_id")?,
            rule_id: row.try_get("rule_id")?,
            reason: row.try_get("reason")?,
            banned_at: row.try_get::<i64, _>("banned_at")? as u64,
            banned_until: row.try_get::<i64, _>("banned_until")? as u64,
            unbanned_at: row.try_get::<Option<i64>, _>("unbanned_at")?.map(|t| t as u64),
            metrics_snapshot: BanMetricsSnapshot {
                requests: row.try_get::<i64, _>("metrics_requests")? as u64,
                errors: row.try_get::<i64, _>("metrics_errors")? as u64,
                error_rate: row.try_get("metrics_error_rate")?,
            },
        })
    }
}

#[async_trait]
impl BanLogStore for SqliteBanLogStore {
    async fn insert(&self, entry: BanLogEntry) -> Result<(), BanLogError> {
        let (id, api_key_id, rule_id, reason, banned_at, banned_until, unbanned_at,
             metrics_requests, metrics_errors, metrics_error_rate) = Self::entry_to_row(&entry);

        sqlx::query(
            r#"
            INSERT INTO ban_logs (
                id, api_key_id, rule_id, reason, banned_at, banned_until, unbanned_at,
                metrics_requests, metrics_errors, metrics_error_rate
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(api_key_id)
        .bind(rule_id)
        .bind(reason)
        .bind(banned_at)
        .bind(banned_until)
        .bind(unbanned_at)
        .bind(metrics_requests)
        .bind(metrics_errors)
        .bind(metrics_error_rate)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn mark_unbanned(&self, entry_id: &str, unbanned_at: u64) -> Result<(), BanLogError> {
        let result = sqlx::query(
            r#"
            UPDATE ban_logs SET unbanned_at = ? WHERE id = ?
            "#,
        )
        .bind(unbanned_at as i64)
        .bind(entry_id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(BanLogError::NotFound(entry_id.to_string()));
        }

        Ok(())
    }

    async fn query_by_api_key(
        &self,
        api_key_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM ban_logs
            WHERE api_key_id = ?
            ORDER BY banned_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(api_key_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| Self::row_to_entry(row))
            .collect::<Result<Vec<_>, _>>()
    }

    async fn query_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM ban_logs
            ORDER BY banned_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| Self::row_to_entry(row))
            .collect::<Result<Vec<_>, _>>()
    }

    async fn query_active_bans(
        &self,
        before: u64,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM ban_logs
            WHERE unbanned_at IS NULL AND banned_until > ?
            ORDER BY banned_at DESC
            "#,
        )
        .bind(before as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| Self::row_to_entry(row))
            .collect::<Result<Vec<_>, _>>()
    }

    async fn cleanup_old_entries(&self, before: u64) -> Result<u64, BanLogError> {
        let result = sqlx::query(
            r#"
            DELETE FROM ban_logs WHERE banned_at < ?
            "#,
        )
        .bind(before as i64)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

/// 内存封禁日志存储（用于测试）
pub struct InMemoryBanLogStore {
    entries: std::sync::Mutex<Vec<BanLogEntry>>,
}

impl InMemoryBanLogStore {
    pub fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl BanLogStore for InMemoryBanLogStore {
    async fn insert(&self, entry: BanLogEntry) -> Result<(), BanLogError> {
        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);
        Ok(())
    }

    async fn mark_unbanned(&self, entry_id: &str, unbanned_at: u64) -> Result<(), BanLogError> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .iter_mut()
            .find(|e| e.id == entry_id)
            .ok_or_else(|| BanLogError::NotFound(entry_id.to_string()))?;
        entry.unbanned_at = Some(unbanned_at);
        Ok(())
    }

    async fn query_by_api_key(
        &self,
        api_key_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let entries = self.entries.lock().unwrap();
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.api_key_id == api_key_id)
            .cloned()
            .collect();

        Ok(filtered.into_iter().skip(offset).take(limit).collect())
    }

    async fn query_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let entries = self.entries.lock().unwrap();
        // 按 banned_at 降序排序
        let mut sorted: Vec<_> = entries.iter().cloned().collect();
        sorted.sort_by(|a, b| b.banned_at.cmp(&a.banned_at));
        Ok(sorted.into_iter().skip(offset).take(limit).collect())
    }

    async fn query_active_bans(
        &self,
        before: u64,
    ) -> Result<Vec<BanLogEntry>, BanLogError> {
        let entries = self.entries.lock().unwrap();
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.unbanned_at.is_none() && e.banned_until > before)
            .cloned()
            .collect();
        Ok(filtered)
    }

    async fn cleanup_old_entries(&self, before: u64) -> Result<u64, BanLogError> {
        let mut entries = self.entries.lock().unwrap();
        let original_len = entries.len();
        entries.retain(|e| e.banned_at >= before);
        Ok((original_len - entries.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entry(id: &str, api_key_id: &str) -> BanLogEntry {
        BanLogEntry {
            id: id.to_string(),
            api_key_id: api_key_id.to_string(),
            rule_id: "rule_001".to_string(),
            reason: "Test ban".to_string(),
            banned_at: 1000,
            banned_until: 2000,
            unbanned_at: None,
            metrics_snapshot: BanMetricsSnapshot {
                requests: 100,
                errors: 50,
                error_rate: 0.5,
            },
        }
    }

    #[tokio::test]
    async fn test_in_memory_store() {
        let store = InMemoryBanLogStore::new();

        // 插入记录
        let entry = create_test_entry("log_001", "ak_001");
        store.insert(entry.clone()).await.unwrap();

        // 查询
        let results = store.query_by_api_key("ak_001", 10, 0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "log_001");

        // 标记解封
        store.mark_unbanned("log_001", 1500).await.unwrap();
        let results = store.query_by_api_key("ak_001", 10, 0).await.unwrap();
        assert_eq!(results[0].unbanned_at, Some(1500));

        // 查询活跃封禁
        let active = store.query_active_bans(1800).await.unwrap();
        assert!(active.is_empty()); // 已经解封

        // 清理旧记录
        let deleted = store.cleanup_old_entries(1500).await.unwrap();
        assert_eq!(deleted, 1);
    }
}

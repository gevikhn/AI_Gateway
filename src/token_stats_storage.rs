use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite, Row};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info, warn};

/// Token使用记录
#[derive(Debug, Clone)]
pub struct TokenUsageRecord {
    pub timestamp: i64,
    pub api_key_id: String,
    pub route_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_id: Option<String>,
}

/// 时间窗口类型
#[derive(Debug, Clone, Copy)]
pub enum TimeWindow {
    /// 按小时聚合（最近24小时）
    Day,
    /// 按天聚合（最近7天）
    Week,
    /// 按天聚合（最近30天）
    Month,
}

/// Token统计行
#[derive(Debug, Clone)]
pub struct TokenStatsRow {
    pub time_bucket: i64,
    pub api_key_id: Option<String>,
    pub route_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

/// Token统计存储
pub struct TokenStatsStorage {
    pool: Pool<Sqlite>,
    sender: mpsc::UnboundedSender<TokenUsageRecord>,
}

impl TokenStatsStorage {
    /// 创建新的Token统计存储
    pub async fn new(
        db_path: &str,
        flush_interval_secs: u64,
        batch_size: usize,
    ) -> Result<Self, String> {
        // 确保目录存在
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        // 创建连接池
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path))
            .map_err(|e| format!("Invalid connection string: {}", e))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| format!("Failed to connect to database: {}", e))?;

        // 创建表
        Self::create_tables(&pool).await?;

        // 创建通道
        let (sender, mut receiver) = mpsc::unbounded_channel();

        let storage = Self { pool, sender };

        // 启动后台写入任务
        let pool_clone = storage.pool.clone();
        tokio::spawn(async move {
            Self::background_writer(
                pool_clone,
                &mut receiver,
                flush_interval_secs,
                batch_size,
            )
            .await;
        });

        info!("TokenStatsStorage initialized at: {}", db_path);
        Ok(storage)
    }

    /// 创建数据库表
    async fn create_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
        // Token使用明细表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS token_usage_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                api_key_id TEXT NOT NULL,
                route_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                request_id TEXT
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create token_usage_records table: {}", e))?;

        // 创建索引
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_time ON token_usage_records(timestamp)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create time index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_key ON token_usage_records(api_key_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create key index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_usage_route ON token_usage_records(route_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create route index: {}", e))?;

        // 小时级聚合表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS token_stats_hourly (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_epoch INTEGER NOT NULL,
                api_key_id TEXT,
                route_id TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                request_count INTEGER NOT NULL DEFAULT 0,
                UNIQUE(hour_epoch, api_key_id, route_id)
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create hourly stats table: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_hourly_time ON token_stats_hourly(hour_epoch)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create hourly time index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_hourly_key ON token_stats_hourly(api_key_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create hourly key index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_hourly_route ON token_stats_hourly(route_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create hourly route index: {}", e))?;

        // 日级聚合表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS token_stats_daily (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                day_epoch INTEGER NOT NULL,
                api_key_id TEXT,
                route_id TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                request_count INTEGER NOT NULL DEFAULT 0,
                UNIQUE(day_epoch, api_key_id, route_id)
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create daily stats table: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_daily_time ON token_stats_daily(day_epoch)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create daily time index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_daily_key ON token_stats_daily(api_key_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create daily key index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_stats_daily_route ON token_stats_daily(route_id)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create daily route index: {}", e))?;

        info!("Token stats database tables created successfully");
        Ok(())
    }

    /// 后台写入任务
    async fn background_writer(
        pool: Pool<Sqlite>,
        receiver: &mut mpsc::UnboundedReceiver<TokenUsageRecord>,
        flush_interval_secs: u64,
        batch_size: usize,
    ) {
        let mut batch: Vec<TokenUsageRecord> = Vec::with_capacity(batch_size);
        let mut flush_tick = interval(tokio::time::Duration::from_secs(flush_interval_secs));

        loop {
            tokio::select! {
                Some(record) = receiver.recv() => {
                    batch.push(record);
                    if batch.len() >= batch_size {
                        Self::flush_batch(&pool, &mut batch).await;
                    }
                }
                _ = flush_tick.tick() => {
                    if !batch.is_empty() {
                        Self::flush_batch(&pool, &mut batch).await;
                    }
                    // 清理过期数据
                    if let Err(e) = Self::cleanup_old_records(&pool, 30).await {
                        warn!("Failed to cleanup old token stats records: {}", e);
                    }
                }
                else => break,
            }
        }

        // 刷新剩余记录
        if !batch.is_empty() {
            Self::flush_batch(&pool, &mut batch).await;
        }
    }

    /// 刷新批量记录到数据库
    async fn flush_batch(pool: &Pool<Sqlite>, batch: &mut Vec<TokenUsageRecord>) {
        if batch.is_empty() {
            return;
        }

        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                error!("Failed to begin transaction: {}", e);
                return;
            }
        };

        for record in batch.drain(..) {
            // 写入明细表
            let total_tokens = record.input_tokens + record.output_tokens;
            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO token_usage_records
                (timestamp, api_key_id, route_id, input_tokens, output_tokens, total_tokens, request_id)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
            )
            .bind(record.timestamp)
            .bind(&record.api_key_id)
            .bind(&record.route_id)
            .bind(record.input_tokens as i64)
            .bind(record.output_tokens as i64)
            .bind(total_tokens as i64)
            .bind(&record.request_id)
            .execute(&mut *tx)
            .await
            {
                error!("Failed to insert token usage record: {}", e);
                continue;
            }

            // 更新小时级聚合
            let hour_epoch = truncate_to_hour(record.timestamp as u64) as i64;
            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO token_stats_hourly (hour_epoch, api_key_id, route_id, input_tokens, output_tokens, request_count)
                VALUES (?1, ?2, ?3, ?4, ?5, 1)
                ON CONFLICT(hour_epoch, api_key_id, route_id) DO UPDATE SET
                    input_tokens = input_tokens + excluded.input_tokens,
                    output_tokens = output_tokens + excluded.output_tokens,
                    request_count = request_count + 1
                "#,
            )
            .bind(hour_epoch)
            .bind(&record.api_key_id)
            .bind(&record.route_id)
            .bind(record.input_tokens as i64)
            .bind(record.output_tokens as i64)
            .execute(&mut *tx)
            .await
            {
                error!("Failed to upsert hourly stats: {}", e);
                continue;
            }

            // 更新日级聚合
            let day_epoch = truncate_to_day(record.timestamp as u64) as i64;
            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO token_stats_daily (day_epoch, api_key_id, route_id, input_tokens, output_tokens, request_count)
                VALUES (?1, ?2, ?3, ?4, ?5, 1)
                ON CONFLICT(day_epoch, api_key_id, route_id) DO UPDATE SET
                    input_tokens = input_tokens + excluded.input_tokens,
                    output_tokens = output_tokens + excluded.output_tokens,
                    request_count = request_count + 1
                "#,
            )
            .bind(day_epoch)
            .bind(&record.api_key_id)
            .bind(&record.route_id)
            .bind(record.input_tokens as i64)
            .bind(record.output_tokens as i64)
            .execute(&mut *tx)
            .await
            {
                error!("Failed to upsert daily stats: {}", e);
                continue;
            }
        }

        if let Err(e) = tx.commit().await {
            error!("Failed to commit transaction: {}", e);
        }
    }

    /// 清理过期记录
    async fn cleanup_old_records(pool: &Pool<Sqlite>, retention_days: u32) -> Result<(), sqlx::Error> {
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            - (retention_days as i64 * 86400);

        // 删除明细表过期数据
        sqlx::query("DELETE FROM token_usage_records WHERE timestamp < ?1")
            .bind(cutoff)
            .execute(pool)
            .await?;

        // 删除小时级聚合过期数据（保留30天）
        let hour_cutoff = truncate_to_hour(cutoff as u64) as i64;
        sqlx::query("DELETE FROM token_stats_hourly WHERE hour_epoch < ?1")
            .bind(hour_cutoff)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// 队列记录（异步批量写入）
    pub fn queue_record(&self, record: TokenUsageRecord) {
        let _ = self.sender.send(record);
    }

    /// 记录token使用（便捷方法）
    pub fn record_usage(
        &self,
        api_key_id: &str,
        route_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        request_id: Option<String>,
    ) {
        let record = TokenUsageRecord {
            timestamp: current_epoch_seconds() as i64,
            api_key_id: api_key_id.to_string(),
            route_id: route_id.to_string(),
            input_tokens,
            output_tokens,
            request_id,
        };
        self.queue_record(record);
    }

    /// 查询API Key的统计
    pub async fn query_api_key_stats(
        &self,
        api_key_id: &str,
        window: TimeWindow,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        match window {
            TimeWindow::Day => {
                // 查询最近24小时的小时级聚合
                let now = current_epoch_seconds();
                let day_ago = now - 86400;
                let hour_start = truncate_to_hour(day_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT hour_epoch as time_bucket, api_key_id, route_id,
                           input_tokens, output_tokens, request_count
                    FROM token_stats_hourly
                    WHERE api_key_id = ?1 AND hour_epoch >= ?2
                    ORDER BY hour_epoch ASC
                    "#,
                )
                .bind(api_key_id)
                .bind(hour_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: row.try_get("route_id")?,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
            TimeWindow::Week | TimeWindow::Month => {
                // 查询最近7天或30天的日级聚合
                let days = match window {
                    TimeWindow::Week => 7,
                    TimeWindow::Month => 30,
                    _ => 7,
                };
                let now = current_epoch_seconds();
                let days_ago = now - (days as u64 * 86400);
                let day_start = truncate_to_day(days_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT day_epoch as time_bucket, api_key_id, route_id,
                           input_tokens, output_tokens, request_count
                    FROM token_stats_daily
                    WHERE api_key_id = ?1 AND day_epoch >= ?2
                    ORDER BY day_epoch ASC
                    "#,
                )
                .bind(api_key_id)
                .bind(day_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: row.try_get("route_id")?,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
        }
    }

    /// 查询Route的统计
    pub async fn query_route_stats(
        &self,
        route_id: &str,
        window: TimeWindow,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        match window {
            TimeWindow::Day => {
                let now = current_epoch_seconds();
                let day_ago = now - 86400;
                let hour_start = truncate_to_hour(day_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT hour_epoch as time_bucket, api_key_id, route_id,
                           input_tokens, output_tokens, request_count
                    FROM token_stats_hourly
                    WHERE route_id = ?1 AND hour_epoch >= ?2
                    ORDER BY hour_epoch ASC
                    "#,
                )
                .bind(route_id)
                .bind(hour_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: row.try_get("route_id")?,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
            TimeWindow::Week | TimeWindow::Month => {
                let days = match window {
                    TimeWindow::Week => 7,
                    TimeWindow::Month => 30,
                    _ => 7,
                };
                let now = current_epoch_seconds();
                let days_ago = now - (days as u64 * 86400);
                let day_start = truncate_to_day(days_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT day_epoch as time_bucket, api_key_id, route_id,
                           input_tokens, output_tokens, request_count
                    FROM token_stats_daily
                    WHERE route_id = ?1 AND day_epoch >= ?2
                    ORDER BY day_epoch ASC
                    "#,
                )
                .bind(route_id)
                .bind(day_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: row.try_get("route_id")?,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
        }
    }

    /// 查询所有API Key的汇总统计
    pub async fn query_all_api_keys_summary(
        &self,
        window: TimeWindow,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        match window {
            TimeWindow::Day => {
                let now = current_epoch_seconds();
                let day_ago = now - 86400;
                let hour_start = truncate_to_hour(day_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT api_key_id,
                           SUM(input_tokens) as input_tokens,
                           SUM(output_tokens) as output_tokens,
                           SUM(request_count) as request_count
                    FROM token_stats_hourly
                    WHERE hour_epoch >= ?1
                    GROUP BY api_key_id
                    ORDER BY api_key_id ASC
                    "#,
                )
                .bind(hour_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: 0,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: None,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
            TimeWindow::Week => {
                let now = current_epoch_seconds();
                let week_ago = now - 7 * 86400;
                let day_start = truncate_to_day(week_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT api_key_id,
                           SUM(input_tokens) as input_tokens,
                           SUM(output_tokens) as output_tokens,
                           SUM(request_count) as request_count
                    FROM token_stats_daily
                    WHERE day_epoch >= ?1
                    GROUP BY api_key_id
                    ORDER BY api_key_id ASC
                    "#,
                )
                .bind(day_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: 0,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: None,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
            TimeWindow::Month => {
                let now = current_epoch_seconds();
                let month_ago = now - 30 * 86400;
                let day_start = truncate_to_day(month_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT api_key_id,
                           SUM(input_tokens) as input_tokens,
                           SUM(output_tokens) as output_tokens,
                           SUM(request_count) as request_count
                    FROM token_stats_daily
                    WHERE day_epoch >= ?1
                    GROUP BY api_key_id
                    ORDER BY api_key_id ASC
                    "#,
                )
                .bind(day_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: 0,
                    api_key_id: row.try_get("api_key_id")?,
                    route_id: None,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
        }
    }

    /// 查询API Key的统计数据用于恢复（从小时级聚合表）
    pub async fn query_api_key_stats_for_restore(
        &self,
        day_start: i64,
        _week_start: i64,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        // 查询今天的小时级聚合数据
        sqlx::query(
            r#"
            SELECT hour_epoch as time_bucket, api_key_id, route_id,
                   input_tokens, output_tokens, request_count
            FROM token_stats_hourly
            WHERE hour_epoch >= ?1 AND api_key_id IS NOT NULL
            ORDER BY hour_epoch ASC
            "#,
        )
        .bind(day_start)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| Ok(TokenStatsRow {
            time_bucket: row.try_get("time_bucket")?,
            api_key_id: row.try_get("api_key_id")?,
            route_id: row.try_get("route_id")?,
            input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
            output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
            request_count: row.try_get::<i64, _>("request_count")? as u64,
        }))
        .collect()
    }

    /// 查询Route的统计数据用于恢复（从小时级聚合表）
    pub async fn query_route_stats_for_restore(
        &self,
        day_start: i64,
        _week_start: i64,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        // 查询今天的小时级聚合数据
        sqlx::query(
            r#"
            SELECT hour_epoch as time_bucket, api_key_id, route_id,
                   input_tokens, output_tokens, request_count
            FROM token_stats_hourly
            WHERE hour_epoch >= ?1 AND route_id IS NOT NULL
            ORDER BY hour_epoch ASC
            "#,
        )
        .bind(day_start)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| Ok(TokenStatsRow {
            time_bucket: row.try_get("time_bucket")?,
            api_key_id: row.try_get("api_key_id")?,
            route_id: row.try_get("route_id")?,
            input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
            output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
            request_count: row.try_get::<i64, _>("request_count")? as u64,
        }))
        .collect()
    }

    /// 查询整体时间序列数据（所有API Key和Route的聚合）
    pub async fn query_time_series(
        &self,
        window: TimeWindow,
    ) -> Result<Vec<TokenStatsRow>, sqlx::Error> {
        match window {
            TimeWindow::Day => {
                // 查询最近24小时的小时级聚合
                let now = current_epoch_seconds();
                let day_ago = now - 86400;
                let hour_start = truncate_to_hour(day_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT hour_epoch as time_bucket,
                           SUM(input_tokens) as input_tokens,
                           SUM(output_tokens) as output_tokens,
                           SUM(request_count) as request_count
                    FROM token_stats_hourly
                    WHERE hour_epoch >= ?1
                    GROUP BY hour_epoch
                    ORDER BY hour_epoch ASC
                    "#,
                )
                .bind(hour_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: None,
                    route_id: None,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
            TimeWindow::Week | TimeWindow::Month => {
                // 查询最近7天或30天的日级聚合
                let days = match window {
                    TimeWindow::Week => 7,
                    TimeWindow::Month => 30,
                    _ => 7,
                };
                let now = current_epoch_seconds();
                let days_ago = now - (days as u64 * 86400);
                let day_start = truncate_to_day(days_ago) as i64;

                sqlx::query(
                    r#"
                    SELECT day_epoch as time_bucket,
                           SUM(input_tokens) as input_tokens,
                           SUM(output_tokens) as output_tokens,
                           SUM(request_count) as request_count
                    FROM token_stats_daily
                    WHERE day_epoch >= ?1
                    GROUP BY day_epoch
                    ORDER BY day_epoch ASC
                    "#,
                )
                .bind(day_start)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|row| Ok(TokenStatsRow {
                    time_bucket: row.try_get("time_bucket")?,
                    api_key_id: None,
                    route_id: None,
                    input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
                    output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
                    request_count: row.try_get::<i64, _>("request_count")? as u64,
                }))
                .collect()
            }
        }
    }
}

/// 获取当前Unix时间戳（秒）
fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 截断到小时
fn truncate_to_hour(timestamp: u64) -> u64 {
    (timestamp / 3600) * 3600
}

/// 截断到天
fn truncate_to_day(timestamp: u64) -> u64 {
    (timestamp / 86400) * 86400
}

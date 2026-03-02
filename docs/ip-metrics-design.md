# IP 维度监控数据模型和存储方案设计文档

## 1. 概述

本文档描述 AI Gateway IP 维度监控系统的设计，包括数据模型、存储方案、数据清理策略和与现有系统的集成方案。

## 2. 数据结构定义

### 2.1 核心数据结构

```rust
/// 时间窗口类型
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum TimeWindow {
    FiveMinutes,      // 5分钟
    OneHour,          // 1小时
    TwentyFourHours,  // 24小时
    OneWeek,          // 1周
    OneMonth,         // 1个月
}

impl TimeWindow {
    /// 获取窗口持续时间（分钟）
    pub fn duration_minutes(&self) -> u64 {
        match self {
            TimeWindow::FiveMinutes => 5,
            TimeWindow::OneHour => 60,
            TimeWindow::TwentyFourHours => 24 * 60,
            TimeWindow::OneWeek => 7 * 24 * 60,
            TimeWindow::OneMonth => 30 * 24 * 60,
        }
    }

    /// 获取窗口起始时间戳（向下取整到窗口边界）
    pub fn window_start(&self, timestamp_secs: u64) -> u64 {
        let duration_secs = self.duration_minutes() * 60;
        (timestamp_secs / duration_secs) * duration_secs
    }
}

/// 单条 IP 请求记录（内存缓冲用）
#[derive(Clone, Debug)]
pub struct IpMetricsRecord {
    pub ip_address: String,
    pub route_id: String,
    pub url_path: String,
    pub token_label: Option<String>,
    pub timestamp_secs: u64,
}

/// IP 维度聚合指标
#[derive(Clone, Debug, Serialize)]
pub struct IpAggregatedMetrics {
    pub ip_address: String,
    pub window_start: u64,           // Unix timestamp (seconds)
    pub window_type: TimeWindow,
    pub request_count: u64,
    pub unique_url_count: u32,       // 去重后的 URL 数量
    pub url_paths: Vec<String>,      // 访问的 URL 列表（限制数量）
    pub token_labels: Vec<String>,   // 使用的 token 列表（脱敏后，去重）
}

/// IP 统计摘要（实时汇总）
#[derive(Clone, Debug, Serialize)]
pub struct IpStatsSummary {
    pub ip_address: String,
    pub first_seen: u64,             // 首次出现时间
    pub last_seen: u64,              // 最后出现时间
    pub total_requests: u64,         // 总请求数
    pub requests_5min: u64,
    pub requests_1hour: u64,
    pub requests_24hour: u64,
    pub requests_1week: u64,
    pub requests_1month: u64,
}

/// IP 监控配置
#[derive(Clone, Debug)]
pub struct IpMetricsConfig {
    pub enabled: bool,
    pub db_path: PathBuf,
    pub buffer_size: usize,          // 内存缓冲区大小
    pub flush_interval_secs: u64,    // 刷盘间隔
}

impl Default for IpMetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: PathBuf::from("./data/ip_metrics.db"),
            buffer_size: 1000,
            flush_interval_secs: 60,
        }
    }
}
```

### 2.2 IP 监控管理器

```rust
/// IP 维度监控管理器
pub struct IpMetrics {
    config: IpMetricsConfig,
    db: Arc<Mutex<rusqlite::Connection>>,
    memory_buffer: Arc<Mutex<Vec<IpMetricsRecord>>>,
    last_flush: Arc<AtomicU64>,
}

impl IpMetrics {
    /// 创建新的 IP 监控实例
    pub fn new(config: IpMetricsConfig) -> Result<Self, String> {
        // 初始化数据库连接
        // 创建表结构
        // 启动定时刷盘任务
    }

    /// 记录请求（从 observe_request 调用）
    pub fn record_request(
        &self,
        ip_address: &str,
        route_id: &str,
        url_path: &str,
        token_label: Option<&str>,
    ) {
        // 添加到内存缓冲区
        // 检查是否需要刷盘
    }

    /// 获取指定时间窗口的 IP 聚合数据
    pub fn query_aggregated(
        &self,
        window: TimeWindow,
        limit: usize,
    ) -> Result<Vec<IpAggregatedMetrics>, String> {
        // 查询聚合表
    }

    /// 获取特定 IP 的详细统计
    pub fn query_ip_detail(
        &self,
        ip_address: &str,
        window: TimeWindow,
    ) -> Result<Option<IpAggregatedMetrics>, String> {
        // 查询特定 IP 的指标
    }

    /// 获取 IP 统计摘要列表
    pub fn query_summary(&self, limit: usize) -> Result<Vec<IpStatsSummary>, String> {
        // 查询汇总表
    }

    /// 手动刷盘（内存缓冲区写入数据库）
    pub fn flush(&self) -> Result<(), String> {
        // 批量插入数据
        // 更新聚合表
    }

    /// 执行数据清理
    pub fn prune_old_data(&self) -> Result<(), String> {
        // 清理过期数据
    }
}
```

## 3. 存储方案

### 3.1 方案选择：SQLite

选择 SQLite 作为持久化存储方案，理由如下：

| 特性 | SQLite | 纯内存 | 自定义文件 | PostgreSQL |
|------|--------|--------|------------|------------|
| 持久化 | ✅ | ❌ | ✅ | ✅ |
| 轻量级 | ✅ | ✅ | ✅ | ❌ |
| 单文件 | ✅ | N/A | ✅ | ❌ |
| SQL查询 | ✅ | 需实现 | 需实现 | ✅ |
| 事务支持 | ✅ | 需实现 | 需实现 | ✅ |
| 运维成本 | 极低 | 无 | 低 | 高 |
| 适用规模 | 中小 | 小 | 中小 | 大 |

**决策理由**：
1. **零运维**：单文件存储，无需额外服务进程
2. **足够性能**：对于网关监控场景，SQLite 可轻松处理数千 QPS 的写入
3. **查询灵活**：支持复杂的时间窗口聚合查询
4. **生态成熟**：Rust 的 `rusqlite` 库稳定可靠
5. **部署简单**：与 AI Gateway 一起打包，无需额外依赖

### 3.2 数据库表结构

```sql
-- 原始请求日志表（可选，用于详细审计）
-- 保留策略：24小时
CREATE TABLE IF NOT EXISTS ip_request_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ip_address TEXT NOT NULL,
    route_id TEXT NOT NULL,
    url_path TEXT NOT NULL,
    token_label TEXT,
    timestamp INTEGER NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 5分钟聚合表
-- 保留策略：7天
CREATE TABLE IF NOT EXISTS ip_metrics_5min (
    ip_address TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    request_count INTEGER DEFAULT 0,
    unique_url_count INTEGER DEFAULT 0,
    url_paths TEXT,  -- JSON array, 最多存储20个URL
    token_labels TEXT, -- JSON array, 去重后的token
    PRIMARY KEY (ip_address, window_start)
) WITHOUT ROWID;

-- 1小时聚合表
-- 保留策略：30天
CREATE TABLE IF NOT EXISTS ip_metrics_1hour (
    ip_address TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    request_count INTEGER DEFAULT 0,
    unique_url_count INTEGER DEFAULT 0,
    url_paths TEXT,
    token_labels TEXT,
    PRIMARY KEY (ip_address, window_start)
) WITHOUT ROWID;

-- 24小时聚合表
-- 保留策略：90天
CREATE TABLE IF NOT EXISTS ip_metrics_24hour (
    ip_address TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    request_count INTEGER DEFAULT 0,
    unique_url_count INTEGER DEFAULT 0,
    url_paths TEXT,
    token_labels TEXT,
    PRIMARY KEY (ip_address, window_start)
) WITHOUT ROWID;

-- IP 统计汇总表（实时更新）
-- 长期保留，定期清理不活跃的IP
CREATE TABLE IF NOT EXISTS ip_stats_summary (
    ip_address TEXT PRIMARY KEY,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    total_requests INTEGER DEFAULT 0,
    requests_5min INTEGER DEFAULT 0,
    requests_1hour INTEGER DEFAULT 0,
    requests_24hour INTEGER DEFAULT 0,
    requests_1week INTEGER DEFAULT 0,
    requests_1month INTEGER DEFAULT 0,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
) WITHOUT ROWID;

-- 索引
CREATE INDEX IF NOT EXISTS idx_logs_timestamp ON ip_request_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_logs_ip ON ip_request_logs(ip_address);
CREATE INDEX IF NOT EXISTS idx_5min_window ON ip_metrics_5min(window_start);
CREATE INDEX IF NOT EXISTS idx_1hour_window ON ip_metrics_1hour(window_start);
CREATE INDEX IF NOT EXISTS idx_24hour_window ON ip_metrics_24hour(window_start);
CREATE INDEX IF NOT EXISTS idx_summary_last_seen ON ip_stats_summary(last_seen);
```

### 3.3 写入优化策略

```rust
/// 批量写入实现
fn batch_insert_records(
    conn: &mut rusqlite::Connection,
    records: &[IpMetricsRecord],
) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO ip_request_logs (ip_address, route_id, url_path, token_label, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)"
        )?;

        for record in records {
            stmt.execute((
                &record.ip_address,
                &record.route_id,
                &record.url_path,
                &record.token_label,
                record.timestamp_secs as i64,
            ))?;
        }
    }

    tx.commit()?;
    Ok(())
}

/// 聚合更新（使用 UPSERT）
fn upsert_aggregation(
    conn: &mut rusqlite::Connection,
    table: &str,
    metrics: &IpAggregatedMetrics,
) -> Result<(), rusqlite::Error> {
    let sql = format!(
        "INSERT INTO {} (ip_address, window_start, request_count, unique_url_count, url_paths, token_labels)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(ip_address, window_start) DO UPDATE SET
         request_count = request_count + excluded.request_count,
         unique_url_count = excluded.unique_url_count,
         url_paths = excluded.url_paths,
         token_labels = excluded.token_labels",
        table
    );

    conn.execute(&sql, (
        &metrics.ip_address,
        metrics.window_start as i64,
        metrics.request_count as i64,
        metrics.unique_url_count as i64,
        serde_json::to_string(&metrics.url_paths).unwrap_or_default(),
        serde_json::to_string(&metrics.token_labels).unwrap_or_default(),
    ))?;

    Ok(())
}
```

## 4. 数据清理策略

### 4.1 分层保留策略

| 数据类型 | 保留时间 | 清理频率 |
|----------|----------|----------|
| 原始请求日志 | 24小时 | 每小时 |
| 5分钟聚合 | 7天 | 每天 |
| 1小时聚合 | 30天 | 每天 |
| 24小时聚合 | 90天 | 每周 |
| IP 汇总表 | 长期（不活跃IP清理） | 每周 |

### 4.2 清理实现

```rust
impl IpMetrics {
    /// 执行数据清理
    pub fn prune_old_data(&self) -> Result<PruneResult, String> {
        let mut conn = self.db.lock().map_err(|_| "lock poisoned")?;
        let now = epoch_secs_now();

        let result = PruneResult {
            logs_deleted: 0,
            agg_5min_deleted: 0,
            agg_1hour_deleted: 0,
            agg_24hour_deleted: 0,
            summary_deleted: 0,
        };

        // 清理原始日志（24小时前）
        let cutoff_24h = now - 24 * 3600;
        result.logs_deleted = conn.execute(
            "DELETE FROM ip_request_logs WHERE timestamp < ?1",
            [cutoff_24h],
        )?;

        // 清理5分钟聚合（7天前）
        let cutoff_7d = now - 7 * 24 * 3600;
        result.agg_5min_deleted = conn.execute(
            "DELETE FROM ip_metrics_5min WHERE window_start < ?1",
            [cutoff_7d],
        )?;

        // 清理1小时聚合（30天前）
        let cutoff_30d = now - 30 * 24 * 3600;
        result.agg_1hour_deleted = conn.execute(
            "DELETE FROM ip_metrics_1hour WHERE window_start < ?1",
            [cutoff_30d],
        )?;

        // 清理24小时聚合（90天前）
        let cutoff_90d = now - 90 * 24 * 3600;
        result.agg_24hour_deleted = conn.execute(
            "DELETE FROM ip_metrics_24hour WHERE window_start < ?1",
            [cutoff_90d],
        )?;

        // 清理不活跃的IP（1个月无活动）
        let cutoff_inactive = now - 30 * 24 * 3600;
        result.summary_deleted = conn.execute(
            "DELETE FROM ip_stats_summary WHERE last_seen < ?1",
            [cutoff_inactive],
        )?;

        // 执行 VACUUM 回收空间（可选，低峰期执行）
        // conn.execute("VACUUM", [])?;

        Ok(result)
    }
}

#[derive(Debug, Serialize)]
pub struct PruneResult {
    pub logs_deleted: usize,
    pub agg_5min_deleted: usize,
    pub agg_1hour_deleted: usize,
    pub agg_24hour_deleted: usize,
    pub summary_deleted: usize,
}
```

### 4.3 定时任务

```rust
/// 启动后台清理任务
pub fn start_prune_task(metrics: Arc<IpMetrics>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600)); // 每小时

        loop {
            interval.tick().await;

            match metrics.prune_old_data() {
                Ok(result) => {
                    info!(
                        "IP metrics prune completed: logs={}, 5min={}, 1hour={}, 24hour={}, inactive_ips={}",
                        result.logs_deleted,
                        result.agg_5min_deleted,
                        result.agg_1hour_deleted,
                        result.agg_24hour_deleted,
                        result.summary_deleted,
                    );
                }
                Err(err) => {
                    error!("IP metrics prune failed: {}", err);
                }
            }
        }
    });
}
```

## 5. 与现有监控系统集成

### 5.1 集成点

```rust
// observability.rs 修改

pub struct ObservabilityRuntime {
    pub metrics: Option<Arc<GatewayMetrics>>,
    pub ip_metrics: Option<Arc<IpMetrics>>,  // 新增
    // ... 其他字段
}

impl ObservabilityRuntime {
    pub fn from_config(config: Option<&ObservabilityConfig>) -> Self {
        // ... 现有代码

        let ip_metrics = if config.ip_metrics.enabled {
            match IpMetrics::new(config.ip_metrics.clone()) {
                Ok(metrics) => {
                    let metrics = Arc::new(metrics);
                    start_prune_task(metrics.clone());
                    Some(metrics)
                }
                Err(err) => {
                    error!("Failed to initialize IP metrics: {}", err);
                    None
                }
            }
        } else {
            None
        };

        Self {
            // ...
            ip_metrics,
        }
    }

    /// 记录 IP 维度请求
    pub fn record_ip_request(
        &self,
        ip_address: &str,
        route_id: &str,
        url_path: &str,
        token_label: Option<&str>,
    ) {
        if let Some(ip_metrics) = &self.ip_metrics {
            ip_metrics.record_request(ip_address, route_id, url_path, token_label);
        }
    }

    /// 获取 IP 指标快照
    pub fn snapshot_ip_metrics(&self, window: TimeWindow) -> Option<Vec<IpAggregatedMetrics>> {
        self.ip_metrics.as_ref()?.query_aggregated(window, 100).ok()
    }
}
```

### 5.2 请求处理集成

```rust
// server.rs 中 proxy_handler 修改

async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Response<Body> {
    // 获取客户端 IP
    let client_ip = extract_client_ip(&headers, addr);

    // ... 现有处理逻辑

    // 记录 IP 维度指标
    state.observability.record_ip_request(
        &client_ip,
        route_id,
        req.uri().path(),
        token_label.as_deref(),
    );

    // ... 继续处理
}

/// 提取客户端 IP
fn extract_client_ip(headers: &HeaderMap, socket_addr: SocketAddr) -> String {
    // 优先从 X-Forwarded-For 获取
    if let Some(forwarded) = headers.get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        // 取第一个 IP（最原始的客户端）
        forwarded.split(',').next()
            .map(|ip| ip.trim().to_string())
            .filter(|ip| !ip.is_empty())
            .unwrap_or_else(|| socket_addr.ip().to_string())
    } else {
        socket_addr.ip().to_string()
    }
}
```

### 5.3 Admin API 扩展

```rust
// admin.rs 新增接口

pub fn register_admin_routes(router: Router<AppState>, prefix: &str) -> Router<AppState> {
    let prefix = prefix.trim_end_matches('/');
    router
        // ... 现有路由
        .route(&format!("{prefix}/api/metrics/ip"), get(admin_ip_metrics_handler))
        .route(&format!("{prefix}/api/metrics/ip/:ip"), get(admin_ip_detail_handler))
}

/// 获取 IP 维度指标列表
async fn admin_ip_metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<IpMetricsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let window = params.window.unwrap_or(TimeWindow::OneHour);
    let limit = params.limit.unwrap_or(100).min(1000);

    match state.observability.snapshot_ip_metrics(window) {
        Some(metrics) => json_ok(&metrics),
        None => json_error(StatusCode::NOT_FOUND, "ip_metrics_not_available"),
    }
}

/// 获取特定 IP 的详细指标
async fn admin_ip_detail_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ip): Path<String>,
    Query(params): Query<IpMetricsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let window = params.window.unwrap_or(TimeWindow::TwentyFourHours);

    // 查询特定 IP 的详细数据
    // ...

    json_ok(&detail)
}

#[derive(Deserialize)]
struct IpMetricsQuery {
    window: Option<TimeWindow>,
    limit: Option<usize>,
}
```

## 6. 配置示例

```yaml
# config.yaml
observability:
  metrics:
    enabled: true
    path: /metrics
    token: ${METRICS_TOKEN}

  ip_metrics:
    enabled: true
    db_path: ./data/ip_metrics.db
    buffer_size: 1000
    flush_interval_secs: 60
```

## 7. 依赖添加

```toml
# Cargo.toml
[dependencies]
# 现有依赖...

# IP 监控依赖
rusqlite = { version = "0.32", features = ["bundled", "chrono"] }
```

## 8. 性能考虑

1. **内存缓冲**：批量写入减少 I/O 操作
2. **索引优化**：为常用查询字段创建索引
3. **异步刷盘**：后台任务处理数据持久化
4. **分层聚合**：预聚合减少实时计算
5. **定期清理**：防止数据无限增长

## 9. 安全考虑

1. **IP 隐私**：敏感环境考虑 IP 哈希化存储
2. **Token 脱敏**：使用现有的 `token_label` 函数
3. **访问控制**：Admin API 需要认证
4. **SQL 注入**：使用参数化查询

## 10. 后续扩展

1. 支持按地理位置聚合 IP 数据
2. 添加 IP 黑名单/限流集成
3. 异常检测（识别异常流量模式）
4. 数据导出（CSV/JSON 格式）

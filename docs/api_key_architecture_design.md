# API Key 管理架构设计文档

## 1. 设计目标

- API Key 从路由独立出来，改为全局配置
- 每个 API Key 支持独立的限流和并发配置
- 支持梯度封禁规则和状态管理
- 封禁日志持久化
- 保持向后兼容
- 配置继承机制：api_key级 > 路由级 > 全局级

---

## 2. 新的配置结构定义

### 2.1 ApiKeyConfig - API Key 完整配置

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    /// 唯一标识符（用于管理）
    pub id: String,
    /// 关联的路由ID列表（None 表示所有路由）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_ids: Option<Vec<String>>,
    /// API Key 值
    pub key: String,
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 备注说明
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remark: String,
    /// 限流配置（覆盖全局/路由级）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitConfig>,
    /// 并发配置（覆盖全局/路由级）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ApiKeyConcurrencyConfig>,
    /// 封禁规则
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ban_rules: Vec<BanRule>,
    /// 当前封禁状态（运行时会更新）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ban_status: Option<BanStatus>,
}
```

### 2.2 BanRule - 梯度封禁规则

```rust
/// 梯度封禁规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanRule {
    /// 规则ID
    pub id: String,
    /// 规则名称
    pub name: String,
    /// 触发条件
    pub condition: BanCondition,
    /// 封禁时长（秒）
    pub ban_duration_secs: u64,
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// 封禁触发条件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BanCondition {
    /// 错误率超过阈值
    ErrorRate {
        /// 时间窗口（秒）
        window_secs: u64,
        /// 错误率阈值（0.0-1.0）
        threshold: f64,
        /// 最小请求数（避免样本过少）
        min_requests: u64,
    },
    /// 请求数超过阈值
    RequestCount {
        /// 时间窗口（秒）
        window_secs: u64,
        /// 最大请求数
        max_requests: u64,
    },
    /// 连续错误数
    ConsecutiveErrors {
        /// 连续错误数阈值
        count: u32,
    },
}
```

### 2.3 BanStatus - 封禁状态

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanStatus {
    /// 是否被封禁
    pub is_banned: bool,
    /// 封禁开始时间（Unix秒）
    pub banned_at: Option<u64>,
    /// 封禁结束时间（Unix秒）
    pub banned_until: Option<u64>,
    /// 触发封禁的规则ID
    pub triggered_rule_id: Option<String>,
    /// 封禁原因
    pub reason: Option<String>,
    /// 历史封禁次数
    pub ban_count: u32,
}

impl Default for BanStatus {
    fn default() -> Self {
        Self {
            is_banned: false,
            banned_at: None,
            banned_until: None,
            triggered_rule_id: None,
            reason: None,
            ban_count: 0,
        }
    }
}
```

### 2.4 BanLogEntry - 封禁日志条目

```rust
/// 封禁日志条目（用于持久化）
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

/// 封禁时的指标快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanMetricsSnapshot {
    /// 时间窗口内的请求数
    pub requests: u64,
    /// 时间窗口内的错误数
    pub errors: u64,
    /// 错误率
    pub error_rate: f64,
}
```

### 2.5 ApiKeyConcurrencyConfig - API Key 级别并发配置

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConcurrencyConfig {
    /// 该API Key的最大并发请求数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inflight: Option<usize>,
}
```

### 2.6 修改后的 GatewayAuthConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayAuthConfig {
    /// 简写格式：仅包含 key 字符串列表（向后兼容）
    #[serde(alias = "tokens", default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<String>,
    /// 完整格式：包含详细配置的 API Key 列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_key_configs: Vec<ApiKeyConfig>,
    /// Token 来源配置
    #[serde(default = "default_token_sources")]
    pub token_sources: Vec<TokenSourceConfig>,
}
```

### 2.7 修改后的 AppConfig（新增 api_keys 字段）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub listen: String,
    pub gateway_auth: GatewayAuthConfig,
    pub routes: Vec<RouteConfig>,
    /// 全局 API Key 配置（独立管理）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_keys: Option<ApiKeysGlobalConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbound_tls: Option<InboundTlsConfig>,
    // ... 其他字段保持不变
}

/// API Key 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeysGlobalConfig {
    /// API Key 列表
    pub keys: Vec<ApiKeyConfig>,
    /// 封禁日志持久化配置
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ban_log: Option<BanLogConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanLogConfig {
    /// SQLite 数据库路径
    #[serde(default = "default_ban_log_path")]
    pub path: String,
    /// 日志保留天数
    #[serde(default = "default_ban_log_retention_days")]
    pub retention_days: u32,
}
```

---

## 3. 向后兼容的迁移策略

### 3.1 配置解析阶段合并

```rust
impl AppConfig {
    /// 获取所有有效的 API Key 配置（合并简写格式和完整格式）
    pub fn resolved_api_keys(&self) -> Vec<ResolvedApiKey> {
        let mut resolved = Vec::new();

        // 1. 处理 gateway_auth.api_keys（简写格式）
        for key in &self.gateway_auth.api_keys {
            resolved.push(ResolvedApiKey {
                id: generate_key_id(key),
                key: key.clone(),
                route_ids: None, // 所有路由
                enabled: true,
                remark: String::new(),
                rate_limit: None,
                concurrency: None,
                ban_rules: Vec::new(),
                ban_status: None,
            });
        }

        // 2. 处理 gateway_auth.api_key_configs（完整格式）
        for config in &self.gateway_auth.api_key_configs {
            resolved.push(ResolvedApiKey::from_config(config));
        }

        // 3. 处理 api_keys.keys（新的全局配置）
        if let Some(global) = &self.api_keys {
            for config in &global.keys {
                resolved.push(ResolvedApiKey::from_config(config));
            }
        }

        // 4. 处理 route.api_keys（路由级白名单，转换为限制路由的key）
        for route in &self.routes {
            if let Some(route_keys) = &route.api_keys {
                for key in route_keys {
                    // 检查是否已存在
                    if !resolved.iter().any(|r| r.key == *key) {
                        resolved.push(ResolvedApiKey {
                            id: generate_key_id(key),
                            key: key.clone(),
                            route_ids: Some(vec![route.id.clone()]),
                            enabled: true,
                            remark: format!("Migrated from route {}", route.id),
                            rate_limit: None,
                            concurrency: None,
                            ban_rules: Vec::new(),
                            ban_status: None,
                        });
                    }
                }
            }
        }

        resolved
    }
}

/// 运行时使用的统一 API Key 结构
#[derive(Debug, Clone)]
pub struct ResolvedApiKey {
    pub id: String,
    pub key: String,
    pub route_ids: Option<Vec<String>>,
    pub enabled: bool,
    pub remark: String,
    pub rate_limit: Option<RateLimitConfig>,
    pub concurrency: Option<ApiKeyConcurrencyConfig>,
    pub ban_rules: Vec<BanRule>,
    pub ban_status: Option<BanStatus>,
}
```

### 3.2 配置验证策略

```rust
impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        // ... 现有验证逻辑 ...

        // 验证 api_key_configs 中的配置
        for key_config in &self.gateway_auth.api_key_configs {
            Self::validate_api_key_config(key_config)?;
        }

        // 验证全局 api_keys
        if let Some(global) = &self.api_keys {
            for key_config in &global.keys {
                Self::validate_api_key_config(key_config)?;
            }

            // 验证 key ID 唯一性
            let mut ids = HashSet::new();
            for key in &global.keys {
                if !ids.insert(key.id.clone()) {
                    return Err(ConfigError::Validation(
                        format!("duplicate api_key id: {}", key.id)
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_api_key_config(config: &ApiKeyConfig) -> Result<(), ConfigError> {
        if config.id.trim().is_empty() {
            return Err(ConfigError::Validation(
                "api_key id must not be empty".to_string()
            ));
        }
        if config.key.trim().is_empty() {
            return Err(ConfigError::Validation(
                format!("api_key {}: key must not be empty", config.id)
            ));
        }

        // 验证 ban_rules
        for rule in &config.ban_rules {
            Self::validate_ban_rule(rule)?;
        }

        Ok(())
    }

    fn validate_ban_rule(rule: &BanRule) -> Result<(), ConfigError> {
        match &rule.condition {
            BanCondition::ErrorRate { threshold, .. } => {
                if !(0.0..=1.0).contains(threshold) {
                    return Err(ConfigError::Validation(
                        format!("ban_rule {}: error_rate threshold must be in [0.0, 1.0]", rule.id)
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

---

## 4. 配置继承机制

### 4.1 继承优先级

```rust
/// 获取 API Key 在指定路由上的有效限流配置
pub fn resolve_rate_limit(
    api_key: &ResolvedApiKey,
    route: &RouteConfig,
    global: &Option<RateLimitConfig>,
) -> Option<RateLimitConfig> {
    // 优先级1: API Key 级别
    if let Some(limit) = &api_key.rate_limit {
        return Some(limit.clone());
    }

    // 优先级2: 路由级别（未来可扩展）
    // if let Some(limit) = &route.rate_limit { ... }

    // 优先级3: 全局级别
    global.clone()
}

/// 获取 API Key 在指定路由上的有效并发配置
pub fn resolve_concurrency(
    api_key: &ResolvedApiKey,
    route: &RouteConfig,
    global: &Option<ConcurrencyConfig>,
) -> Option<ApiKeyConcurrencyConfig> {
    // 优先级1: API Key 级别
    if let Some(cfg) = &api_key.concurrency {
        return Some(cfg.clone());
    }

    // 优先级2: 路由级别
    if let Some(limit) = route.upstream.upstream_key_max_inflight {
        return Some(ApiKeyConcurrencyConfig {
            max_inflight: Some(limit),
        });
    }

    // 优先级3: 全局级别
    global.as_ref().and_then(|g| {
        g.upstream_per_key_max_inflight.map(|limit| {
            ApiKeyConcurrencyConfig {
                max_inflight: Some(limit),
            }
        })
    })
}
```

---

## 5. 关键接口设计

### 5.1 API Key 管理接口（Admin API）

```rust
// src/admin/api_keys.rs

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put, delete},
    Json,
};

/// 列出所有 API Keys
/// GET /admin/api/keys
async fn list_api_keys(
    State(state): State<AppState>,
    Query(filter): Query<ApiKeyFilter>,
) -> Response<Body> {
    // 支持筛选: route_id, enabled, banned
    // 支持搜索: key, remark (模糊匹配)
}

/// 获取单个 API Key 详情
/// GET /admin/api/keys/:id
async fn get_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response<Body> {
    // 返回完整配置 + 当前状态 + 统计信息
}

/// 创建 API Key
/// POST /admin/api/keys
async fn create_api_key(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Response<Body> {
    // 生成唯一ID和随机key（如果未提供）
}

/// 更新 API Key
/// PUT /admin/api/keys/:id
async fn update_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiKeyRequest>,
) -> Response<Body> {
    // 支持部分更新
}

/// 删除 API Key
/// DELETE /admin/api/keys/:id
async fn delete_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response<Body>;

/// 手动封禁 API Key
/// POST /admin/api/keys/:id/ban
async fn ban_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<BanRequest>,
) -> Response<Body>;

/// 手动解封 API Key
/// POST /admin/api/keys/:id/unban
async fn unban_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response<Body>;

/// 获取封禁日志
/// GET /admin/api/keys/:id/ban-logs
async fn get_ban_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(pagination): Query<PaginationParams>,
) -> Response<Body>;
```

### 5.2 请求处理接口

```rust
// src/api_keys/manager.rs

/// API Key 管理器（运行时状态）
pub struct ApiKeyManager {
    /// 所有 API Key 配置
    keys: RwLock<HashMap<String, ResolvedApiKey>>,
    /// 按 key 值索引到 ID
    key_index: RwLock<HashMap<String, String>>,
    /// 限流器（按 API Key ID）
    rate_limiters: RwLock<HashMap<String, Arc<RateLimiter>>>,
    /// 并发控制器（按 API Key ID）
    concurrency_controllers: RwLock<HashMap<String, Arc<Semaphore>>>,
    /// 封禁日志存储
    ban_log_store: Option<Arc<dyn BanLogStore>>,
}

impl ApiKeyManager {
    /// 验证 API Key 是否有效且可访问指定路由
    pub async fn validate_key(
        &self,
        key_value: &str,
        route_id: &str,
    ) -> Result<ValidationResult, ValidationError> {
        // 1. 查找 key
        let key_id = self.key_index.read().await
            .get(key_value)
            .cloned()
            .ok_or(ValidationError::InvalidKey)?;

        let keys = self.keys.read().await;
        let key = keys.get(&key_id).ok_or(ValidationError::InvalidKey)?;

        // 2. 检查是否启用
        if !key.enabled {
            return Err(ValidationError::KeyDisabled);
        }

        // 3. 检查封禁状态
        if let Some(status) = &key.ban_status {
            if status.is_banned {
                // 检查是否到期
                if let Some(until) = status.banned_until {
                    let now = current_epoch_seconds();
                    if now < until {
                        return Err(ValidationError::KeyBanned {
                            until,
                            reason: status.reason.clone(),
                        });
                    }
                }
            }
        }

        // 4. 检查路由权限
        if let Some(allowed_routes) = &key.route_ids {
            if !allowed_routes.contains(&route_id.to_string()) {
                return Err(ValidationError::RouteNotAllowed);
            }
        }

        Ok(ValidationResult {
            key_id: key_id.clone(),
            key: key.clone(),
        })
    }

    /// 检查限流
    pub fn check_rate_limit(&self, key_id: &str, route_id: &str) -> RateLimitDecision {
        // 获取或创建限流器
        // 使用 API Key 级别的配置
    }

    /// 获取并发许可
    pub async fn acquire_concurrency_permit(
        &self,
        key_id: &str,
    ) -> Result<OwnedSemaphorePermit, ConcurrencyError> {
        // 获取或创建信号量
        // 使用 API Key 级别的配置
    }

    /// 上报请求结果（用于封禁规则检查）
    pub async fn report_request_result(
        &self,
        key_id: &str,
        result: RequestResult,
    ) {
        // 更新指标
        // 检查封禁规则
        // 如果触发封禁，更新状态并记录日志
    }
}

/// 验证结果
pub struct ValidationResult {
    pub key_id: String,
    pub key: ResolvedApiKey,
}

/// 请求结果
pub struct RequestResult {
    pub success: bool,
    pub latency_ms: u64,
    pub response_status: u16,
}
```

### 5.3 封禁日志存储接口

```rust
// src/api_keys/ban_log.rs

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

    /// 查询活跃的封禁（未解封且未过期）
    async fn query_active_bans(
        &self,
        before: u64,
    ) -> Result<Vec<BanLogEntry>, BanLogError>;

    /// 清理过期日志
    async fn cleanup_old_entries(&self, before: u64) -> Result<u64, BanLogError>;
}

/// SQLite 实现
pub struct SqliteBanLogStore {
    pool: SqlitePool,
}

#[async_trait]
impl BanLogStore for SqliteBanLogStore {
    // ... 实现
}
```

---

## 6. 配置示例

### 6.1 新的完整配置格式

```yaml
listen: "127.0.0.1:8080"

gateway_auth:
  # 简写格式（向后兼容）
  api_keys:
    - "legacy_key_1"
    - "legacy_key_2"
  token_sources:
    - type: "authorization_bearer"

# 新的 API Key 全局配置
api_keys:
  keys:
    - id: "key_001"
      key: "sk_prod_abc123"
      enabled: true
      remark: "生产环境主密钥"
      route_ids: ["openai", "anthropic"]  # 限制只能访问这些路由
      rate_limit:
        per_minute: 600
      concurrency:
        max_inflight: 10
      ban_rules:
        - id: "rule_001"
          name: "高错误率封禁"
          condition:
            type: "error_rate"
            window_secs: 300
            threshold: 0.5
            min_requests: 10
          ban_duration_secs: 3600
          enabled: true
        - id: "rule_002"
          name: "请求过多封禁"
          condition:
            type: "request_count"
            window_secs: 60
            max_requests: 10000
          ban_duration_secs: 1800
          enabled: true
      ban_status:  # 运行时会自动更新
        is_banned: false
        ban_count: 0

    - id: "key_002"
      key: "sk_test_xyz789"
      enabled: true
      remark: "测试环境密钥"
      # route_ids 不设置 = 可以访问所有路由
      rate_limit:
        per_minute: 60
      ban_rules: []

  ban_log:
    path: "./data/ban_logs.db"
    retention_days: 90

routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      strip_prefix: true
      inject_headers:
        - name: "authorization"
          value: "Bearer sk-openai-upstream"

# 全局默认值
rate_limit:
  per_minute: 120

concurrency:
  downstream_max_inflight: 100
  upstream_per_key_max_inflight: 8
```

---

## 7. 数据结构关系图

```
AppConfig
├── gateway_auth: GatewayAuthConfig
│   ├── api_keys: Vec<String>              # 简写格式（向后兼容）
│   ├── api_key_configs: Vec<ApiKeyConfig> # 完整格式（新）
│   └── token_sources: Vec<TokenSourceConfig>
│
├── api_keys: ApiKeysGlobalConfig           # 新的独立配置（推荐）
│   ├── keys: Vec<ApiKeyConfig>
│   │   ├── id: String
│   │   ├── route_ids: Option<Vec<String>>
│   │   ├── key: String
│   │   ├── enabled: bool
│   │   ├── remark: String
│   │   ├── rate_limit: Option<RateLimitConfig>
│   │   ├── concurrency: Option<ApiKeyConcurrencyConfig>
│   │   ├── ban_rules: Vec<BanRule>
│   │   │   ├── id: String
│   │   │   ├── name: String
│   │   │   ├── condition: BanCondition
│   │   │   │   ├── ErrorRate { window_secs, threshold, min_requests }
│   │   │   │   ├── RequestCount { window_secs, max_requests }
│   │   │   │   └── ConsecutiveErrors { count }
│   │   │   ├── ban_duration_secs: u64
│   │   │   └── enabled: bool
│   │   └── ban_status: Option<BanStatus>
│   │       ├── is_banned: bool
│   │       ├── banned_at: Option<u64>
│   │       ├── banned_until: Option<u64>
│   │       ├── triggered_rule_id: Option<String>
│   │       ├── reason: Option<String>
│   │       └── ban_count: u32
│   │
│   └── ban_log: Option<BanLogConfig>
│       ├── path: String
│       └── retention_days: u32
│
└── routes: Vec<RouteConfig>
    ├── id: String
    ├── prefix: String
    ├── api_keys: Option<Vec<String>>       # 向后兼容
    └── upstream: UpstreamConfig

BanLogEntry（持久化）
├── id: String
├── api_key_id: String
├── rule_id: String
├── reason: String
├── banned_at: u64
├── banned_until: u64
├── unbanned_at: Option<u64>
└── metrics_snapshot: BanMetricsSnapshot
    ├── requests: u64
    ├── errors: u64
    └── error_rate: f64
```

---

## 8. 关键设计决策

1. **向后兼容策略**：保留旧字段，通过 `resolved_api_keys()` 统一转换为新格式
2. **配置位置**：支持 `gateway_auth.api_key_configs` 和独立的 `api_keys` 两种位置
3. **路由权限**：使用 `route_ids: Option<Vec<String>>` 控制，None 表示所有路由
4. **封禁状态持久化**：`ban_status` 存储在配置中，重启后保持
5. **封禁日志独立**：使用单独的 SQLite 数据库，避免影响主配置
6. **配置继承**：api_key级 > 路由级 > 全局级，便于灵活配置

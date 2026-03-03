use http::HeaderValue;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub listen: String,
    pub gateway_auth: GatewayAuthConfig,
    pub routes: Vec<RouteConfig>,
    /// API Key 全局配置（独立管理）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_keys: Option<ApiKeysGlobalConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbound_tls: Option<InboundTlsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<CorsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<ConcurrencyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observability: Option<ObservabilityConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<AdminConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayAuthConfig {
    #[serde(default = "default_token_sources")]
    pub token_sources: Vec<TokenSourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenSourceConfig {
    AuthorizationBearer,
    Header { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub prefix: String,
    pub upstream: UpstreamConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    pub base_url: String,
    #[serde(default = "default_true")]
    pub strip_prefix: bool,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inject_headers: Vec<HeaderInjection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub forward_xff: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<UpstreamProxyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_key_max_inflight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderInjection {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamProxyConfig {
    pub protocol: ProxyProtocol,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundTlsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default = "default_self_signed_cert_path")]
    pub self_signed_cert_path: String,
    #[serde(default = "default_self_signed_key_path")]
    pub self_signed_key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_headers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expose_headers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub per_minute: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream_max_inflight: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_per_key_max_inflight: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_observability_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: LogFormat,
    #[serde(default = "default_true")]
    pub to_stdout: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<LogFileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Json,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogFileConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_log_file_dir")]
    pub dir: String,
    #[serde(default = "default_log_file_prefix")]
    pub prefix: String,
    #[serde(default = "default_log_rotation")]
    pub rotation: LogRotation,
    #[serde(default = "default_log_file_max_files")]
    pub max_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogRotation {
    Minutely,
    Hourly,
    #[default]
    Daily,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default = "default_metrics_path")]
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// SQLite persistence configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite: Option<MetricsSqliteConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsSqliteConfig {
    #[serde(default = "default_sqlite_path")]
    pub path: String,
    /// Flush interval in seconds (default: 60)
    #[serde(default = "default_sqlite_flush_interval_secs")]
    pub flush_interval_secs: u64,
    /// Maximum batch size before flush (default: 1000)
    #[serde(default = "default_sqlite_batch_size")]
    pub batch_size: usize,
    /// Retention days for detailed records (default: 7)
    #[serde(default = "default_sqlite_retention_days")]
    pub retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default = "default_trace_sample_ratio")]
    pub sample_ratio: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp: Option<OtlpConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpConfig {
    pub endpoint: String,
    #[serde(default = "default_otlp_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default = "default_admin_path_prefix")]
    pub path_prefix: String,
}

// ==================== API Key Management ====================

/// API Key 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeysGlobalConfig {
    /// API Key 列表
    pub keys: Vec<ApiKeyConfig>,
    /// 全局封禁规则（对所有 API Key 生效）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ban_rules: Vec<BanRule>,
    /// 封禁日志存储配置
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite: Option<ApiKeysSqliteConfig>,
}

/// API Key 封禁日志 SQLite 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeysSqliteConfig {
    /// 数据库文件路径
    #[serde(default = "default_ban_log_db_path")]
    pub path: String,
}

fn default_ban_log_db_path() -> String {
    "./data/ban_logs.db".to_string()
}

/// API Key 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    /// 唯一标识符
    pub id: String,
    /// 关联的路由ID（None 表示所有路由）
    /// 注意：route_ids 优先级高于 route_id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    /// 关联的多个路由ID（None 表示所有路由）
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
    /// 封禁状态
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ban_status: Option<BanStatus>,
}

/// 封禁规则
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
    /// 触发次数阈值：在 trigger_window_secs 内触发多少次后才执行封禁
    #[serde(default = "default_one")]
    pub trigger_count_threshold: u32,
    /// 触发计数窗口（秒）：统计触发次数的时间窗口
    #[serde(default = "default_trigger_window")]
    pub trigger_window_secs: u64,
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

/// 封禁状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

/// API Key 级别的并发配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConcurrencyConfig {
    /// 该API Key的最大并发请求数（下游）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream_max_inflight: Option<usize>,
    /// 上游每个key的最大并发
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_per_key_max_inflight: Option<usize>,
}

/// 运行时解析后的 API Key 配置
#[derive(Debug, Clone)]
pub struct ResolvedApiKey {
    pub id: String,
    pub key: String,
    pub route_id: Option<String>,
    pub route_ids: Option<Vec<String>>,
    pub enabled: bool,
    pub remark: String,
    pub rate_limit: Option<RateLimitConfig>,
    pub concurrency: Option<ApiKeyConcurrencyConfig>,
    pub ban_rules: Vec<BanRule>,
    pub ban_status: Option<BanStatus>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_observability_log_level(),
            format: default_log_format(),
            to_stdout: true,
            file: None,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            token: String::new(),
            sqlite: None,
        }
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_ratio: default_trace_sample_ratio(),
            otlp: None,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    MissingEnvVar(String),
    Validation(String),
}

/// 生成 API Key 的 ID（基于 key 的 hash）
fn generate_key_id(key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    format!("ak-{:016x}", hasher.finish())
}

impl ResolvedApiKey {
    /// 从配置创建
    pub fn from_config(config: &ApiKeyConfig) -> Self {
        Self {
            id: config.id.clone(),
            key: config.key.clone(),
            route_id: config.route_id.clone(),
            route_ids: config.route_ids.clone(),
            enabled: config.enabled,
            remark: config.remark.clone(),
            rate_limit: config.rate_limit.clone(),
            concurrency: config.concurrency.clone(),
            ban_rules: config.ban_rules.clone(),
            ban_status: config.ban_status.clone(),
        }
    }

    /// 从简写格式创建（向后兼容）
    pub fn from_key_string(key: &str) -> Self {
        Self {
            id: generate_key_id(key),
            key: key.to_string(),
            route_id: None,
            route_ids: None,
            enabled: true,
            remark: String::new(),
            rate_limit: None,
            concurrency: None,
            ban_rules: Vec::new(),
            ban_status: None,
        }
    }
}

impl AppConfig {
    /// 获取所有解析后的 API Key 配置
    pub fn resolved_api_keys(&self) -> Vec<ResolvedApiKey> {
        let mut resolved = Vec::new();
        let mut seen_keys = HashSet::new();

        // 处理 api_keys.keys（全局配置）
        if let Some(global) = &self.api_keys {
            for config in &global.keys {
                if seen_keys.insert(config.key.clone()) {
                    resolved.push(ResolvedApiKey::from_config(config));
                }
            }
        }

        resolved
    }

    /// 获取指定 API Key 的有效限流配置（继承机制：api_key级 > 全局级）
    pub fn resolve_rate_limit(
        &self,
        api_key: &ResolvedApiKey,
    ) -> Option<RateLimitConfig> {
        // 优先级1: API Key 级别
        if let Some(limit) = &api_key.rate_limit {
            return Some(limit.clone());
        }

        // 优先级2: 全局级别
        self.rate_limit.clone()
    }

    /// 获取指定 API Key 的有效并发配置（继承机制：api_key级 > 路由级 > 全局级）
    pub fn resolve_concurrency(
        &self,
        api_key: &ResolvedApiKey,
        route: &RouteConfig,
    ) -> Option<ApiKeyConcurrencyConfig> {
        // 优先级1: API Key 级别
        if api_key.concurrency.is_some() {
            return api_key.concurrency.clone();
        }

        // 优先级2: 路由级别
        if route.upstream.upstream_key_max_inflight.is_some() {
            return Some(ApiKeyConcurrencyConfig {
                downstream_max_inflight: None,
                upstream_per_key_max_inflight: route.upstream.upstream_key_max_inflight,
            });
        }

        // 优先级3: 全局级别
        self.concurrency.as_ref().map(|c| ApiKeyConcurrencyConfig {
            downstream_max_inflight: c.downstream_max_inflight,
            upstream_per_key_max_inflight: c.upstream_per_key_max_inflight,
        })
    }
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_yaml_str(&raw)
    }

    pub fn from_yaml_str(yaml: &str) -> Result<Self, ConfigError> {
        let interpolated = interpolate_env_vars(yaml)?;
        let config: Self = serde_yaml::from_str(&interpolated).map_err(ConfigError::Yaml)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listen.trim().is_empty() {
            return Err(ConfigError::Validation(
                "`listen` must not be empty".to_string(),
            ));
        }

        // 检查至少配置了一个 API Key
        let has_global_api_keys = self
            .api_keys
            .as_ref()
            .map(|g| !g.keys.is_empty())
            .unwrap_or(false);

        if !has_global_api_keys {
            return Err(ConfigError::Validation(
                "must configure at least one API key in `api_keys.keys`".to_string(),
            ));
        }

        // 验证 api_keys.keys
        if let Some(global) = &self.api_keys {
            let mut ids = HashSet::new();
            for key_config in &global.keys {
                if key_config.id.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "api_key id must not be empty".to_string(),
                    ));
                }
                if !ids.insert(key_config.id.clone()) {
                    return Err(ConfigError::Validation(format!(
                        "duplicate api_key id: {}",
                        key_config.id
                    )));
                }
                if key_config.key.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "api_key {}: key must not be empty",
                        key_config.id
                    )));
                }
                // 验证限流配置
                if let Some(rate_limit) = &key_config.rate_limit {
                    if rate_limit.per_minute == 0 {
                        return Err(ConfigError::Validation(format!(
                            "api_key {}: rate_limit.per_minute must be > 0",
                            key_config.id
                        )));
                    }
                }
                // 验证并发配置
                if let Some(concurrency) = &key_config.concurrency {
                    if let Some(limit) = concurrency.downstream_max_inflight {
                        if limit == 0 {
                            return Err(ConfigError::Validation(format!(
                                "api_key {}: concurrency.downstream_max_inflight must be > 0",
                                key_config.id
                            )));
                        }
                    }
                    if let Some(limit) = concurrency.upstream_per_key_max_inflight {
                        if limit == 0 {
                            return Err(ConfigError::Validation(format!(
                                "api_key {}: concurrency.upstream_per_key_max_inflight must be > 0",
                                key_config.id
                            )));
                        }
                    }
                }
            }
        }

        if self.routes.is_empty() {
            return Err(ConfigError::Validation(
                "`routes` must contain at least one route".to_string(),
            ));
        }

        let mut ids = HashSet::new();
        let mut prefixes = HashSet::new();
        let mut has_route_upstream_key_concurrency = false;

        for route in &self.routes {
            if route.id.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "route `id` must not be empty".to_string(),
                ));
            }

            if !ids.insert(route.id.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate route id `{}`",
                    route.id
                )));
            }

            if !route.prefix.starts_with('/') {
                return Err(ConfigError::Validation(format!(
                    "route `{}` prefix must start with `/`",
                    route.id
                )));
            }

            if route.prefix.len() > 1 && route.prefix.ends_with('/') {
                return Err(ConfigError::Validation(format!(
                    "route `{}` prefix must not end with `/`",
                    route.id
                )));
            }

            if !prefixes.insert(route.prefix.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate route prefix `{}`",
                    route.prefix
                )));
            }

            if route.upstream.base_url.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.base_url must not be empty",
                    route.id
                )));
            }

            if route.upstream.connect_timeout_ms == 0 {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.connect_timeout_ms must be > 0",
                    route.id
                )));
            }

            if route.upstream.request_timeout_ms == 0 {
                return Err(ConfigError::Validation(format!(
                    "route `{}` upstream.request_timeout_ms must be > 0",
                    route.id
                )));
            }

            if let Some(user_agent) = route.upstream.user_agent.as_deref() {
                if user_agent.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.user_agent must not be empty when provided",
                        route.id
                    )));
                }
                if HeaderValue::from_str(user_agent).is_err() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.user_agent must be a valid header value",
                        route.id
                    )));
                }
            }

            if let Some(limit) = route.upstream.upstream_key_max_inflight {
                has_route_upstream_key_concurrency = true;
                if limit == 0 {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.upstream_key_max_inflight must be > 0 when provided",
                        route.id
                    )));
                }
            }

            if let Some(proxy) = &route.upstream.proxy {
                if proxy.address.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` upstream.proxy.address must not be empty",
                        route.id
                    )));
                }

                match (&proxy.username, &proxy.password) {
                    (Some(username), Some(password))
                        if username.trim().is_empty() || password.trim().is_empty() =>
                    {
                        return Err(ConfigError::Validation(format!(
                            "route `{}` upstream.proxy.username/password must not be empty",
                            route.id
                        )));
                    }
                    (Some(_), Some(_)) | (None, None) => {}
                    _ => {
                        return Err(ConfigError::Validation(format!(
                            "route `{}` upstream.proxy.username and upstream.proxy.password must be set together",
                            route.id
                        )));
                    }
                }
            }

            for header in &route.upstream.inject_headers {
                if header.name.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` has empty inject_headers.name",
                        route.id
                    )));
                }
            }

        }

        let mut has_global_upstream_key_concurrency = false;
        if let Some(rate_limit) = &self.rate_limit
            && rate_limit.per_minute == 0
        {
            return Err(ConfigError::Validation(
                "`rate_limit.per_minute` must be > 0".to_string(),
            ));
        }

        if let Some(concurrency) = &self.concurrency {
            if let Some(limit) = concurrency.downstream_max_inflight
                && limit == 0
            {
                return Err(ConfigError::Validation(
                    "`concurrency.downstream_max_inflight` must be > 0 when provided".to_string(),
                ));
            }

            if let Some(limit) = concurrency.upstream_per_key_max_inflight {
                has_global_upstream_key_concurrency = true;
                if limit == 0 {
                    return Err(ConfigError::Validation(
                        "`concurrency.upstream_per_key_max_inflight` must be > 0 when provided"
                            .to_string(),
                    ));
                }
            }
        }

        if has_global_upstream_key_concurrency || has_route_upstream_key_concurrency {
            for route in &self.routes {
                let route_uses_upstream_key_concurrency = has_global_upstream_key_concurrency
                    || route.upstream.upstream_key_max_inflight.is_some();
                if !route_uses_upstream_key_concurrency {
                    continue;
                }

                if !route_has_upstream_key_injection(route) {
                    return Err(ConfigError::Validation(format!(
                        "route `{}` must configure `upstream.inject_headers` with one of {:?} when upstream key concurrency is enabled",
                        route.id,
                        upstream_key_header_names(),
                    )));
                }
            }
        }

        if let Some(tls) = &self.inbound_tls {
            validate_optional_path(
                tls.cert_path.as_deref(),
                "`inbound_tls.cert_path` must not be empty when provided",
            )?;
            validate_optional_path(
                tls.key_path.as_deref(),
                "`inbound_tls.key_path` must not be empty when provided",
            )?;

            if tls.self_signed_cert_path.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`inbound_tls.self_signed_cert_path` must not be empty".to_string(),
                ));
            }
            if tls.self_signed_key_path.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`inbound_tls.self_signed_key_path` must not be empty".to_string(),
                ));
            }

            match (&tls.cert_path, &tls.key_path) {
                (Some(_), Some(_)) | (None, None) => {}
                _ => {
                    return Err(ConfigError::Validation(
                        "`inbound_tls.cert_path` and `inbound_tls.key_path` must be set together"
                            .to_string(),
                    ));
                }
            }
        }

        if let Some(observability) = &self.observability {
            if observability.logging.level.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`observability.logging.level` must not be empty".to_string(),
                ));
            }
            if !observability.logging.to_stdout
                && !matches!(
                    observability.logging.file.as_ref(),
                    Some(file) if file.enabled
                )
            {
                return Err(ConfigError::Validation(
                    "`observability.logging` must enable at least one sink (`to_stdout` or `file.enabled`)"
                        .to_string(),
                ));
            }
            if let Some(file) = &observability.logging.file
                && file.enabled
            {
                if file.dir.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.dir` must not be empty".to_string(),
                    ));
                }
                if file.prefix.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.prefix` must not be empty".to_string(),
                    ));
                }
                if file.max_files == 0 {
                    return Err(ConfigError::Validation(
                        "`observability.logging.file.max_files` must be > 0".to_string(),
                    ));
                }
            }

            if !observability.metrics.path.starts_with('/') {
                return Err(ConfigError::Validation(
                    "`observability.metrics.path` must start with `/`".to_string(),
                ));
            }
            if observability.metrics.path == "/healthz" {
                return Err(ConfigError::Validation(
                    "`observability.metrics.path` must not conflict with `/healthz`".to_string(),
                ));
            }
            if observability.metrics.enabled && observability.metrics.token.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`observability.metrics.token` must not be empty when metrics are enabled"
                        .to_string(),
                ));
            }

            if !(0.0..=1.0).contains(&observability.tracing.sample_ratio) {
                return Err(ConfigError::Validation(
                    "`observability.tracing.sample_ratio` must be within [0.0, 1.0]".to_string(),
                ));
            }

            if let Some(otlp) = &observability.tracing.otlp {
                if otlp.endpoint.trim().is_empty() {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.endpoint` must not be empty".to_string(),
                    ));
                }
                if reqwest::Url::parse(otlp.endpoint.trim()).is_err() {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.endpoint` must be a valid URL".to_string(),
                    ));
                }
                if otlp.timeout_ms == 0 {
                    return Err(ConfigError::Validation(
                        "`observability.tracing.otlp.timeout_ms` must be > 0".to_string(),
                    ));
                }
            }
        }

        if let Some(admin) = &self.admin {
            if admin.enabled && admin.token.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "`admin.token` must not be empty when admin is enabled".to_string(),
                ));
            }
            if !admin.path_prefix.starts_with('/') {
                return Err(ConfigError::Validation(
                    "`admin.path_prefix` must start with `/`".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Yaml(err) => write!(f, "yaml parse error: {err}"),
            Self::MissingEnvVar(name) => write!(f, "missing environment variable `{name}`"),
            Self::Validation(msg) => write!(f, "config validation error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn interpolate_env_vars(input: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(rel_start) = input[cursor..].find("${") {
        let start = cursor + rel_start;
        out.push_str(&input[cursor..start]);

        let key_start = start + 2;
        let rel_end = input[key_start..].find('}').ok_or_else(|| {
            ConfigError::Validation("unterminated `${...}` expression".to_string())
        })?;
        let end = key_start + rel_end;
        let key = &input[key_start..end];

        if key.is_empty() {
            return Err(ConfigError::Validation(
                "empty environment variable name in `${}`".to_string(),
            ));
        }

        let value = env::var(key).map_err(|_| ConfigError::MissingEnvVar(key.to_string()))?;
        out.push_str(&value);
        cursor = end + 1;
    }

    out.push_str(&input[cursor..]);
    Ok(out)
}

fn default_true() -> bool {
    true
}

fn default_one() -> u32 {
    1
}

fn default_trigger_window() -> u64 {
    3600 // 默认1小时
}

fn default_token_sources() -> Vec<TokenSourceConfig> {
    vec![TokenSourceConfig::AuthorizationBearer]
}

fn default_connect_timeout_ms() -> u64 {
    10_000
}

fn default_request_timeout_ms() -> u64 {
    60_000
}

fn default_observability_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> LogFormat {
    LogFormat::Json
}

fn default_log_file_dir() -> String {
    "logs".to_string()
}

fn default_log_file_prefix() -> String {
    "ai-gw-lite".to_string()
}

fn default_log_rotation() -> LogRotation {
    LogRotation::Daily
}

fn default_log_file_max_files() -> usize {
    7
}

fn default_metrics_path() -> String {
    "/metrics".to_string()
}

fn default_trace_sample_ratio() -> f64 {
    0.05
}

fn default_otlp_timeout_ms() -> u64 {
    3_000
}

fn default_admin_path_prefix() -> String {
    "/admin".to_string()
}

fn default_sqlite_path() -> String {
    "./data/metrics.db".to_string()
}

fn default_sqlite_flush_interval_secs() -> u64 {
    60
}

fn default_sqlite_batch_size() -> usize {
    1000
}

fn default_sqlite_retention_days() -> u32 {
    7
}

fn default_self_signed_cert_path() -> String {
    "certs/gateway-selfsigned.crt".to_string()
}

fn default_self_signed_key_path() -> String {
    "certs/gateway-selfsigned.key".to_string()
}

fn upstream_key_header_names() -> &'static [&'static str] {
    &["authorization", "x-api-key"]
}

fn validate_optional_path(path: Option<&str>, message: &str) -> Result<(), ConfigError> {
    if matches!(path, Some(value) if value.trim().is_empty()) {
        return Err(ConfigError::Validation(message.to_string()));
    }
    Ok(())
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn route_has_upstream_key_injection(route: &RouteConfig) -> bool {
    route.upstream.inject_headers.iter().any(|header| {
        !header.value.trim().is_empty()
            && upstream_key_header_names()
                .iter()
                .any(|name| header.name.trim().eq_ignore_ascii_case(name.trim()))
    })
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, LogFormat, LogRotation, ProxyProtocol};

    #[test]
    fn parse_minimal_config() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert_eq!(config.routes.len(), 1);
        assert!(config.cors.is_none());
        assert!(config.rate_limit.is_none());
        assert!(config.concurrency.is_none());
    }

    #[test]
    fn parse_config_with_env_interpolation() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: '${PATH}'
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert!(!config.api_keys.as_ref().unwrap().keys[0].key.is_empty());
    }

    #[test]
    fn parse_config_with_proxy() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      proxy:
        protocol: "socks"
        address: "127.0.0.1:1080"
        username: "proxy-user"
        password: "proxy-pass"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let proxy = config.routes[0]
            .upstream
            .proxy
            .as_ref()
            .expect("proxy should exist");
        assert_eq!(proxy.protocol, ProxyProtocol::Socks);
        assert_eq!(proxy.address, "127.0.0.1:1080");
        assert_eq!(proxy.username.as_deref(), Some("proxy-user"));
        assert_eq!(proxy.password.as_deref(), Some("proxy-pass"));
    }

    #[test]
    fn reject_proxy_with_partial_auth() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      proxy:
        protocol: "http"
        address: "127.0.0.1:8080"
        username: "proxy-user"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error.to_string().contains(
                "upstream.proxy.username and upstream.proxy.password must be set together"
            )
        );
    }

    #[test]
    fn reject_invalid_user_agent() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      user_agent: "bad\nua"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("upstream.user_agent must be a valid header value")
        );
    }

    #[test]
    fn parse_config_with_inbound_tls() {
        let yaml = r#"
listen: "127.0.0.1:8443"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
inbound_tls:
  cert_path: "./tls/server.crt"
  key_path: "./tls/server.key"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let tls = config
            .inbound_tls
            .as_ref()
            .expect("inbound tls config should exist");
        assert_eq!(tls.cert_path.as_deref(), Some("./tls/server.crt"));
        assert_eq!(tls.key_path.as_deref(), Some("./tls/server.key"));
        assert_eq!(tls.self_signed_cert_path, "certs/gateway-selfsigned.crt");
        assert_eq!(tls.self_signed_key_path, "certs/gateway-selfsigned.key");
    }

    #[test]
    fn reject_inbound_tls_partial_cert_key() {
        let yaml = r#"
listen: "127.0.0.1:8443"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
inbound_tls:
  cert_path: "./tls/server.crt"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error.to_string().contains(
                "`inbound_tls.cert_path` and `inbound_tls.key_path` must be set together"
            )
        );
    }

    #[test]
    fn parse_config_with_rate_limit_and_concurrency() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      upstream_key_max_inflight: 3
      inject_headers:
        - name: "authorization"
          value: "Bearer upstream-key"
rate_limit:
  per_minute: 120
concurrency:
  downstream_max_inflight: 40
  upstream_per_key_max_inflight: 8
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        assert_eq!(
            config.rate_limit.as_ref().expect("rate limit").per_minute,
            120
        );
        let concurrency = config.concurrency.as_ref().expect("concurrency");
        assert_eq!(concurrency.downstream_max_inflight, Some(40));
        assert_eq!(concurrency.upstream_per_key_max_inflight, Some(8));
        assert_eq!(config.routes[0].upstream.upstream_key_max_inflight, Some(3));
    }

    #[test]
    fn parse_config_with_observability() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
observability:
  logging:
    level: "debug"
    format: "text"
    to_stdout: true
    file:
      enabled: true
      dir: "./logs"
      prefix: "gateway"
      rotation: "hourly"
      max_files: 12
  metrics:
    enabled: true
    path: "/metrics"
    token: "metrics_token"
  tracing:
    enabled: true
    sample_ratio: 0.1
    otlp:
      endpoint: "http://127.0.0.1:4317"
      timeout_ms: 5000
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let observability = config
            .observability
            .as_ref()
            .expect("observability should exist");
        assert_eq!(observability.logging.level, "debug");
        assert_eq!(observability.logging.format, LogFormat::Text);
        assert!(observability.logging.to_stdout);
        let log_file = observability
            .logging
            .file
            .as_ref()
            .expect("file should exist");
        assert!(log_file.enabled);
        assert_eq!(log_file.dir, "./logs");
        assert_eq!(log_file.prefix, "gateway");
        assert_eq!(log_file.rotation, LogRotation::Hourly);
        assert_eq!(log_file.max_files, 12);
        assert!(observability.metrics.enabled);
        assert_eq!(observability.metrics.path, "/metrics");
        assert_eq!(observability.metrics.token, "metrics_token");
        assert!(observability.tracing.enabled);
        assert_eq!(observability.tracing.sample_ratio, 0.1);
        let otlp = observability
            .tracing
            .otlp
            .as_ref()
            .expect("otlp should exist");
        assert_eq!(otlp.endpoint, "http://127.0.0.1:4317");
        assert_eq!(otlp.timeout_ms, 5_000);
    }

    #[test]
    fn reject_enabled_metrics_without_token() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
observability:
  metrics:
    enabled: true
    path: "/metrics"
    token: ""
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.metrics.token` must not be empty")
        );
    }

    #[test]
    fn reject_invalid_trace_sample_ratio() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
observability:
  tracing:
    enabled: true
    sample_ratio: 1.2
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.tracing.sample_ratio` must be within [0.0, 1.0]")
        );
    }

    #[test]
    fn reject_logging_without_any_sink() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
observability:
  logging:
    level: "info"
    format: "json"
    to_stdout: false
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("must enable at least one sink (`to_stdout` or `file.enabled`)")
        );
    }

    #[test]
    fn reject_log_file_with_invalid_limits() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
observability:
  logging:
    level: "info"
    format: "json"
    to_stdout: false
    file:
      enabled: true
      dir: "./logs"
      prefix: "gateway"
      rotation: "daily"
      max_files: 0
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`observability.logging.file.max_files` must be > 0")
        );
    }

    #[test]
    fn reject_zero_rate_limit() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
rate_limit:
  per_minute: 0
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("`rate_limit.per_minute` must be > 0")
        );
    }

    #[test]
    fn reject_removed_upstream_key_headers_field() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
concurrency:
  upstream_per_key_max_inflight: 8
  upstream_key_headers:
    - "bad header"
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("unknown field `upstream_key_headers`")
        );
    }

    #[test]
    fn reject_upstream_concurrency_without_injected_key_value() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
concurrency:
  upstream_per_key_max_inflight: 8
"#;

        let error = AppConfig::from_yaml_str(yaml).expect_err("config should fail");
        assert!(
            error
                .to_string()
                .contains("must configure `upstream.inject_headers` with one of")
        );
    }

    #[test]
    fn serialize_config_without_null_fields() {
        let yaml = r#"
listen: "127.0.0.1:8080"
gateway_auth:
  token_sources:
    - type: "authorization_bearer"
api_keys:
  keys:
    - id: "default"
      key: "gw_token"
routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
"#;

        let config = AppConfig::from_yaml_str(yaml).expect("config should parse");
        let serialized = serde_yaml::to_string(&config).expect("should serialize");

        // Should not contain null fields for optional values
        assert!(!serialized.contains("cors: null"), "should not serialize null cors");
        assert!(!serialized.contains("inbound_tls: null"), "should not serialize null inbound_tls");
        assert!(!serialized.contains("rate_limit: null"), "should not serialize null rate_limit");
        assert!(!serialized.contains("concurrency: null"), "should not serialize null concurrency");

        // Should still be parseable after serialization
        let reparsed = AppConfig::from_yaml_str(&serialized).expect("should reparse");
        assert_eq!(reparsed.listen, config.listen);
        assert_eq!(reparsed.routes.len(), config.routes.len());
    }
}

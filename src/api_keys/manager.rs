use crate::api_keys::ban::{BanRuleEngine, BanStatus, BanMetricsSnapshot, BanRule};
use crate::api_keys::ban_log::{BanLogEntry, BanLogStore};
use crate::api_keys::current_epoch_seconds;
use crate::config::ResolvedApiKey;
use crate::ratelimit::{RateLimitDecision, RateLimiter};
use crate::token_quota::{TokenQuotaChecker, CheckQuotaResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

#[derive(Debug)]
pub enum ApiKeyError {
    KeyNotFound,
    KeyDisabled,
    KeyBanned { until: u64, reason: Option<String> },
    RouteNotAllowed,
    RateLimitExceeded { retry_after_secs: u64 },
    ConcurrencyLimitExceeded,
    TokenQuotaExceeded { quota_type: String, limit: u64, used: u64 },
}

impl std::fmt::Display for ApiKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiKeyError::KeyNotFound => write!(f, "API key not found"),
            ApiKeyError::KeyDisabled => write!(f, "API key is disabled"),
            ApiKeyError::KeyBanned { until, reason } => {
                write!(f, "API key is banned until {}", until)?;
                if let Some(r) = reason {
                    write!(f, ": {}", r)?;
                }
                Ok(())
            }
            ApiKeyError::RouteNotAllowed => write!(f, "API key not allowed for this route"),
            ApiKeyError::RateLimitExceeded { retry_after_secs } => {
                write!(f, "Rate limit exceeded, retry after {} seconds", retry_after_secs)
            }
            ApiKeyError::ConcurrencyLimitExceeded => write!(f, "Concurrency limit exceeded"),
            ApiKeyError::TokenQuotaExceeded { quota_type, limit, used } => {
                write!(f, "Token quota exceeded: {} limit {}/{} tokens", quota_type, used, limit)
            }
        }
    }
}

impl std::error::Error for ApiKeyError {}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub key_id: String,
    pub key: ResolvedApiKey,
}

#[derive(Debug, Clone)]
pub struct RequestResult {
    pub success: bool,
    pub latency_ms: u64,
    pub response_status: u16,
}

pub struct ApiKeyManager {
    keys: RwLock<HashMap<String, ApiKeyRuntimeInfo>>,
    id_index: RwLock<HashMap<String, String>>,
    ban_log_store: Option<Arc<dyn BanLogStore>>,
    /// 全局封禁规则（对所有 API Key 生效）
    ban_rules: Vec<BanRule>,
    /// 封禁引擎的最大时间窗口（用于初始化每个 key 的计数器）
    _ban_max_window_secs: u64,
    /// Token配额检查器
    token_quota_checker: Option<Arc<TokenQuotaChecker>>,
}

#[derive(Debug)]
pub struct ApiKeyRuntimeInfo {
    pub resolved: ResolvedApiKey,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub concurrency_semaphore: Option<Arc<Semaphore>>,
    /// 封禁规则引擎（使用全局规则，但每个 key 有自己的计数器）
    pub ban_engine: BanRuleEngine,
}

impl ApiKeyManager {
    pub fn new(
        resolved_keys: Vec<ResolvedApiKey>,
        ban_log_store: Option<Arc<dyn BanLogStore>>,
        global_ban_rules: Vec<BanRule>,
        token_quota_checker: Option<Arc<TokenQuotaChecker>>,
    ) -> Self {
        let mut keys = HashMap::new();
        let mut id_index = HashMap::new();

        // 计算全局封禁规则的最大时间窗口
        let ban_max_window_secs = if !global_ban_rules.is_empty() {
            global_ban_rules
                .iter()
                .filter_map(|r| match &r.condition {
                    crate::api_keys::ban::BanCondition::ErrorRate { window_secs, .. } => {
                        Some(*window_secs)
                    }
                    crate::api_keys::ban::BanCondition::RequestCount { window_secs, .. } => {
                        Some(*window_secs)
                    }
                    _ => Some(3600),
                })
                .max()
                .unwrap_or(3600)
        } else {
            3600
        };

        for resolved in resolved_keys {
            let key_value = resolved.key.clone();
            let key_id = resolved.id.clone();

            let rate_limiter = resolved.rate_limit.as_ref().map(|cfg| {
                Arc::new(RateLimiter::new(cfg.per_minute))
            });

            let concurrency_semaphore = resolved.concurrency.as_ref()
                .and_then(|cfg| cfg.downstream_max_inflight)
                .map(|limit| Arc::new(Semaphore::new(limit)));

            // 为每个 key 创建封禁引擎（使用全局最大窗口）
            let ban_engine = BanRuleEngine::new(ban_max_window_secs);

            let runtime_info = ApiKeyRuntimeInfo {
                resolved,
                rate_limiter,
                concurrency_semaphore,
                ban_engine,
            };

            id_index.insert(key_id, key_value.clone());
            keys.insert(key_value, runtime_info);
        }

        Self {
            keys: RwLock::new(keys),
            id_index: RwLock::new(id_index),
            ban_log_store,
            ban_rules: global_ban_rules,
            _ban_max_window_secs: ban_max_window_secs,
            token_quota_checker,
        }
    }

    pub async fn validate_key(&self, key_value: &str, route_id: &str) -> Result<ValidationResult, ApiKeyError> {
        let keys = self.keys.read().await;
        let info = keys.get(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if !info.resolved.enabled {
            return Err(ApiKeyError::KeyDisabled);
        }

        if let Some(status) = &info.resolved.ban_status {
            if status.is_banned {
                if let Some(until) = status.banned_until {
                    let now = current_epoch_seconds();
                    if now < until {
                        return Err(ApiKeyError::KeyBanned {
                            until,
                            reason: status.reason.clone(),
                        });
                    }
                }
            }
        }

        // 检查路由权限（优先使用 route_ids，兼容 route_id）
        if let Some(allowed_routes) = &info.resolved.route_ids {
            // 使用新的 route_ids 字段（多路由）
            if !allowed_routes.is_empty() && !allowed_routes.contains(&route_id.to_string()) {
                return Err(ApiKeyError::RouteNotAllowed);
            }
        } else if let Some(allowed_route) = &info.resolved.route_id {
            // 兼容旧的 route_id 字段（单路由）
            if allowed_route != route_id {
                return Err(ApiKeyError::RouteNotAllowed);
            }
        }

        Ok(ValidationResult {
            key_id: info.resolved.id.clone(),
            key: info.resolved.clone(),
        })
    }

    pub async fn check_rate_limit(&self, key_value: &str, _route_id: &str) -> Result<(), ApiKeyError> {
        let keys = self.keys.read().await;
        let info = keys.get(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if let Some(limiter) = &info.rate_limiter {
            // API Key 级别的限流是针对该 Key 的全局限制，不区分路由
            // 使用固定的 key 来统计该 API Key 的所有请求
            match limiter.check(&info.resolved.id, "global") {
                RateLimitDecision::Allowed => Ok(()),
                RateLimitDecision::Rejected { retry_after_secs } => {
                    Err(ApiKeyError::RateLimitExceeded { retry_after_secs })
                }
            }
        } else {
            Ok(())
        }
    }

    pub async fn acquire_concurrency_permit(&self, key_value: &str) -> Result<Option<OwnedSemaphorePermit>, ApiKeyError> {
        let keys = self.keys.read().await;
        let info = keys.get(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if let Some(semaphore) = &info.concurrency_semaphore {
            match semaphore.clone().try_acquire_owned() {
                Ok(permit) => Ok(Some(permit)),
                Err(_) => Err(ApiKeyError::ConcurrencyLimitExceeded),
            }
        } else {
            Ok(None)
        }
    }

    /// 检查Token配额
    pub fn check_token_quota(&self, key_value: &str) -> Result<CheckQuotaResult, ApiKeyError> {
        if let Some(checker) = &self.token_quota_checker {
            // 获取API Key ID
            // 注意：这里我们使用key_value作为临时ID，实际应该使用key_id
            // 但由于check_token_quota是同步方法，无法async获取key_id
            // 我们将在validate_key之后调用此方法，传入key_id
            let key_id = self.get_key_id_sync(key_value);
            if let Some(id) = key_id {
                let result = checker.check_quota(&id);
                if !result.allowed {
                    // 构造错误信息
                    if let Some(limit) = result.daily_limit_total {
                        if result.daily_used_total >= limit {
                            return Err(ApiKeyError::TokenQuotaExceeded {
                                quota_type: "daily_total".to_string(),
                                limit,
                                used: result.daily_used_total,
                            });
                        }
                    }
                    if let Some(limit) = result.daily_limit_input {
                        if result.daily_used_input >= limit {
                            return Err(ApiKeyError::TokenQuotaExceeded {
                                quota_type: "daily_input".to_string(),
                                limit,
                                used: result.daily_used_input,
                            });
                        }
                    }
                    if let Some(limit) = result.daily_limit_output {
                        if result.daily_used_output >= limit {
                            return Err(ApiKeyError::TokenQuotaExceeded {
                                quota_type: "daily_output".to_string(),
                                limit,
                                used: result.daily_used_output,
                            });
                        }
                    }
                    if let Some(limit) = result.weekly_limit_total {
                        if result.weekly_used_total >= limit {
                            return Err(ApiKeyError::TokenQuotaExceeded {
                                quota_type: "weekly_total".to_string(),
                                limit,
                                used: result.weekly_used_total,
                            });
                        }
                    }
                }
                return Ok(result);
            }
        }
        // 无配额限制时返回默认允许的结果
        Ok(CheckQuotaResult {
            allowed: true,
            daily_used_input: 0,
            daily_used_output: 0,
            daily_used_total: 0,
            daily_limit_input: None,
            daily_limit_output: None,
            daily_limit_total: None,
            daily_remaining_input: None,
            daily_remaining_output: None,
            daily_remaining_total: None,
            weekly_used_input: 0,
            weekly_used_output: 0,
            weekly_used_total: 0,
            weekly_limit_input: None,
            weekly_limit_output: None,
            weekly_limit_total: None,
            weekly_remaining_input: None,
            weekly_remaining_output: None,
            weekly_remaining_total: None,
            reason: None,
        })
    }

    /// 检查Token配额（使用key_id版本）
    pub fn check_token_quota_by_id(&self, key_id: &str) -> Result<CheckQuotaResult, ApiKeyError> {
        if let Some(checker) = &self.token_quota_checker {
            let result = checker.check_quota(key_id);
            if !result.allowed {
                if let Some(limit) = result.daily_limit_total {
                    if result.daily_used_total >= limit {
                        return Err(ApiKeyError::TokenQuotaExceeded {
                            quota_type: "daily_total".to_string(),
                            limit,
                            used: result.daily_used_total,
                        });
                    }
                }
                if let Some(limit) = result.daily_limit_input {
                    if result.daily_used_input >= limit {
                        return Err(ApiKeyError::TokenQuotaExceeded {
                            quota_type: "daily_input".to_string(),
                            limit,
                            used: result.daily_used_input,
                        });
                    }
                }
                if let Some(limit) = result.daily_limit_output {
                    if result.daily_used_output >= limit {
                        return Err(ApiKeyError::TokenQuotaExceeded {
                            quota_type: "daily_output".to_string(),
                            limit,
                            used: result.daily_used_output,
                        });
                    }
                }
                if let Some(limit) = result.weekly_limit_total {
                    if result.weekly_used_total >= limit {
                        return Err(ApiKeyError::TokenQuotaExceeded {
                            quota_type: "weekly_total".to_string(),
                            limit,
                            used: result.weekly_used_total,
                        });
                    }
                }
            }
            return Ok(result);
        }
        Ok(CheckQuotaResult {
            allowed: true,
            daily_used_input: 0,
            daily_used_output: 0,
            daily_used_total: 0,
            daily_limit_input: None,
            daily_limit_output: None,
            daily_limit_total: None,
            daily_remaining_input: None,
            daily_remaining_output: None,
            daily_remaining_total: None,
            weekly_used_input: 0,
            weekly_used_output: 0,
            weekly_used_total: 0,
            weekly_limit_input: None,
            weekly_limit_output: None,
            weekly_limit_total: None,
            weekly_remaining_input: None,
            weekly_remaining_output: None,
            weekly_remaining_total: None,
            reason: None,
        })
    }

    /// 同步获取key_id（用于非async上下文）
    fn get_key_id_sync(&self, key_value: &str) -> Option<String> {
        // 由于无法使用async/await，我们尝试通过迭代id_index来查找
        // 这是一个折衷方案，实际使用中应该优先使用key_id
        Some(crate::api_keys::generate_key_id(key_value))
    }

    /// 获取token配额检查器
    pub fn token_quota_checker(&self) -> Option<&Arc<TokenQuotaChecker>> {
        self.token_quota_checker.as_ref()
    }

    pub async fn report_request_result(
        &self,
        key_value: &str,
        result: RequestResult,
    ) -> Option<BanStatus> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value)?;

        // 使用全局封禁规则检查
        if !self.ban_rules.is_empty() {
            let now = current_epoch_seconds();

            if let Some(triggered) = info
                .ban_engine
                .check_rules(&self.ban_rules, now, result.success)
            {
                let ban_until = now + triggered.ban_duration_secs;
                let new_status = BanStatus {
                    is_banned: true,
                    banned_at: Some(now),
                    banned_until: Some(ban_until),
                    triggered_rule_id: Some(triggered.rule_id.clone()),
                    reason: Some(triggered.reason.clone()),
                    ban_count: info
                        .resolved
                        .ban_status
                        .as_ref()
                        .map(|s| s.ban_count + 1)
                        .unwrap_or(1),
                };

                info.resolved.ban_status = Some(new_status.clone());

                if let Some(store) = &self.ban_log_store {
                    let entry = BanLogEntry {
                        id: format!("ban_{}_{}", key_value, now),
                        api_key_id: info.resolved.id.clone(),
                        rule_id: triggered.rule_id.clone(),
                        reason: triggered.reason.clone(),
                        banned_at: now,
                        banned_until: ban_until,
                        unbanned_at: None,
                        metrics_snapshot: triggered.metrics_snapshot.clone(),
                    };

                    tracing::info!(
                        "Inserting ban log for key {} (rule: {}, until: {})",
                        info.resolved.id,
                        triggered.rule_id,
                        ban_until
                    );

                    let store = Arc::clone(store);
                    tokio::spawn(async move {
                        match store.insert(entry).await {
                            Ok(()) => tracing::info!("Ban log inserted successfully"),
                            Err(e) => tracing::error!("Failed to insert ban log: {}", e),
                        }
                    });
                } else {
                    tracing::warn!("Ban log store not available, skipping ban log insertion");
                }

                return Some(new_status);
            }
        }

        None
    }

    pub async fn ban_key(&self, key_value: &str, duration_secs: u64, reason: String) -> Result<BanStatus, ApiKeyError> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        let now = current_epoch_seconds();
        let ban_until = now + duration_secs;

        let new_status = BanStatus {
            is_banned: true,
            banned_at: Some(now),
            banned_until: Some(ban_until),
            triggered_rule_id: None,
            reason: Some(reason.clone()),
            ban_count: info.resolved.ban_status.as_ref()
                .map(|s| s.ban_count + 1)
                .unwrap_or(1),
        };

        info.resolved.ban_status = Some(new_status.clone());

        if let Some(store) = &self.ban_log_store {
            let entry = BanLogEntry {
                id: format!("ban_manual_{}_{}", info.resolved.id, now),
                api_key_id: info.resolved.id.clone(),
                rule_id: "manual".to_string(),
                reason: reason.clone(),
                banned_at: now,
                banned_until: ban_until,
                unbanned_at: None,
                metrics_snapshot: BanMetricsSnapshot {
                    requests: 0,
                    errors: 0,
                    error_rate: 0.0,
                },
            };

            tracing::info!(
                "Inserting manual ban log for key {} (until: {})",
                info.resolved.id,
                ban_until
            );

            let store = Arc::clone(store);
            tokio::spawn(async move {
                match store.insert(entry).await {
                    Ok(()) => tracing::info!("Manual ban log inserted successfully"),
                    Err(e) => tracing::error!("Failed to insert manual ban log: {}", e),
                }
            });
        } else {
            tracing::warn!("Ban log store not available, skipping manual ban log insertion");
        }

        Ok(new_status)
    }

    pub async fn unban_key(&self, key_value: &str) -> Result<(), ApiKeyError> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if let Some(status) = &mut info.resolved.ban_status {
            let now = current_epoch_seconds();
            let was_banned = status.is_banned;
            status.is_banned = false;

            // 如果之前是封禁状态，尝试更新封禁日志的解封时间
            if was_banned {
                if let Some(store) = &self.ban_log_store {
                    if let Some(banned_at) = status.banned_at {
                        // 尝试两种可能的 entry_id 格式：
                        // 1. 自动封禁: ban_{key_value}_{banned_at}
                        // 2. 手动封禁: ban_manual_{key_id}_{banned_at}
                        let entry_ids = if let Some(_rule_id) = &status.triggered_rule_id {
                            // 自动封禁：只有一个可能的 ID
                            vec![format!("ban_{}_{}", key_value, banned_at)]
                        } else {
                            // 手动封禁：只有一个可能的 ID
                            vec![format!("ban_manual_{}_{}", info.resolved.id, banned_at)]
                        };

                        let store = Arc::clone(store);
                        let key_value_owned = key_value.to_string();
                        tokio::spawn(async move {
                            for entry_id in entry_ids {
                                if let Ok(()) = store.mark_unbanned(&entry_id, now).await {
                                    tracing::debug!("Marked ban log as unbanned: {}", entry_id);
                                    return;
                                }
                            }
                            tracing::warn!("Failed to find ban log entry to mark as unbanned for key: {}", key_value_owned);
                        });
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn get_key_by_id(&self, id: &str) -> Option<String> {
        let id_index = self.id_index.read().await;
        id_index.get(id).cloned()
    }

    pub async fn get_all_keys(&self) -> Vec<ResolvedApiKey> {
        let keys = self.keys.read().await;
        keys.values()
            .map(|info| info.resolved.clone())
            .collect()
    }

    pub async fn get_key_info(&self, key_value: &str) -> Option<ResolvedApiKey> {
        let keys = self.keys.read().await;
        keys.get(key_value).map(|info| info.resolved.clone())
    }

    pub fn ban_log_store(&self) -> Option<Arc<dyn BanLogStore>> {
        self.ban_log_store.clone()
    }

    /// 恢复封禁状态（用于配置热更新时迁移状态）
    pub async fn restore_ban_status(
        &self,
        key_value: &str,
        ban_status: BanStatus,
    ) -> Result<(), ApiKeyError> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value).ok_or(ApiKeyError::KeyNotFound)?;
        info.resolved.ban_status = Some(ban_status);
        Ok(())
    }
}

pub async fn create_api_key_manager(
    config: &crate::config::AppConfig,
    old_manager: Option<&ApiKeyManager>,
    token_quota_checker: Option<Arc<TokenQuotaChecker>>,
) -> Option<ApiKeyManager> {
    let resolved_keys = config.resolved_api_keys();
    if resolved_keys.is_empty() {
        None
    } else {
        // 提取全局封禁规则
        let global_ban_rules = config
            .api_keys
            .as_ref()
            .map(|ak| ak.ban_rules.clone())
            .unwrap_or_default();

        // 创建封禁日志存储（使用配置路径或默认路径）
        let db_path = config
            .api_keys
            .as_ref()
            .and_then(|ak| ak.sqlite.as_ref())
            .map(|s| s.path.as_str())
            .unwrap_or("./data/ban_logs.db");
        let ban_log_store = match create_ban_log_store(db_path).await {
            Some(store) => Some(store),
            None => {
                tracing::warn!("Failed to create ban log store, ban logging will be disabled");
                None
            }
        };

        let new_manager = ApiKeyManager::new(resolved_keys, ban_log_store, global_ban_rules, token_quota_checker);

        // 如果有旧的 manager，迁移封禁状态
        if let Some(old) = old_manager {
            migrate_ban_status(&new_manager, old).await;
        }

        Some(new_manager)
    }
}

/// 从旧的 ApiKeyManager 迁移封禁状态到新的 Manager
async fn migrate_ban_status(new_manager: &ApiKeyManager, old_manager: &ApiKeyManager) {
    // 获取旧 manager 中的所有 key 及其封禁状态
    let old_keys = old_manager.get_all_keys().await;

    for old_key in old_keys {
        if let Some(ban_status) = &old_key.ban_status {
            // 只迁移处于封禁状态且未过期的
            if ban_status.is_banned {
                if let Some(until) = ban_status.banned_until {
                    let now = crate::api_keys::current_epoch_seconds();
                    if now < until {
                        // 封禁仍然有效，迁移状态
                        if let Err(e) = new_manager
                            .restore_ban_status(&old_key.key, ban_status.clone())
                            .await
                        {
                            tracing::warn!(
                                "Failed to migrate ban status for key {}: {}",
                                old_key.id,
                                e
                            );
                        } else {
                            tracing::info!(
                                "Migrated ban status for key {} (banned until {})",
                                old_key.id,
                                until
                            );
                        }
                    }
                }
            }
        }
    }
}

/// 创建封禁日志存储（SQLite）
async fn create_ban_log_store(db_path: &str) -> Option<Arc<dyn BanLogStore>> {
    use crate::api_keys::ban_log::SqliteBanLogStore;

    tracing::info!("Creating ban log store at: {}", db_path);
    match SqliteBanLogStore::new(db_path).await {
        Ok(store) => {
            tracing::info!("Ban log store created successfully");
            Some(Arc::new(store))
        }
        Err(e) => {
            tracing::error!("Failed to create SQLite ban log store: {}", e);
            None
        }
    }
}

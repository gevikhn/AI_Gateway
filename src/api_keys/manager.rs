use crate::api_keys::ban::{BanRuleEngine, BanStatus, BanMetricsSnapshot};
use crate::api_keys::ban_log::{BanLogEntry, BanLogStore};
use crate::api_keys::current_epoch_seconds;
use crate::config::ResolvedApiKey;
use crate::ratelimit::{RateLimitDecision, RateLimiter};
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
}

#[derive(Debug)]
pub struct ApiKeyRuntimeInfo {
    pub resolved: ResolvedApiKey,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub concurrency_semaphore: Option<Arc<Semaphore>>,
    pub ban_engine: Option<BanRuleEngine>,
}

impl ApiKeyManager {
    pub fn new(resolved_keys: Vec<ResolvedApiKey>, ban_log_store: Option<Arc<dyn BanLogStore>>) -> Self {
        let mut keys = HashMap::new();
        let mut id_index = HashMap::new();

        for resolved in resolved_keys {
            let key_value = resolved.key.clone();
            let key_id = resolved.id.clone();

            let rate_limiter = resolved.rate_limit.as_ref().map(|cfg| {
                Arc::new(RateLimiter::new(cfg.per_minute))
            });

            let concurrency_semaphore = resolved.concurrency.as_ref()
                .and_then(|cfg| cfg.downstream_max_inflight)
                .map(|limit| Arc::new(Semaphore::new(limit)));

            let ban_engine = if !resolved.ban_rules.is_empty() {
                let max_window = resolved.ban_rules.iter()
                    .filter_map(|r| match &r.condition {
                        crate::api_keys::ban::BanCondition::ErrorRate { window_secs, .. } => Some(window_secs),
                        crate::api_keys::ban::BanCondition::RequestCount { window_secs, .. } => Some(window_secs),
                        _ => Some(&3600),
                    })
                    .copied()
                    .max()
                    .unwrap_or(3600);
                Some(BanRuleEngine::new(max_window))
            } else {
                None
            };

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

        if let Some(allowed_route) = &info.resolved.route_id {
            if allowed_route != route_id {
                return Err(ApiKeyError::RouteNotAllowed);
            }
        }

        Ok(ValidationResult {
            key_id: info.resolved.id.clone(),
            key: info.resolved.clone(),
        })
    }

    pub async fn check_rate_limit(&self, key_value: &str, route_id: &str) -> Result<(), ApiKeyError> {
        let keys = self.keys.read().await;
        let info = keys.get(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if let Some(limiter) = &info.rate_limiter {
            match limiter.check(&info.resolved.id, route_id) {
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

    pub async fn report_request_result(&self, key_value: &str, result: RequestResult) -> Option<BanStatus> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value)?;

        if let Some(engine) = info.ban_engine.as_mut() {
            let now = current_epoch_seconds();

            if let Some(triggered) = engine.check_rules(&info.resolved.ban_rules, now, result.success) {
                let ban_until = now + triggered.ban_duration_secs;
                let new_status = BanStatus {
                    is_banned: true,
                    banned_at: Some(now),
                    banned_until: Some(ban_until),
                    triggered_rule_id: Some(triggered.rule_id.clone()),
                    reason: Some(triggered.reason.clone()),
                    ban_count: info.resolved.ban_status.as_ref()
                        .map(|s| s.ban_count + 1)
                        .unwrap_or(1),
                };

                info.resolved.ban_status = Some(new_status.clone());

                if let Some(store) = &self.ban_log_store {
                    let entry = BanLogEntry {
                        id: format!("ban_{}_{}", key_value, now),
                        api_key_id: info.resolved.id.clone(),
                        rule_id: triggered.rule_id,
                        reason: triggered.reason,
                        banned_at: now,
                        banned_until: ban_until,
                        unbanned_at: None,
                        metrics_snapshot: triggered.metrics_snapshot,
                    };

                    let store = Arc::clone(store);
                    tokio::spawn(async move {
                        if let Err(e) = store.insert(entry).await {
                            tracing::error!("Failed to insert ban log: {}", e);
                        }
                    });
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

            let store = Arc::clone(store);
            tokio::spawn(async move {
                if let Err(e) = store.insert(entry).await {
                    tracing::error!("Failed to insert ban log: {}", e);
                }
            });
        }

        Ok(new_status)
    }

    pub async fn unban_key(&self, key_value: &str) -> Result<(), ApiKeyError> {
        let mut keys = self.keys.write().await;
        let info = keys.get_mut(key_value).ok_or(ApiKeyError::KeyNotFound)?;

        if let Some(status) = &mut info.resolved.ban_status {
            let now = current_epoch_seconds();
            status.is_banned = false;

            if let Some(store) = &self.ban_log_store {
                if let Some(_rule_id) = &status.triggered_rule_id {
                    let entry_id = format!("ban_{}_{}", key_value, status.banned_at.unwrap_or(0));
                    let store = Arc::clone(store);
                    tokio::spawn(async move {
                        if let Err(e) = store.mark_unbanned(&entry_id, now).await {
                            tracing::error!("Failed to mark ban as unbanned: {}", e);
                        }
                    });
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
}

pub fn create_api_key_manager(
    config: &crate::config::AppConfig,
    ban_log_store: Option<Arc<dyn BanLogStore>>,
) -> Option<ApiKeyManager> {
    let resolved_keys = config.resolved_api_keys();
    if resolved_keys.is_empty() {
        None
    } else {
        Some(ApiKeyManager::new(resolved_keys, ban_log_store))
    }
}

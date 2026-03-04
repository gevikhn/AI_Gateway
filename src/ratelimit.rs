use crate::config::RateLimitConfig;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum RateLimitDecision {
    Allowed,
    Rejected { retry_after_secs: u64 },
}

/// 限流器管理器，支持多级别限流配置
pub struct RateLimiterManager {
    /// 全局默认限流配置
    global_config: Option<RateLimitConfig>,
    /// API Key 级别的限流器，使用 DashMap 实现细粒度锁
    key_limiters: DashMap<String, RateLimiter>,
}

impl RateLimiterManager {
    pub fn new(global_config: Option<RateLimitConfig>) -> Self {
        Self {
            global_config,
            key_limiters: DashMap::new(),
        }
    }

    /// 检查请求是否通过限流
    ///
    /// # Arguments
    /// * `api_key` - API Key 值
    /// * `api_key_config` - API Key 级别的限流配置（可选）
    /// * `route_config` - 路由级别的限流配置（可选）
    pub fn check(
        &self,
        api_key: &str,
        api_key_config: Option<&RateLimitConfig>,
        route_config: Option<&RateLimitConfig>,
    ) -> RateLimitDecision {
        // 配置继承：api_key级 > 路由级 > 全局级
        let effective_config = api_key_config
            .or(route_config)
            .or(self.global_config.as_ref());

        let Some(config) = effective_config else {
            // 没有配置限流，直接通过
            return RateLimitDecision::Allowed;
        };

        let entry = self.key_limiters.entry(api_key.to_string());
        let mut limiter_ref = entry.or_insert_with(|| RateLimiter::new(config.per_minute));

        // 如果配置变更，更新限流器的限制
        if limiter_ref.per_minute != config.per_minute {
            limiter_ref.update_limit(config.per_minute);
        }

        limiter_ref.check_internal(current_epoch_seconds())
    }

    /// 获取或创建限流器（用于向后兼容）
    pub fn get_or_create_limiter(
        &self,
        api_key: &str,
        per_minute: u64,
    ) -> RateLimiter {
        self.key_limiters
            .entry(api_key.to_string())
            .or_insert_with(|| RateLimiter::new(per_minute))
            .clone_for_key(api_key)
    }
}

/// 单个限流器实例
#[derive(Debug, Clone)]
pub struct RateLimiter {
    per_minute: u64,
    state: std::sync::Arc<Mutex<RateLimiterState>>,
}

#[derive(Debug)]
struct RateLimiterState {
    minute_bucket: u64,
    counters: HashMap<String, u64>,
}

impl RateLimiter {
    pub fn new(per_minute: u64) -> Self {
        Self {
            per_minute,
            state: std::sync::Arc::new(Mutex::new(RateLimiterState {
                minute_bucket: current_epoch_seconds() / 60,
                counters: HashMap::new(),
            })),
        }
    }

    /// 为特定 API Key 创建限流器实例（向后兼容）
    fn clone_for_key(&self, _api_key: &str) -> Self {
        Self {
            per_minute: self.per_minute,
            state: self.state.clone(),
        }
    }

    /// 更新限流限制
    fn update_limit(&mut self, per_minute: u64) {
        self.per_minute = per_minute;
    }

    /// 检查请求（向后兼容方法）
    pub fn check(&self, token: &str, route_id: &str) -> RateLimitDecision {
        let key = format!("{route_id}\n{token}");
        self.check_with_key(&key)
    }

    /// 使用指定 key 检查限流
    fn check_with_key(&self, key: &str) -> RateLimitDecision {
        self.check_at_epoch_seconds(key, current_epoch_seconds())
    }

    /// 内部检查方法（使用单一 key）
    fn check_internal(&self, epoch_seconds: u64) -> RateLimitDecision {
        let key = "default";
        self.check_at_epoch_seconds(key, epoch_seconds)
    }

    fn check_at_epoch_seconds(&self, key: &str, epoch_seconds: u64) -> RateLimitDecision {
        let minute_bucket = epoch_seconds / 60;
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if state.minute_bucket != minute_bucket {
            state.minute_bucket = minute_bucket;
            state.counters.clear();
        }

        let counter = state.counters.entry(key.to_string()).or_insert(0);

        if *counter >= self.per_minute {
            return RateLimitDecision::Rejected {
                retry_after_secs: retry_after_seconds(epoch_seconds),
            };
        }

        *counter += 1;
        RateLimitDecision::Allowed
    }
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn retry_after_seconds(epoch_seconds: u64) -> u64 {
    let remaining = 60 - (epoch_seconds % 60);
    if remaining == 0 { 60 } else { remaining }
}

#[cfg(test)]
mod tests {
    use super::{RateLimitDecision, RateLimiter, RateLimiterManager};
    use crate::config::RateLimitConfig;

    #[test]
    fn allows_until_limit_then_rejects() {
        let limiter = RateLimiter::new(2);
        let now = 1_700_000_040;

        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", now),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", now),
            RateLimitDecision::Allowed
        ));

        match limiter.check_at_epoch_seconds("gw_token", now) {
            RateLimitDecision::Rejected { retry_after_secs } => {
                assert!((1..=60).contains(&retry_after_secs));
            }
            RateLimitDecision::Allowed => panic!("third request should be rejected"),
        }
    }

    #[test]
    fn separates_counters_by_key() {
        let limiter = RateLimiter::new(1);
        let now = 1_700_000_040;

        assert!(matches!(
            limiter.check_at_epoch_seconds("key_a", now),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("key_b", now),
            RateLimitDecision::Allowed
        ));
    }

    #[test]
    fn rotates_window_every_minute() {
        let limiter = RateLimiter::new(1);
        let t1 = 1_700_000_040;
        let t2 = t1 + 61;

        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", t1),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", t2),
            RateLimitDecision::Allowed
        ));
    }

    #[test]
    fn manager_uses_api_key_config_first() {
        let global_config = RateLimitConfig { per_minute: 10 };
        let manager = RateLimiterManager::new(Some(global_config));

        // API Key 级别配置：每分钟 2 次
        let api_key_config = RateLimitConfig { per_minute: 2 };

        // 应该使用 API Key 级别的配置（2次限制）
        assert!(matches!(
            manager.check("test_key", Some(&api_key_config), None),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            manager.check("test_key", Some(&api_key_config), None),
            RateLimitDecision::Allowed
        ));
        // 第三次应该被拒绝（因为 API Key 级别限制是 2）
        assert!(matches!(
            manager.check("test_key", Some(&api_key_config), None),
            RateLimitDecision::Rejected { .. }
        ));
    }

    #[test]
    fn manager_falls_back_to_route_config() {
        let global_config = RateLimitConfig { per_minute: 10 };
        let manager = RateLimiterManager::new(Some(global_config));

        // 路由级别配置：每分钟 2 次
        let route_config = RateLimitConfig { per_minute: 2 };

        // 没有 API Key 配置，应该使用路由级别配置
        assert!(matches!(
            manager.check("test_key", None, Some(&route_config)),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            manager.check("test_key", None, Some(&route_config)),
            RateLimitDecision::Allowed
        ));
        // 第三次应该被拒绝
        assert!(matches!(
            manager.check("test_key", None, Some(&route_config)),
            RateLimitDecision::Rejected { .. }
        ));
    }

    #[test]
    fn manager_falls_back_to_global_config() {
        // 全局配置：每分钟 2 次
        let global_config = RateLimitConfig { per_minute: 2 };
        let manager = RateLimiterManager::new(Some(global_config));

        // 没有 API Key 和路由配置，应该使用全局配置
        assert!(matches!(
            manager.check("test_key", None, None),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            manager.check("test_key", None, None),
            RateLimitDecision::Allowed
        ));
        // 第三次应该被拒绝
        assert!(matches!(
            manager.check("test_key", None, None),
            RateLimitDecision::Rejected { .. }
        ));
    }

    #[test]
    fn manager_allows_when_no_config() {
        let manager = RateLimiterManager::new(None);

        // 没有任何配置，应该直接通过
        for _ in 0..100 {
            assert!(matches!(
                manager.check("test_key", None, None),
                RateLimitDecision::Allowed
            ));
        }
    }

    #[test]
    fn backward_compatible_check() {
        let limiter = RateLimiter::new(2);

        // 测试旧的 check 方法
        assert!(matches!(
            limiter.check("gw_token", "openai"),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check("gw_token", "openai"),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check("gw_token", "openai"),
            RateLimitDecision::Rejected { .. }
        ));
    }

    #[test]
    fn api_key_rate_limit_is_global_across_routes() {
        // 测试 API Key 级别的限流应该跨路由共享计数器
        // 这是 manager.rs 中使用 "global" 作为固定 route_id 的行为
        let limiter = RateLimiter::new(2);

        // 使用 "global" 作为 key（模拟 manager.rs 的修复后行为）
        assert!(matches!(
            limiter.check("api_key_1", "global"),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check("api_key_1", "global"),
            RateLimitDecision::Allowed
        ));
        // 第三次应该被拒绝
        assert!(matches!(
            limiter.check("api_key_1", "global"),
            RateLimitDecision::Rejected { .. }
        ));
    }
}

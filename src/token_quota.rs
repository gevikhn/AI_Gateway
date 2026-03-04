use crate::config::TokenQuotaConfig;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Token配额管理器
pub struct TokenQuotaManager {
    /// api_key_id -> 配额配置
    quotas: DashMap<String, TokenQuotaConfig>,
    /// api_key_id -> 实时使用统计
    usage_stats: DashMap<String, TokenUsageWindow>,
}

/// Token使用时间窗口统计（内存中）
pub struct TokenUsageWindow {
    /// 小时级统计（最近24小时）
    hourly_stats: VecDeque<HourlyTokenStat>,
    /// 天级统计（最近7天）
    daily_stats: VecDeque<DailyTokenStat>,
}

/// 小时级Token统计
#[derive(Debug, Clone, Copy)]
pub struct HourlyTokenStat {
    pub hour_epoch: u64, // Unix时间戳，小时级（整点）
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 天级Token统计
#[derive(Debug, Clone, Copy)]
pub struct DailyTokenStat {
    pub day_epoch: u64, // Unix时间戳，日期级（00:00:00）
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 配额检查结果
#[derive(Debug, Clone)]
pub struct CheckQuotaResult {
    pub allowed: bool,
    pub daily_used_input: u64,
    pub daily_used_output: u64,
    pub daily_used_total: u64,
    pub daily_limit_input: Option<u64>,
    pub daily_limit_output: Option<u64>,
    pub daily_limit_total: Option<u64>,
    pub daily_remaining_input: Option<u64>,
    pub daily_remaining_output: Option<u64>,
    pub daily_remaining_total: Option<u64>,
    pub weekly_used_input: u64,
    pub weekly_used_output: u64,
    pub weekly_used_total: u64,
    pub weekly_limit_input: Option<u64>,
    pub weekly_limit_output: Option<u64>,
    pub weekly_limit_total: Option<u64>,
    pub weekly_remaining_input: Option<u64>,
    pub weekly_remaining_output: Option<u64>,
    pub weekly_remaining_total: Option<u64>,
    pub reason: Option<String>,
}

impl TokenQuotaManager {
    pub fn new() -> Self {
        Self {
            quotas: DashMap::new(),
            usage_stats: DashMap::new(),
        }
    }

    /// 从解析后的API Key配置初始化配额
    pub fn init_from_resolved_keys(&self, keys: &[crate::config::ResolvedApiKey]) {
        for key in keys {
            if let Some(quota) = &key.token_quota {
                self.quotas.insert(key.id.clone(), quota.clone());
            }
        }
    }

    /// 获取API Key的配额配置
    pub fn get_quota(&self, api_key_id: &str) -> Option<TokenQuotaConfig> {
        self.quotas.get(api_key_id).map(|q| q.clone())
    }

    /// 获取或创建使用统计窗口
    fn get_or_create_usage_window(&self, api_key_id: &str) -> dashmap::mapref::one::RefMut<'_, String, TokenUsageWindow> {
        self.usage_stats.entry(api_key_id.to_string()).or_insert_with(|| TokenUsageWindow {
            hourly_stats: VecDeque::with_capacity(24),
            daily_stats: VecDeque::with_capacity(7),
        })
    }

    /// 记录token使用（在请求完成后调用）
    pub fn record_usage(&self, api_key_id: &str, input_tokens: u64, output_tokens: u64) {
        let now = current_epoch_seconds();
        let hour_epoch = truncate_to_hour(now);
        let day_epoch = truncate_to_day(now);

        let mut window = self.get_or_create_usage_window(api_key_id);

        // 更新小时级统计
        if let Some(hourly) = window.hourly_stats.back_mut() {
            if hourly.hour_epoch == hour_epoch {
                hourly.input_tokens += input_tokens;
                hourly.output_tokens += output_tokens;
            } else {
                // 新的小时，添加新记录
                window.hourly_stats.push_back(HourlyTokenStat {
                    hour_epoch,
                    input_tokens,
                    output_tokens,
                });
                // 清理过期数据（保留24小时）
                while window.hourly_stats.len() > 24 {
                    window.hourly_stats.pop_front();
                }
            }
        } else {
            window.hourly_stats.push_back(HourlyTokenStat {
                hour_epoch,
                input_tokens,
                output_tokens,
            });
        }

        // 更新天级统计
        if let Some(daily) = window.daily_stats.back_mut() {
            if daily.day_epoch == day_epoch {
                daily.input_tokens += input_tokens;
                daily.output_tokens += output_tokens;
            } else {
                // 新的一天，添加新记录
                window.daily_stats.push_back(DailyTokenStat {
                    day_epoch,
                    input_tokens,
                    output_tokens,
                });
                // 清理过期数据（保留7天）
                while window.daily_stats.len() > 7 {
                    window.daily_stats.pop_front();
                }
            }
        } else {
            window.daily_stats.push_back(DailyTokenStat {
                day_epoch,
                input_tokens,
                output_tokens,
            });
        }
    }

    /// 检查配额状态
    pub fn check_quota(&self, api_key_id: &str) -> CheckQuotaResult {
        let quota = self.get_quota(api_key_id);

        // 计算当前使用量
        let (daily_input, daily_output, daily_total) = self.calculate_daily_usage(api_key_id);
        let (weekly_input, weekly_output, weekly_total) = self.calculate_weekly_usage(api_key_id);

        let daily_limit_input = quota.as_ref().and_then(|q| q.daily_input_limit);
        let daily_limit_output = quota.as_ref().and_then(|q| q.daily_output_limit);
        let daily_limit_total = quota.as_ref().and_then(|q| q.daily_total_limit);
        let weekly_limit_input = quota.as_ref().and_then(|q| q.weekly_input_limit);
        let weekly_limit_output = quota.as_ref().and_then(|q| q.weekly_output_limit);
        let weekly_limit_total = quota.as_ref().and_then(|q| q.weekly_total_limit);

        // 计算剩余量
        let daily_remaining_input = daily_limit_input.map(|limit| limit.saturating_sub(daily_input));
        let daily_remaining_output = daily_limit_output.map(|limit| limit.saturating_sub(daily_output));
        let daily_remaining_total = daily_limit_total.map(|limit| limit.saturating_sub(daily_total));
        let weekly_remaining_input = weekly_limit_input.map(|limit| limit.saturating_sub(weekly_input));
        let weekly_remaining_output = weekly_limit_output.map(|limit| limit.saturating_sub(weekly_output));
        let weekly_remaining_total = weekly_limit_total.map(|limit| limit.saturating_sub(weekly_total));

        // 检查是否超出限制
        let mut exceeded = false;
        let mut reason = None;

        if let Some(limit) = daily_limit_total {
            if daily_total >= limit {
                exceeded = true;
                reason = Some(format!("Daily total token limit exceeded: {}/{} tokens", daily_total, limit));
            }
        }

        if let Some(limit) = daily_limit_input {
            if daily_input >= limit && !exceeded {
                exceeded = true;
                reason = Some(format!("Daily input token limit exceeded: {}/{} tokens", daily_input, limit));
            }
        }

        if let Some(limit) = daily_limit_output {
            if daily_output >= limit && !exceeded {
                exceeded = true;
                reason = Some(format!("Daily output token limit exceeded: {}/{} tokens", daily_output, limit));
            }
        }

        if let Some(limit) = weekly_limit_total {
            if weekly_total >= limit && !exceeded {
                exceeded = true;
                reason = Some(format!("Weekly total token limit exceeded: {}/{} tokens", weekly_total, limit));
            }
        }

        if let Some(limit) = weekly_limit_input {
            if weekly_input >= limit && !exceeded {
                exceeded = true;
                reason = Some(format!("Weekly input token limit exceeded: {}/{} tokens", weekly_input, limit));
            }
        }

        if let Some(limit) = weekly_limit_output {
            if weekly_output >= limit && !exceeded {
                exceeded = true;
                reason = Some(format!("Weekly output token limit exceeded: {}/{} tokens", weekly_output, limit));
            }
        }

        CheckQuotaResult {
            allowed: !exceeded,
            daily_used_input: daily_input,
            daily_used_output: daily_output,
            daily_used_total: daily_total,
            daily_limit_input,
            daily_limit_output,
            daily_limit_total,
            daily_remaining_input,
            daily_remaining_output,
            daily_remaining_total,
            weekly_used_input: weekly_input,
            weekly_used_output: weekly_output,
            weekly_used_total: weekly_total,
            weekly_limit_input,
            weekly_limit_output,
            weekly_limit_total,
            weekly_remaining_input,
            weekly_remaining_output,
            weekly_remaining_total,
            reason,
        }
    }

    /// 计算当日使用量（从内存统计）
    fn calculate_daily_usage(&self, api_key_id: &str) -> (u64, u64, u64) {
        let now = current_epoch_seconds();
        let day_start = truncate_to_day(now);

        if let Some(window) = self.usage_stats.get(api_key_id) {
            let input: u64 = window.hourly_stats
                .iter()
                .filter(|h| h.hour_epoch >= day_start)
                .map(|h| h.input_tokens)
                .sum();
            let output: u64 = window.hourly_stats
                .iter()
                .filter(|h| h.hour_epoch >= day_start)
                .map(|h| h.output_tokens)
                .sum();
            (input, output, input + output)
        } else {
            (0, 0, 0)
        }
    }

    /// 计算当周使用量（从内存统计）
    fn calculate_weekly_usage(&self, api_key_id: &str) -> (u64, u64, u64) {
        let now = current_epoch_seconds();
        let week_start = truncate_to_day(now - 6 * 86400); // 7天前

        if let Some(window) = self.usage_stats.get(api_key_id) {
            let input: u64 = window.daily_stats
                .iter()
                .filter(|d| d.day_epoch >= week_start)
                .map(|d| d.input_tokens)
                .sum();
            let output: u64 = window.daily_stats
                .iter()
                .filter(|d| d.day_epoch >= week_start)
                .map(|d| d.output_tokens)
                .sum();
            (input, output, input + output)
        } else {
            (0, 0, 0)
        }
    }

    /// 获取API Key的统计摘要
    pub fn get_stats_summary(&self, api_key_id: &str) -> Option<TokenStatsSummary> {
        let window = self.usage_stats.get(api_key_id)?;
        let now = current_epoch_seconds();
        let day_start = truncate_to_day(now);
        let week_start = truncate_to_day(now - 6 * 86400);

        // 今日统计
        let today_input: u64 = window.hourly_stats
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.input_tokens)
            .sum();
        let today_output: u64 = window.hourly_stats
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.output_tokens)
            .sum();

        // 本周统计
        let week_input: u64 = window.daily_stats
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.input_tokens)
            .sum();
        let week_output: u64 = window.daily_stats
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.output_tokens)
            .sum();

        Some(TokenStatsSummary {
            today_input,
            today_output,
            week_input,
            week_output,
        })
    }
}

impl Default for TokenQuotaManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Token统计摘要
#[derive(Debug, Clone)]
pub struct TokenStatsSummary {
    pub today_input: u64,
    pub today_output: u64,
    pub week_input: u64,
    pub week_output: u64,
}

/// Token配额检查器（包装器，用于与ApiKeyManager集成）
pub struct TokenQuotaChecker {
    manager: Arc<TokenQuotaManager>,
}

impl TokenQuotaChecker {
    pub fn new(manager: Arc<TokenQuotaManager>) -> Self {
        Self { manager }
    }

    /// 检查配额是否已超出
    pub fn check_quota(&self, api_key_id: &str) -> CheckQuotaResult {
        self.manager.check_quota(api_key_id)
    }

    /// 记录token使用
    pub fn record_usage(&self, api_key_id: &str, input_tokens: u64, output_tokens: u64) {
        self.manager.record_usage(api_key_id, input_tokens, output_tokens);
    }

    /// 获取manager引用
    pub fn manager(&self) -> &Arc<TokenQuotaManager> {
        &self.manager
    }
}

/// 获取当前Unix时间戳（秒）
fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 截断到小时（返回整点的Unix时间戳）
fn truncate_to_hour(timestamp: u64) -> u64 {
    (timestamp / 3600) * 3600
}

/// 截断到天（返回00:00:00的Unix时间戳）
fn truncate_to_day(timestamp: u64) -> u64 {
    (timestamp / 86400) * 86400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_functions() {
        // 2024-01-01 12:34:56 = 1704113696
        let ts = 1704113696u64;
        assert_eq!(truncate_to_hour(ts), 1704112800); // 12:00:00
        assert_eq!(truncate_to_day(ts), 1704067200);  // 00:00:00
    }

    #[test]
    fn test_record_and_check_quota() {
        let manager = TokenQuotaManager::new();
        let api_key_id = "test_key_001";

        // 设置配额
        manager.quotas.insert(api_key_id.to_string(), TokenQuotaConfig {
            daily_total_limit: Some(1000),
            daily_input_limit: None,
            daily_output_limit: None,
            weekly_total_limit: None,
            weekly_input_limit: None,
            weekly_output_limit: None,
        });

        // 初始状态应该允许
        let result = manager.check_quota(api_key_id);
        assert!(result.allowed);

        // 记录使用
        manager.record_usage(api_key_id, 500, 400);

        // 检查使用量
        let result = manager.check_quota(api_key_id);
        assert!(result.allowed);
        assert_eq!(result.daily_used_input, 500);
        assert_eq!(result.daily_used_output, 400);
        assert_eq!(result.daily_used_total, 900);

        // 记录更多使用，超出限制
        manager.record_usage(api_key_id, 200, 100);

        let result = manager.check_quota(api_key_id);
        assert!(!result.allowed);
        assert!(result.reason.is_some());
    }
}

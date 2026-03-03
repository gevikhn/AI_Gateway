pub use crate::config::{BanCondition, BanRule, BanStatus};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

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

/// 违规计数器（按时间窗口统计）
#[derive(Debug)]
pub struct ViolationCounter {
    /// 请求记录（时间戳，是否成功）
    requests: VecDeque<(u64, bool)>,
    /// 连续错误数
    consecutive_errors: u32,
    /// 最大窗口大小（用于清理过期数据）
    max_window_secs: u64,
}

impl ViolationCounter {
    pub fn new(max_window_secs: u64) -> Self {
        Self {
            requests: VecDeque::new(),
            consecutive_errors: 0,
            max_window_secs,
        }
    }

    /// 记录一次请求
    pub fn record(&mut self, timestamp: u64, success: bool) {
        self.requests.push_back((timestamp, success));

        if success {
            self.consecutive_errors = 0;
        } else {
            self.consecutive_errors += 1;
        }

        self.cleanup_old_records(timestamp);
    }

    /// 清理过期记录
    fn cleanup_old_records(&mut self, now: u64) {
        let cutoff = now.saturating_sub(self.max_window_secs);
        while self.requests.front().map(|(t, _)| *t < cutoff).unwrap_or(false) {
            self.requests.pop_front();
        }
    }

    /// 获取时间窗口内的统计
    pub fn get_window_stats(&self, window_secs: u64, now: u64) -> WindowStats {
        let cutoff = now.saturating_sub(window_secs);
        let mut total = 0u64;
        let mut errors = 0u64;

        for (timestamp, success) in &self.requests {
            if *timestamp >= cutoff {
                total += 1;
                if !success {
                    errors += 1;
                }
            }
        }

        let error_rate = if total > 0 {
            errors as f64 / total as f64
        } else {
            0.0
        };

        WindowStats {
            total,
            errors,
            error_rate,
            consecutive_errors: self.consecutive_errors,
        }
    }
}

/// 时间窗口统计
#[derive(Debug, Clone)]
pub struct WindowStats {
    pub total: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub consecutive_errors: u32,
}

/// 规则触发记录
#[derive(Debug)]
pub struct RuleTrigger {
    /// 规则ID
    pub rule_id: String,
    /// 触发时间戳
    pub triggered_at: u64,
}

/// 封禁规则引擎
#[derive(Debug)]
pub struct BanRuleEngine {
    counter: ViolationCounter,
    /// 记录每个规则的触发历史（用于多次触发才封禁）
    rule_triggers: Vec<RuleTrigger>,
}

impl BanRuleEngine {
    pub fn new(max_window_secs: u64) -> Self {
        Self {
            counter: ViolationCounter::new(max_window_secs),
            rule_triggers: Vec::new(),
        }
    }

    /// 记录请求结果并检查是否触发封禁规则
    pub fn check_rules(
        &mut self,
        rules: &[BanRule],
        now: u64,
        success: bool,
    ) -> Option<TriggeredRule> {
        // 记录请求
        self.counter.record(now, success);

        // 清理过期的触发记录
        self.cleanup_old_triggers(now, rules);

        // 检查每个启用的规则
        for rule in rules.iter().filter(|r| r.enabled) {
            if let Some(triggered) = self.check_single_rule(rule, now) {
                // 记录这次触发
                self.rule_triggers.push(RuleTrigger {
                    rule_id: rule.id.clone(),
                    triggered_at: now,
                });

                // 检查是否达到触发次数阈值
                let trigger_count = self.count_triggers_in_window(&rule.id, now, rule.trigger_window_secs);
                if trigger_count >= rule.trigger_count_threshold {
                    // 达到阈值，执行封禁，并清除该规则的触发记录
                    self.clear_triggers_for_rule(&rule.id);
                    return Some(triggered);
                }
            }
        }

        None
    }

    /// 清理过期的触发记录
    fn cleanup_old_triggers(&mut self, now: u64, rules: &[BanRule]) {
        // 找到最大的触发窗口
        let max_window = rules.iter()
            .map(|r| r.trigger_window_secs)
            .max()
            .unwrap_or(3600);

        let cutoff = now.saturating_sub(max_window);
        self.rule_triggers.retain(|t| t.triggered_at >= cutoff);
    }

    /// 统计指定规则在窗口期内的触发次数
    fn count_triggers_in_window(&self, rule_id: &str, now: u64, window_secs: u64) -> u32 {
        let cutoff = now.saturating_sub(window_secs);
        self.rule_triggers
            .iter()
            .filter(|t| t.rule_id == rule_id && t.triggered_at >= cutoff)
            .count() as u32
    }

    /// 清除指定规则的所有触发记录
    fn clear_triggers_for_rule(&mut self, rule_id: &str) {
        self.rule_triggers.retain(|t| t.rule_id != rule_id);
    }

    /// 检查单个规则
    fn check_single_rule(&self, rule: &BanRule, now: u64) -> Option<TriggeredRule> {
        match &rule.condition {
            BanCondition::ErrorRate {
                window_secs,
                threshold,
                min_requests,
            } => {
                let stats = self.counter.get_window_stats(*window_secs, now);
                if stats.total >= *min_requests && stats.error_rate >= *threshold {
                    return Some(TriggeredRule {
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        ban_duration_secs: rule.ban_duration_secs,
                        reason: format!(
                            "Error rate {:.2}% exceeded threshold {:.2}% in {} seconds",
                            stats.error_rate * 100.0,
                            threshold * 100.0,
                            window_secs
                        ),
                        metrics_snapshot: BanMetricsSnapshot {
                            requests: stats.total,
                            errors: stats.errors,
                            error_rate: stats.error_rate,
                        },
                    });
                }
            }
            BanCondition::RequestCount {
                window_secs,
                max_requests,
            } => {
                let stats = self.counter.get_window_stats(*window_secs, now);
                if stats.total >= *max_requests {
                    return Some(TriggeredRule {
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        ban_duration_secs: rule.ban_duration_secs,
                        reason: format!(
                            "Request count {} exceeded limit {} in {} seconds",
                            stats.total, max_requests, window_secs
                        ),
                        metrics_snapshot: BanMetricsSnapshot {
                            requests: stats.total,
                            errors: stats.errors,
                            error_rate: stats.error_rate,
                        },
                    });
                }
            }
            BanCondition::ConsecutiveErrors { count } => {
                let stats = self.counter.get_window_stats(self.counter.max_window_secs, now);
                if stats.consecutive_errors >= *count {
                    return Some(TriggeredRule {
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        ban_duration_secs: rule.ban_duration_secs,
                        reason: format!("{} consecutive errors exceeded threshold {}",
                            stats.consecutive_errors, count),
                        metrics_snapshot: BanMetricsSnapshot {
                            requests: stats.total,
                            errors: stats.errors,
                            error_rate: stats.error_rate,
                        },
                    });
                }
            }
        }

        None
    }
}

/// 触发的规则信息
#[derive(Debug, Clone)]
pub struct TriggeredRule {
    pub rule_id: String,
    pub rule_name: String,
    pub ban_duration_secs: u64,
    pub reason: String,
    pub metrics_snapshot: BanMetricsSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_rate_rule() {
        let mut engine = BanRuleEngine::new(300);
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_001".to_string(),
            name: "High error rate".to_string(),
            condition: BanCondition::ErrorRate {
                window_secs: 60,
                threshold: 0.5,
                min_requests: 10,
            },
            ban_duration_secs: 3600,
            enabled: true,
            trigger_count_threshold: 1, // 立即触发
            trigger_window_secs: 3600,
        }];

        // 记录10个请求，6个错误（60%错误率）
        for i in 0..10 {
            let success = i < 4; // 前4个成功，后6个错误
            let result = engine.check_rules(&rules, now + i, success);
            if i < 9 {
                assert!(result.is_none(), "Should not trigger at request {}", i);
            } else {
                // 第10次请求后，应该有6个错误/10个请求 = 60% 错误率
                assert!(result.is_some(), "Should trigger at request 10");
                let triggered = result.unwrap();
                assert_eq!(triggered.rule_id, "rule_001");
                assert!(triggered.reason.contains("Error rate"));
            }
        }
    }

    #[test]
    fn test_request_count_rule() {
        let mut engine = BanRuleEngine::new(300);
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_002".to_string(),
            name: "Too many requests".to_string(),
            condition: BanCondition::RequestCount {
                window_secs: 60,
                max_requests: 5,
            },
            ban_duration_secs: 1800,
            enabled: true,
            trigger_count_threshold: 1, // 立即触发
            trigger_window_secs: 3600,
        }];

        // 记录5个请求，第5个应该触发
        for i in 0..5 {
            let result = engine.check_rules(&rules, now + i, true);
            if i < 4 {
                assert!(result.is_none());
            } else {
                assert!(result.is_some());
                let triggered = result.unwrap();
                assert_eq!(triggered.rule_id, "rule_002");
                assert!(triggered.reason.contains("Request count"));
            }
        }
    }

    #[test]
    fn test_consecutive_errors_rule() {
        let mut engine = BanRuleEngine::new(300);
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_003".to_string(),
            name: "Consecutive errors".to_string(),
            condition: BanCondition::ConsecutiveErrors { count: 3 },
            ban_duration_secs: 900,
            enabled: true,
            trigger_count_threshold: 1, // 立即触发
            trigger_window_secs: 3600,
        }];

        // 记录3个连续错误
        for i in 0..3 {
            let result = engine.check_rules(&rules, now + i, false);
            if i < 2 {
                assert!(result.is_none());
            } else {
                assert!(result.is_some());
                let triggered = result.unwrap();
                assert_eq!(triggered.rule_id, "rule_003");
                assert!(triggered.reason.contains("consecutive errors"));
            }
        }
    }

    #[test]
    fn test_disabled_rule() {
        let mut engine = BanRuleEngine::new(300);
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_001".to_string(),
            name: "Disabled rule".to_string(),
            condition: BanCondition::RequestCount {
                window_secs: 60,
                max_requests: 1,
            },
            ban_duration_secs: 3600,
            enabled: false, // 禁用
            trigger_count_threshold: 1,
            trigger_window_secs: 3600,
        }];

        // 即使超过阈值，禁用的规则也不会触发
        let result = engine.check_rules(&rules, now, true);
        assert!(result.is_none());
    }

    #[test]
    fn test_window_cleanup() {
        let mut engine = BanRuleEngine::new(60); // 60秒窗口
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_001".to_string(),
            name: "Request count".to_string(),
            condition: BanCondition::RequestCount {
                window_secs: 60,
                max_requests: 5,
            },
            ban_duration_secs: 3600,
            enabled: true,
            trigger_count_threshold: 1,
            trigger_window_secs: 3600,
        }];

        // 记录5个请求
        for i in 0..5 {
            engine.check_rules(&rules, now + i, true);
        }

        // 70秒后，之前的记录应该过期
        let result = engine.check_rules(&rules, now + 70, true);
        assert!(result.is_none(), "Old records should be cleaned up");
    }

    #[test]
    fn test_trigger_count_threshold() {
        let mut engine = BanRuleEngine::new(300);
        let now = 1000;

        let rules = vec![BanRule {
            id: "rule_004".to_string(),
            name: "Multiple triggers required".to_string(),
            condition: BanCondition::RequestCount {
                window_secs: 60,
                max_requests: 5,
            },
            ban_duration_secs: 1800,
            enabled: true,
            trigger_count_threshold: 3, // 需要触发3次才封禁
            trigger_window_secs: 3600,
        }];

        // 第一次满足条件，记录触发但不封禁
        for i in 0..5 {
            let result = engine.check_rules(&rules, now + i, true);
            assert!(result.is_none(), "Should not ban on first trigger");
        }

        // 等待一段时间后，第二次满足条件
        let now2 = now + 100;
        for i in 0..5 {
            let result = engine.check_rules(&rules, now2 + i, true);
            assert!(result.is_none(), "Should not ban on second trigger");
        }

        // 第三次满足条件，应该封禁
        let now3 = now + 200;
        for i in 0..5 {
            let result = engine.check_rules(&rules, now3 + i, true);
            if i < 4 {
                assert!(result.is_none());
            } else {
                assert!(result.is_some(), "Should ban on third trigger");
                let triggered = result.unwrap();
                assert_eq!(triggered.rule_id, "rule_004");
            }
        }
    }
}

use crate::token_quota::TokenQuotaManager;
use crate::token_stats_storage::{TokenStatsRow, TokenStatsStorage, TokenUsageRecord};
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Token统计收集器
pub struct TokenStatsCollector {
    /// API Key级别统计（内存缓存）
    api_key_stats: DashMap<String, TokenStats>,
    /// Route级别统计（内存缓存）
    route_stats: DashMap<String, TokenStats>,
    /// SQLite存储
    storage: Option<Arc<TokenStatsStorage>>,
    /// Token配额管理器（用于实时配额检查）
    quota_manager: Option<Arc<TokenQuotaManager>>,
}

/// Token统计（内存表示）
#[derive(Debug)]
pub struct TokenStats {
    /// 最近24小时（小时粒度）
    hourly_buckets: VecDeque<HourlyTokenCount>,
    /// 最近30天（天粒度）
    daily_buckets: VecDeque<DailyTokenCount>,
}

/// 小时级Token统计
#[derive(Debug, Clone, Copy)]
pub struct HourlyTokenCount {
    pub hour_epoch: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

/// 天级Token统计
#[derive(Debug, Clone, Copy)]
pub struct DailyTokenCount {
    pub day_epoch: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

/// Token统计摘要
#[derive(Debug, Clone)]
pub struct TokenStatsSummary {
    pub today_input: u64,
    pub today_output: u64,
    pub today_total: u64,
    pub week_input: u64,
    pub week_output: u64,
    pub week_total: u64,
    pub month_input: u64,
    pub month_output: u64,
    pub month_total: u64,
    pub request_count_today: u64,
    pub request_count_week: u64,
    pub request_count_month: u64,
}

impl TokenStatsCollector {
    /// 创建新的Token统计收集器
    pub fn new(
        storage: Option<Arc<TokenStatsStorage>>,
        quota_manager: Option<Arc<TokenQuotaManager>>,
    ) -> Self {
        Self {
            api_key_stats: DashMap::new(),
            route_stats: DashMap::new(),
            storage,
            quota_manager,
        }
    }

    /// 记录token使用
    pub fn record_usage(
        &self,
        api_key_id: &str,
        route_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        request_id: Option<String>,
    ) {
        let now = current_epoch_seconds();

        tracing::info!(
            "Recording token usage: api_key_id={}, route_id={}, input={}, output={}",
            api_key_id, route_id, input_tokens, output_tokens
        );

        // 1. 更新内存统计（API Key级别）
        self.update_api_key_stats(api_key_id, input_tokens, output_tokens, now);

        // 2. 更新内存统计（Route级别）
        self.update_route_stats(route_id, input_tokens, output_tokens, now);

        // 3. 更新配额管理器（用于实时配额检查）
        if let Some(quota_manager) = &self.quota_manager {
            quota_manager.record_usage(api_key_id, input_tokens, output_tokens);
        }

        // 4. 发送到SQLite存储队列
        if let Some(storage) = &self.storage {
            let record = TokenUsageRecord {
                timestamp: now as i64,
                api_key_id: api_key_id.to_string(),
                route_id: route_id.to_string(),
                input_tokens,
                output_tokens,
                request_id,
            };
            storage.queue_record(record);
            tracing::debug!("Queued token usage record for SQLite storage");
        } else {
            tracing::debug!("No SQLite storage configured, skipping persistent storage");
        }
    }

    /// 更新API Key级别统计
    fn update_api_key_stats(
        &self,
        api_key_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        now: u64,
    ) {
        let hour_epoch = truncate_to_hour(now);
        let day_epoch = truncate_to_day(now);

        let mut stats = self.api_key_stats.entry(api_key_id.to_string()).or_insert_with(|| TokenStats {
            hourly_buckets: VecDeque::with_capacity(24),
            daily_buckets: VecDeque::with_capacity(30),
        });

        // 更新小时级统计
        if let Some(hourly) = stats.hourly_buckets.back_mut() {
            if hourly.hour_epoch == hour_epoch {
                hourly.input_tokens += input_tokens;
                hourly.output_tokens += output_tokens;
                hourly.request_count += 1;
            } else {
                stats.hourly_buckets.push_back(HourlyTokenCount {
                    hour_epoch,
                    input_tokens,
                    output_tokens,
                    request_count: 1,
                });
                // 清理过期数据
                while stats.hourly_buckets.len() > 24 {
                    stats.hourly_buckets.pop_front();
                }
            }
        } else {
            stats.hourly_buckets.push_back(HourlyTokenCount {
                hour_epoch,
                input_tokens,
                output_tokens,
                request_count: 1,
            });
        }

        // 更新天级统计
        if let Some(daily) = stats.daily_buckets.back_mut() {
            if daily.day_epoch == day_epoch {
                daily.input_tokens += input_tokens;
                daily.output_tokens += output_tokens;
                daily.request_count += 1;
            } else {
                stats.daily_buckets.push_back(DailyTokenCount {
                    day_epoch,
                    input_tokens,
                    output_tokens,
                    request_count: 1,
                });
                // 清理过期数据
                while stats.daily_buckets.len() > 30 {
                    stats.daily_buckets.pop_front();
                }
            }
        } else {
            stats.daily_buckets.push_back(DailyTokenCount {
                day_epoch,
                input_tokens,
                output_tokens,
                request_count: 1,
            });
        }
    }

    /// 更新Route级别统计
    fn update_route_stats(
        &self,
        route_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        now: u64,
    ) {
        let hour_epoch = truncate_to_hour(now);
        let day_epoch = truncate_to_day(now);

        let mut stats = self.route_stats.entry(route_id.to_string()).or_insert_with(|| TokenStats {
            hourly_buckets: VecDeque::with_capacity(24),
            daily_buckets: VecDeque::with_capacity(30),
        });

        // 更新小时级统计
        if let Some(hourly) = stats.hourly_buckets.back_mut() {
            if hourly.hour_epoch == hour_epoch {
                hourly.input_tokens += input_tokens;
                hourly.output_tokens += output_tokens;
                hourly.request_count += 1;
            } else {
                stats.hourly_buckets.push_back(HourlyTokenCount {
                    hour_epoch,
                    input_tokens,
                    output_tokens,
                    request_count: 1,
                });
                while stats.hourly_buckets.len() > 24 {
                    stats.hourly_buckets.pop_front();
                }
            }
        } else {
            stats.hourly_buckets.push_back(HourlyTokenCount {
                hour_epoch,
                input_tokens,
                output_tokens,
                request_count: 1,
            });
        }

        // 更新天级统计
        if let Some(daily) = stats.daily_buckets.back_mut() {
            if daily.day_epoch == day_epoch {
                daily.input_tokens += input_tokens;
                daily.output_tokens += output_tokens;
                daily.request_count += 1;
            } else {
                stats.daily_buckets.push_back(DailyTokenCount {
                    day_epoch,
                    input_tokens,
                    output_tokens,
                    request_count: 1,
                });
                while stats.daily_buckets.len() > 30 {
                    stats.daily_buckets.pop_front();
                }
            }
        } else {
            stats.daily_buckets.push_back(DailyTokenCount {
                day_epoch,
                input_tokens,
                output_tokens,
                request_count: 1,
            });
        }
    }

    /// 获取API Key的统计摘要
    pub fn get_api_key_summary(&self, api_key_id: &str) -> Option<TokenStatsSummary> {
        let stats = self.api_key_stats.get(api_key_id)?;
        let now = current_epoch_seconds();
        let day_start = truncate_to_day(now);
        let week_start = truncate_to_day(now - 6 * 86400);
        let month_start = truncate_to_day(now - 29 * 86400);

        // 今日统计
        let today_input: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.input_tokens)
            .sum();
        let today_output: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.output_tokens)
            .sum();
        let request_count_today: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.request_count)
            .sum();

        // 本周统计
        let week_input: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.input_tokens)
            .sum();
        let week_output: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.output_tokens)
            .sum();
        let request_count_week: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.request_count)
            .sum();

        // 本月统计
        let month_input: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.input_tokens)
            .sum();
        let month_output: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.output_tokens)
            .sum();
        let request_count_month: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.request_count)
            .sum();

        Some(TokenStatsSummary {
            today_input,
            today_output,
            today_total: today_input + today_output,
            week_input,
            week_output,
            week_total: week_input + week_output,
            month_input,
            month_output,
            month_total: month_input + month_output,
            request_count_today,
            request_count_week,
            request_count_month,
        })
    }

    /// 获取Route的统计摘要
    pub fn get_route_summary(&self, route_id: &str) -> Option<TokenStatsSummary> {
        let stats = self.route_stats.get(route_id)?;
        let now = current_epoch_seconds();
        let day_start = truncate_to_day(now);
        let week_start = truncate_to_day(now - 6 * 86400);
        let month_start = truncate_to_day(now - 29 * 86400);

        // 今日统计
        let today_input: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.input_tokens)
            .sum();
        let today_output: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.output_tokens)
            .sum();
        let request_count_today: u64 = stats.hourly_buckets
            .iter()
            .filter(|h| h.hour_epoch >= day_start)
            .map(|h| h.request_count)
            .sum();

        // 本周统计
        let week_input: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.input_tokens)
            .sum();
        let week_output: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.output_tokens)
            .sum();
        let request_count_week: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= week_start)
            .map(|d| d.request_count)
            .sum();

        // 本月统计
        let month_input: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.input_tokens)
            .sum();
        let month_output: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.output_tokens)
            .sum();
        let request_count_month: u64 = stats.daily_buckets
            .iter()
            .filter(|d| d.day_epoch >= month_start)
            .map(|d| d.request_count)
            .sum();

        Some(TokenStatsSummary {
            today_input,
            today_output,
            today_total: today_input + today_output,
            week_input,
            week_output,
            week_total: week_input + week_output,
            month_input,
            month_output,
            month_total: month_input + month_output,
            request_count_today,
            request_count_week,
            request_count_month,
        })
    }

    /// 获取所有API Key的统计
    pub fn get_all_api_key_stats(&self) -> Vec<(String, TokenStatsSummary)> {
        self.api_key_stats
            .iter()
            .filter_map(|entry| {
                let api_key_id = entry.key().clone();
                self.get_api_key_summary(&api_key_id)
                    .map(|summary| (api_key_id, summary))
            })
            .collect()
    }

    /// 获取所有Route的统计
    pub fn get_all_route_stats(&self) -> Vec<(String, TokenStatsSummary)> {
        self.route_stats
            .iter()
            .filter_map(|entry| {
                let route_id = entry.key().clone();
                self.get_route_summary(&route_id)
                    .map(|summary| (route_id, summary))
            })
            .collect()
    }

    /// 获取存储引用
    pub fn storage(&self) -> Option<&Arc<TokenStatsStorage>> {
        self.storage.as_ref()
    }

    /// 获取配额管理器引用
    pub fn quota_manager(&self) -> Option<&Arc<TokenQuotaManager>> {
        self.quota_manager.as_ref()
    }

    /// 从SQLite加载历史统计数据到内存
    /// 在启动时调用，恢复重启前的统计状态
    pub async fn load_historical_stats(&self) -> Result<(), String> {
        let storage = match &self.storage {
            Some(s) => s,
            None => {
                tracing::debug!("No SQLite storage configured, skipping historical data loading");
                return Ok(());
            }
        };

        let now = current_epoch_seconds();
        let day_start = truncate_to_day(now);
        let week_start = truncate_to_day(now - 6 * 86400);

        tracing::info!("Loading historical token stats from SQLite...");

        // 加载API Key的今日和本周统计
        match storage.query_api_key_stats_for_restore(day_start as i64, week_start as i64).await {
            Ok(records) => {
                for record in records {
                    self.restore_api_key_stats(&record);
                }
                tracing::info!("Loaded {} API key stat records from SQLite", self.api_key_stats.len());
            }
            Err(e) => {
                tracing::warn!("Failed to load API key historical stats: {}", e);
            }
        }

        // 加载Route的今日和本周统计
        match storage.query_route_stats_for_restore(day_start as i64, week_start as i64).await {
            Ok(records) => {
                for record in records {
                    self.restore_route_stats(&record);
                }
                tracing::info!("Loaded {} route stat records from SQLite", self.route_stats.len());
            }
            Err(e) => {
                tracing::warn!("Failed to load route historical stats: {}", e);
            }
        }

        Ok(())
    }

    /// 恢复单个API Key的统计
    fn restore_api_key_stats(&self, record: &TokenStatsRow) {
        let hour_epoch = truncate_to_hour(record.time_bucket as u64);
        let day_epoch = truncate_to_day(record.time_bucket as u64);

        let api_key_id = record.api_key_id.clone().unwrap_or_default();
        if api_key_id.is_empty() {
            return;
        }

        let mut stats = self.api_key_stats.entry(api_key_id).or_insert_with(|| TokenStats {
            hourly_buckets: VecDeque::with_capacity(24),
            daily_buckets: VecDeque::with_capacity(30),
        });

        // 恢复小时级统计
        if let Some(hourly) = stats.hourly_buckets.iter_mut().find(|h| h.hour_epoch == hour_epoch) {
            hourly.input_tokens += record.input_tokens;
            hourly.output_tokens += record.output_tokens;
            hourly.request_count += record.request_count;
        } else {
            stats.hourly_buckets.push_back(HourlyTokenCount {
                hour_epoch,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                request_count: record.request_count,
            });
        }

        // 恢复天级统计
        if let Some(daily) = stats.daily_buckets.iter_mut().find(|d| d.day_epoch == day_epoch) {
            daily.input_tokens += record.input_tokens;
            daily.output_tokens += record.output_tokens;
            daily.request_count += record.request_count;
        } else {
            stats.daily_buckets.push_back(DailyTokenCount {
                day_epoch,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                request_count: record.request_count,
            });
        }
    }

    /// 恢复单个Route的统计
    fn restore_route_stats(&self, record: &TokenStatsRow) {
        let hour_epoch = truncate_to_hour(record.time_bucket as u64);
        let day_epoch = truncate_to_day(record.time_bucket as u64);

        let route_id = record.route_id.clone().unwrap_or_default();
        if route_id.is_empty() {
            return;
        }

        let mut stats = self.route_stats.entry(route_id).or_insert_with(|| TokenStats {
            hourly_buckets: VecDeque::with_capacity(24),
            daily_buckets: VecDeque::with_capacity(30),
        });

        // 恢复小时级统计
        if let Some(hourly) = stats.hourly_buckets.iter_mut().find(|h| h.hour_epoch == hour_epoch) {
            hourly.input_tokens += record.input_tokens;
            hourly.output_tokens += record.output_tokens;
            hourly.request_count += record.request_count;
        } else {
            stats.hourly_buckets.push_back(HourlyTokenCount {
                hour_epoch,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                request_count: record.request_count,
            });
        }

        // 恢复天级统计
        if let Some(daily) = stats.daily_buckets.iter_mut().find(|d| d.day_epoch == day_epoch) {
            daily.input_tokens += record.input_tokens;
            daily.output_tokens += record.output_tokens;
            daily.request_count += record.request_count;
        } else {
            stats.daily_buckets.push_back(DailyTokenCount {
                day_epoch,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                request_count: record.request_count,
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_get_summary() {
        let collector = TokenStatsCollector::new(None, None);
        let api_key_id = "test_key_001";
        let route_id = "test_route";

        // 记录一些使用
        collector.record_usage(api_key_id, route_id, 100, 50, None);
        collector.record_usage(api_key_id, route_id, 200, 100, None);

        // 检查API Key统计
        let summary = collector.get_api_key_summary(api_key_id).unwrap();
        assert_eq!(summary.today_input, 300);
        assert_eq!(summary.today_output, 150);
        assert_eq!(summary.today_total, 450);
        assert_eq!(summary.request_count_today, 2);

        // 检查Route统计
        let route_summary = collector.get_route_summary(route_id).unwrap();
        assert_eq!(route_summary.today_input, 300);
        assert_eq!(route_summary.today_output, 150);
        assert_eq!(route_summary.request_count_today, 2);
    }
}

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum RateLimitDecision {
    Allowed,
    Rejected { retry_after_secs: u64 },
}

pub struct RateLimiter {
    per_minute: u64,
    state: Mutex<RateLimiterState>,
}

struct RateLimiterState {
    minute_bucket: u64,
    counters: HashMap<String, u64>,
}

impl RateLimiter {
    pub fn new(per_minute: u64) -> Self {
        Self {
            per_minute,
            state: Mutex::new(RateLimiterState {
                minute_bucket: current_epoch_seconds() / 60,
                counters: HashMap::new(),
            }),
        }
    }

    pub fn check(&self, token: &str, route_id: &str) -> RateLimitDecision {
        self.check_at_epoch_seconds(token, route_id, current_epoch_seconds())
    }

    fn check_at_epoch_seconds(
        &self,
        token: &str,
        route_id: &str,
        epoch_seconds: u64,
    ) -> RateLimitDecision {
        let minute_bucket = epoch_seconds / 60;
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if state.minute_bucket != minute_bucket {
            state.minute_bucket = minute_bucket;
            state.counters.clear();
        }

        let key = format!("{route_id}\n{token}");
        let counter = state.counters.entry(key).or_insert(0);

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
    use super::{RateLimitDecision, RateLimiter};

    #[test]
    fn allows_until_limit_then_rejects() {
        let limiter = RateLimiter::new(2);
        let now = 1_700_000_040;

        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", "openai", now),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", "openai", now),
            RateLimitDecision::Allowed
        ));

        match limiter.check_at_epoch_seconds("gw_token", "openai", now) {
            RateLimitDecision::Rejected { retry_after_secs } => {
                assert!((1..=60).contains(&retry_after_secs));
            }
            RateLimitDecision::Allowed => panic!("third request should be rejected"),
        }
    }

    #[test]
    fn separates_counters_by_route_and_token() {
        let limiter = RateLimiter::new(1);
        let now = 1_700_000_040;

        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token_a", "openai", now),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token_b", "openai", now),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token_a", "claude", now),
            RateLimitDecision::Allowed
        ));
    }

    #[test]
    fn rotates_window_every_minute() {
        let limiter = RateLimiter::new(1);
        let t1 = 1_700_000_040;
        let t2 = t1 + 61;

        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", "openai", t1),
            RateLimitDecision::Allowed
        ));
        assert!(matches!(
            limiter.check_at_epoch_seconds("gw_token", "openai", t2),
            RateLimitDecision::Allowed
        ));
    }
}

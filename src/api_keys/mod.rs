pub mod ban;
pub mod ban_log;
pub mod manager;

pub use ban::{BanCondition, BanRule, BanStatus, BanMetricsSnapshot, BanRuleEngine, TriggeredRule};
pub use ban_log::{BanLogEntry, BanLogStore, SqliteBanLogStore, InMemoryBanLogStore};
pub use manager::{ApiKeyManager, ApiKeyError, ApiKeyRuntimeInfo, ValidationResult, RequestResult, create_api_key_manager};

/// 生成 API Key ID（从 key 值生成）
pub fn generate_key_id(key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();

    // 使用前缀 + 哈希值的前8位
    format!("ak_{:08x}", hash)
}

/// 获取当前 Unix 时间戳（秒）
pub fn current_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key_id() {
        let id1 = generate_key_id("test_key_1");
        let id2 = generate_key_id("test_key_1");
        let id3 = generate_key_id("test_key_2");

        // 相同的 key 应该生成相同的 id
        assert_eq!(id1, id2);
        // 不同的 key 应该生成不同的 id
        assert_ne!(id1, id3);
        // id 应该以 ak_ 开头
        assert!(id1.starts_with("ak_"));
    }
}

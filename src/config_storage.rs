use crate::config::AppConfig;
use crate::config::ResolvedApiKey;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{error, info};

/// Configuration validation result for API keys
#[derive(Debug, Clone)]
pub enum ConfigValidationResult {
    Valid { key_id: String },
    Invalid,
    Deleted { key_id: String },
    NotFound,
}

/// Sync result containing statistics about the sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub routes_added: usize,
    pub routes_updated: usize,
    pub routes_deleted: usize,
    pub api_keys_added: usize,
    pub api_keys_updated: usize,
    pub api_keys_deleted: usize,
}

/// Configuration storage manager
///
/// Tracks route and API key configurations in a SQLite database,
/// with in-memory caching for fast validation lookups.
#[derive(Debug)]
pub struct ConfigStorage {
    pool: Pool<Sqlite>,
    /// In-memory cache: currently valid route IDs
    valid_routes: RwLock<HashSet<String>>,
    /// In-memory cache: currently valid API Key hashes (key_hash -> key_id)
    valid_api_keys: RwLock<HashMap<String, String>>,
}

impl ConfigStorage {
    /// Initialize database connection and table structure
    pub async fn new(db_path: &str) -> Result<Self, String> {
        // Ensure directory exists
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        // Create connection pool
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path))
            .map_err(|e| format!("Invalid connection string: {}", e))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(|e| format!("Failed to connect to database: {}", e))?;

        // Create tables
        Self::create_tables(&pool).await?;

        let storage = Self {
            pool,
            valid_routes: RwLock::new(HashSet::new()),
            valid_api_keys: RwLock::new(HashMap::new()),
        };

        // Load initial cache
        storage.reload_cache().await?;

        info!("ConfigStorage initialized at: {}", db_path);
        Ok(storage)
    }

    /// Create database tables
    async fn create_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
        // Routes table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS routes (
                id TEXT PRIMARY KEY,
                prefix TEXT NOT NULL,
                config_hash TEXT NOT NULL,
                is_deleted BOOLEAN NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create routes table: {}", e))?;

        // API Keys table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS api_keys (
                id TEXT PRIMARY KEY,
                key_hash TEXT NOT NULL UNIQUE,
                key_preview TEXT NOT NULL,
                config_hash TEXT NOT NULL,
                is_deleted BOOLEAN NOT NULL DEFAULT 0,
                is_enabled BOOLEAN NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create api_keys table: {}", e))?;

        // Create indexes
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_routes_deleted ON routes(is_deleted)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create routes deleted index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create api_keys hash index: {}", e))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_api_keys_deleted ON api_keys(is_deleted)"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create api_keys deleted index: {}", e))?;

        info!("Config storage database tables created successfully");
        Ok(())
    }

    /// Reload in-memory cache from database
    async fn reload_cache(&self) -> Result<(), String> {
        // Load valid routes
        let routes: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM routes WHERE is_deleted = 0"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load routes from database: {}", e))?;

        let route_set: HashSet<String> = routes.into_iter().map(|r| r.0).collect();

        // Load valid API keys
        let keys: Vec<(String, String)> = sqlx::query_as(
            "SELECT key_hash, id FROM api_keys WHERE is_deleted = 0 AND is_enabled = 1"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to load api_keys from database: {}", e))?;

        let key_map: HashMap<String, String> = keys.into_iter().collect();

        // Update caches
        {
            let mut routes_cache = self.valid_routes.write().await;
            *routes_cache = route_set;
        }
        {
            let mut keys_cache = self.valid_api_keys.write().await;
            *keys_cache = key_map;
        }

        info!("Config cache reloaded: {} routes, {} api_keys",
            self.valid_routes.read().await.len(),
            self.valid_api_keys.read().await.len()
        );
        Ok(())
    }

    /// Synchronize configuration to database
    ///
    /// Logic:
    /// 1. Calculate hashes for all current routes and api_keys in config
    /// 2. Compare with existing records in database
    /// 3. New records: insert with is_deleted=false
    /// 4. Existing records: update config_hash and updated_at
    /// 5. Records not in config: mark is_deleted=true, set deleted_at
    /// 6. Reload in-memory cache
    pub async fn sync_config(&self, config: &AppConfig) -> Result<SyncResult, String> {
        let mut tx = self.pool.begin().await
            .map_err(|e| format!("Failed to begin transaction: {}", e))?;
        let now = current_epoch_seconds() as i64;

        let mut result = SyncResult {
            routes_added: 0,
            routes_updated: 0,
            routes_deleted: 0,
            api_keys_added: 0,
            api_keys_updated: 0,
            api_keys_deleted: 0,
        };

        // ===== Sync Routes =====
        let current_routes: HashMap<String, (String, String)> = config
            .routes
            .as_ref()
            .map(|routes| {
                routes
                    .iter()
                    .map(|route| {
                        let hash = compute_route_config_hash(route);
                        (route.id.clone(), (route.prefix.clone(), hash))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Get existing routes from database
        let existing_routes: Vec<(String, String, bool)> = sqlx::query_as(
            "SELECT id, config_hash, is_deleted FROM routes"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| format!("Failed to fetch existing routes: {}", e))?;

        let existing_route_map: HashMap<String, (String, bool)> = existing_routes
            .into_iter()
            .map(|(id, hash, deleted)| (id, (hash, deleted)))
            .collect();

        // Process current routes (add or update)
        for (route_id, (prefix, config_hash)) in &current_routes {
            if let Some((existing_hash, was_deleted)) = existing_route_map.get(route_id) {
                if existing_hash != config_hash || *was_deleted {
                    // Update existing route
                    sqlx::query(
                        r#"
                        UPDATE routes
                        SET prefix = ?1, config_hash = ?2, is_deleted = 0, deleted_at = NULL, updated_at = ?3
                        WHERE id = ?4
                        "#
                    )
                    .bind(prefix)
                    .bind(config_hash)
                    .bind(now)
                    .bind(route_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| format!("Failed to update route: {}", e))?;

                    if *was_deleted {
                        result.routes_added += 1;
                        info!("Route {} restored from deleted state", route_id);
                    } else {
                        result.routes_updated += 1;
                        info!("Route {} updated", route_id);
                    }
                }
            } else {
                // Insert new route
                sqlx::query(
                    r#"
                    INSERT INTO routes (id, prefix, config_hash, is_deleted, created_at, updated_at)
                    VALUES (?1, ?2, ?3, 0, ?4, ?4)
                    "#
                )
                .bind(route_id)
                .bind(prefix)
                .bind(config_hash)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Failed to insert route: {}", e))?;

                result.routes_added += 1;
                info!("Route {} added", route_id);
            }
        }

        // Mark deleted routes
        for (route_id, (_, _)) in &existing_route_map {
            if !current_routes.contains_key(route_id) {
                sqlx::query(
                    r#"
                    UPDATE routes
                    SET is_deleted = 1, deleted_at = ?1, updated_at = ?1
                    WHERE id = ?2 AND is_deleted = 0
                    "#
                )
                .bind(now)
                .bind(route_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Failed to mark route as deleted: {}", e))?;

                result.routes_deleted += 1;
                info!("Route {} marked as deleted", route_id);
            }
        }

        // ===== Sync API Keys =====
        let current_keys: HashMap<String, (String, String, String, bool)> = config
            .resolved_api_keys()
            .into_iter()
            .map(|key| {
                let hash = compute_api_key_config_hash(&key);
                let key_hash = compute_key_hash(&key.key);
                let key_preview = key.key.chars().take(8).collect::<String>();
                (
                    key.id.clone(),
                    (key_hash, key_preview, hash, key.enabled),
                )
            })
            .collect();

        // Get existing API keys from database
        let existing_keys: Vec<(String, String, bool, bool)> = sqlx::query_as(
            "SELECT id, config_hash, is_deleted, is_enabled FROM api_keys"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| format!("Failed to fetch existing api_keys: {}", e))?;

        let existing_key_map: HashMap<String, (String, bool, bool)> = existing_keys
            .into_iter()
            .map(|(id, hash, deleted, enabled)| (id, (hash, deleted, enabled)))
            .collect();

        // Process current API keys (add or update)
        for (key_id, (key_hash, key_preview, config_hash, is_enabled)) in &current_keys {
            if let Some((existing_hash, was_deleted, _)) = existing_key_map.get(key_id) {
                if existing_hash != config_hash || *was_deleted || !is_enabled {
                    // Update existing key
                    sqlx::query(
                        r#"
                        UPDATE api_keys
                        SET key_hash = ?1, key_preview = ?2, config_hash = ?3,
                            is_deleted = 0, deleted_at = NULL, is_enabled = ?4, updated_at = ?5
                        WHERE id = ?6
                        "#
                    )
                    .bind(key_hash)
                    .bind(key_preview)
                    .bind(config_hash)
                    .bind(is_enabled)
                    .bind(now)
                    .bind(key_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| format!("Failed to update api_key: {}", e))?;

                    if *was_deleted {
                        result.api_keys_added += 1;
                        info!("API Key {} restored from deleted state", key_id);
                    } else {
                        result.api_keys_updated += 1;
                        info!("API Key {} updated", key_id);
                    }
                }
            } else {
                // Insert new API key
                sqlx::query(
                    r#"
                    INSERT INTO api_keys (id, key_hash, key_preview, config_hash, is_deleted, is_enabled, created_at, updated_at)
                    VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?6)
                    "#
                )
                .bind(key_id)
                .bind(key_hash)
                .bind(key_preview)
                .bind(config_hash)
                .bind(is_enabled)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Failed to insert api_key: {}", e))?;

                result.api_keys_added += 1;
                info!("API Key {} added", key_id);
            }
        }

        // Mark deleted API keys
        for (key_id, _) in &existing_key_map {
            if !current_keys.contains_key(key_id) {
                sqlx::query(
                    r#"
                    UPDATE api_keys
                    SET is_deleted = 1, deleted_at = ?1, updated_at = ?1
                    WHERE id = ?2 AND is_deleted = 0
                    "#
                )
                .bind(now)
                .bind(key_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Failed to mark api_key as deleted: {}", e))?;

                result.api_keys_deleted += 1;
                info!("API Key {} marked as deleted", key_id);
            }
        }

        // Commit transaction
        tx.commit().await
            .map_err(|e| format!("Failed to commit transaction: {}", e))?;

        // Reload cache
        self.reload_cache().await?;

        info!(
            "Config sync completed: routes(+{} ~{} -{}), api_keys(+{} ~{} -{})",
            result.routes_added,
            result.routes_updated,
            result.routes_deleted,
            result.api_keys_added,
            result.api_keys_updated,
            result.api_keys_deleted
        );

        Ok(result)
    }

    /// Validate API Key by its value (SHA256 hash lookup)
    ///
    /// Returns ConfigValidationResult indicating the key's status
    pub async fn validate_api_key(&self, key_value: &str) -> ConfigValidationResult {
        let key_hash = compute_key_hash(key_value);

        // Check in-memory cache first
        {
            let cache = self.valid_api_keys.read().await;
            if let Some(key_id) = cache.get(&key_hash) {
                return ConfigValidationResult::Valid {
                    key_id: key_id.clone(),
                };
            }
        }

        // Not in cache, check database (might be deleted or not exist)
        match sqlx::query_as::<_, (String, bool, bool)>(
            "SELECT id, is_deleted, is_enabled FROM api_keys WHERE key_hash = ?1"
        )
        .bind(&key_hash)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(Some((key_id, is_deleted, is_enabled))) => {
                if is_deleted {
                    ConfigValidationResult::Deleted { key_id }
                } else if !is_enabled {
                    ConfigValidationResult::Invalid
                } else {
                    // Should have been in cache, but return valid anyway
                    ConfigValidationResult::Valid { key_id }
                }
            }
            Ok(None) => ConfigValidationResult::NotFound,
            Err(e) => {
                error!("Failed to validate api_key from database: {}", e);
                ConfigValidationResult::NotFound
            }
        }
    }

    /// Validate Route by its ID
    ///
    /// Returns true if the route exists and is not deleted
    pub async fn validate_route(&self, route_id: &str) -> bool {
        // Check in-memory cache first
        {
            let cache = self.valid_routes.read().await;
            if cache.contains(route_id) {
                return true;
            }
        }

        // Not in cache, check database
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM routes WHERE id = ?1 AND is_deleted = 0)"
        )
        .bind(route_id)
        .fetch_one(&self.pool)
        .await
        {
            Ok(exists) => exists,
            Err(e) => {
                error!("Failed to validate route from database: {}", e);
                false
            }
        }
    }

    /// Get all valid route IDs (for filtering)
    pub async fn get_valid_route_ids(&self) -> HashSet<String> {
        self.valid_routes.read().await.clone()
    }

    /// Get all valid API Key IDs (for filtering)
    pub async fn get_valid_api_key_ids(&self) -> HashSet<String> {
        let cache = self.valid_api_keys.read().await;
        cache.values().cloned().collect()
    }

    /// Get all valid API Key value hashes (for filtering metrics by key value)
    pub async fn get_valid_api_key_hashes(&self) -> HashSet<String> {
        let cache = self.valid_api_keys.read().await;
        cache.keys().cloned().collect()
    }

    /// Get API Key ID by key value (for lookup)
    pub async fn get_api_key_id(&self, key_value: &str) -> Option<String> {
        let key_hash = compute_key_hash(key_value);
        self.valid_api_keys.read().await.get(&key_hash).cloned()
    }

    /// Check if an API Key ID is valid (by ID, not by key value)
    pub async fn is_api_key_id_valid(&self, key_id: &str) -> bool {
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?1 AND is_deleted = 0 AND is_enabled = 1)"
        )
        .bind(key_id)
        .fetch_one(&self.pool)
        .await
        {
            Ok(exists) => exists,
            Err(e) => {
                error!("Failed to check api_key_id validity: {}", e);
                false
            }
        }
    }

    /// Check if an API Key value is valid (by key value, not by ID)
    /// Returns true if the key exists, is not deleted, and is enabled
    pub async fn is_api_key_value_valid(&self, key_value: &str) -> bool {
        matches!(
            self.validate_api_key(key_value).await,
            ConfigValidationResult::Valid { .. }
        )
    }
}

/// Compute SHA256 hash of a string
fn compute_key_hash(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute configuration hash for a RouteConfig
fn compute_route_config_hash(route: &crate::config::RouteConfig) -> String {
    // Hash based on key fields that define the route configuration
    let mut hasher = Sha256::new();
    hasher.update(route.id.as_bytes());
    hasher.update(route.prefix.as_bytes());
    hasher.update(route.upstream.base_url.as_bytes());
    hasher.update(route.upstream.strip_prefix.to_string().as_bytes());
    hasher.update(route.upstream.connect_timeout_ms.to_string().as_bytes());
    hasher.update(route.upstream.request_timeout_ms.to_string().as_bytes());
    // Include inject_headers
    for header in &route.upstream.inject_headers {
        hasher.update(header.name.as_bytes());
        hasher.update(header.value.as_bytes());
    }
    // Include remove_headers
    let mut remove_headers = route.upstream.remove_headers.clone();
    remove_headers.sort();
    for header in remove_headers {
        hasher.update(header.as_bytes());
    }
    hasher.update(route.upstream.forward_xff.to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute configuration hash for a ResolvedApiKey
fn compute_api_key_config_hash(key: &ResolvedApiKey) -> String {
    // Hash based on key fields that define the API key configuration
    let mut hasher = Sha256::new();
    hasher.update(key.id.as_bytes());
    hasher.update(key.key.as_bytes());
    hasher.update(key.enabled.to_string().as_bytes());
    hasher.update(key.remark.as_bytes());

    // Include route permissions in hash
    if let Some(route_id) = &key.route_id {
        hasher.update(route_id.as_bytes());
    }
    if let Some(route_ids) = &key.route_ids {
        let mut sorted: Vec<String> = route_ids.clone();
        sorted.sort();
        for id in sorted {
            hasher.update(id.as_bytes());
        }
    }

    // Include rate limit config
    if let Some(rl) = &key.rate_limit {
        hasher.update(rl.per_minute.to_string().as_bytes());
    }

    // Include concurrency config
    if let Some(cc) = &key.concurrency {
        if let Some(max) = cc.downstream_max_inflight {
            hasher.update(max.to_string().as_bytes());
        }
    }

    // Include token quota config
    if let Some(tq) = &key.token_quota {
        if let Some(limit) = tq.daily_total_limit {
            hasher.update(format!("daily_total:{}", limit).as_bytes());
        }
        if let Some(limit) = tq.daily_input_limit {
            hasher.update(format!("daily_input:{}", limit).as_bytes());
        }
        if let Some(limit) = tq.daily_output_limit {
            hasher.update(format!("daily_output:{}", limit).as_bytes());
        }
        if let Some(limit) = tq.weekly_total_limit {
            hasher.update(format!("weekly_total:{}", limit).as_bytes());
        }
        if let Some(limit) = tq.weekly_input_limit {
            hasher.update(format!("weekly_input:{}", limit).as_bytes());
        }
        if let Some(limit) = tq.weekly_output_limit {
            hasher.update(format!("weekly_output:{}", limit).as_bytes());
        }
    }

    format!("{:x}", hasher.finalize())
}

/// Get current Unix timestamp in seconds
fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_key_hash() {
        let hash1 = compute_key_hash("test-key-123");
        let hash2 = compute_key_hash("test-key-123");
        let hash3 = compute_key_hash("different-key");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA256 hex string length
    }

    #[test]
    fn test_compute_route_config_hash() {
        use crate::config::RouteConfig;
        use crate::config::UpstreamConfig;

        let route1 = RouteConfig {
            id: "test-route".to_string(),
            prefix: "/test".to_string(),
            upstream: UpstreamConfig {
                base_url: "http://example.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 5000,
                request_timeout_ms: 30000,
                inject_headers: vec![],
                remove_headers: vec![],
                forward_xff: true,
                proxy: None,
                upstream_key_max_inflight: None,
                user_agent: None,
            },
        };
        let route2 = RouteConfig {
            id: "test-route".to_string(),
            prefix: "/test".to_string(),
            upstream: UpstreamConfig {
                base_url: "http://example.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 5000,
                request_timeout_ms: 30000,
                inject_headers: vec![],
                remove_headers: vec![],
                forward_xff: true,
                proxy: None,
                upstream_key_max_inflight: None,
                user_agent: None,
            },
        };
        let route3 = RouteConfig {
            id: "test-route".to_string(),
            prefix: "/test".to_string(),
            upstream: UpstreamConfig {
                base_url: "http://different.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 5000,
                request_timeout_ms: 30000,
                inject_headers: vec![],
                remove_headers: vec![],
                forward_xff: true,
                proxy: None,
                upstream_key_max_inflight: None,
                user_agent: None,
            },
        };

        let hash1 = compute_route_config_hash(&route1);
        let hash2 = compute_route_config_hash(&route2);
        let hash3 = compute_route_config_hash(&route3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_compute_api_key_config_hash() {
        let key1 = ResolvedApiKey {
            id: "key-1".to_string(),
            key: "secret-key-123".to_string(),
            route_id: Some("route-1".to_string()),
            route_ids: None,
            enabled: true,
            remark: "Test key".to_string(),
            rate_limit: None,
            concurrency: None,
            token_quota: None,
            ban_rules: vec![],
            ban_status: None,
        };
        let key2 = ResolvedApiKey {
            id: "key-1".to_string(),
            key: "secret-key-123".to_string(),
            route_id: Some("route-1".to_string()),
            route_ids: None,
            enabled: true,
            remark: "Test key".to_string(),
            rate_limit: None,
            concurrency: None,
            token_quota: None,
            ban_rules: vec![],
            ban_status: None,
        };
        let key3 = ResolvedApiKey {
            id: "key-1".to_string(),
            key: "different-key".to_string(),
            route_id: Some("route-1".to_string()),
            route_ids: None,
            enabled: true,
            remark: "Test key".to_string(),
            rate_limit: None,
            concurrency: None,
            token_quota: None,
            ban_rules: vec![],
            ban_status: None,
        };

        let hash1 = compute_api_key_config_hash(&key1);
        let hash2 = compute_api_key_config_hash(&key2);
        let hash3 = compute_api_key_config_hash(&key3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}

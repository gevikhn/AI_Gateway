use crate::config::{ApiKeyConcurrencyConfig, AppConfig, RouteConfig};
use http::header::AUTHORIZATION;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Debug)]
pub enum ConcurrencyError {
    DownstreamLimitExceeded,
    UpstreamLimitExceeded,
}

/// 并发控制器，支持 API Key 级别的并发限制
pub struct ConcurrencyController {
    /// 全局下游并发限制
    downstream_semaphore: Option<Arc<Semaphore>>,
    /// 全局上游默认限制
    upstream_default_limit: Option<usize>,
    /// 上游并发信号量（按 key）
    upstream_semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// API Key 级别的并发配置
    api_key_configs: HashMap<String, ApiKeyConcurrencyConfig>,
}

/// 解析后的并发限制配置
#[derive(Debug, Clone)]
pub struct ResolvedConcurrencyConfig {
    pub downstream_limit: Option<usize>,
    pub upstream_limit: Option<usize>,
}

impl ConcurrencyController {
    pub fn new(config: &AppConfig) -> Option<Self> {
        let downstream_limit = config
            .concurrency
            .as_ref()
            .and_then(|concurrency| concurrency.downstream_max_inflight);
        let upstream_default_limit = config
            .concurrency
            .as_ref()
            .and_then(|concurrency| concurrency.upstream_per_key_max_inflight);
        let has_route_override = config
            .routes
            .iter()
            .any(|route| route.upstream.upstream_key_max_inflight.is_some());

        // 收集 API Key 级别的并发配置
        let mut api_key_configs = HashMap::new();
        if let Some(api_keys_global) = &config.api_keys {
            for api_key_config in &api_keys_global.keys {
                if let Some(concurrency) = &api_key_config.concurrency {
                    api_key_configs.insert(api_key_config.key.clone(), concurrency.clone());
                }
            }
        }

        let has_api_key_concurrency = !api_key_configs.is_empty();

        if downstream_limit.is_none()
            && upstream_default_limit.is_none()
            && !has_route_override
            && !has_api_key_concurrency
        {
            return None;
        }

        Some(Self {
            downstream_semaphore: downstream_limit.map(|limit| Arc::new(Semaphore::new(limit))),
            upstream_default_limit,
            upstream_semaphores: Mutex::new(HashMap::new()),
            api_key_configs,
        })
    }

    /// 获取解析后的并发配置（考虑 API Key 级别配置）
    ///
    /// 配置继承：api_key级 > 路由级 > 全局级
    pub fn resolve_config(
        &self,
        api_key: Option<&str>,
        route: &RouteConfig,
    ) -> ResolvedConcurrencyConfig {
        // 获取 API Key 级别的配置
        let api_key_config = api_key.and_then(|key| self.api_key_configs.get(key));

        // 下游限制：api_key级 > 全局级
        let downstream_limit = api_key_config
            .and_then(|c| c.downstream_max_inflight)
            .or_else(|| self.downstream_semaphore.as_ref().map(|s| s.available_permits()));

        // 上游限制：api_key级 > 路由级 > 全局级
        let upstream_limit = api_key_config
            .and_then(|c| c.upstream_per_key_max_inflight)
            .or_else(|| route.upstream.upstream_key_max_inflight)
            .or(self.upstream_default_limit);

        ResolvedConcurrencyConfig {
            downstream_limit,
            upstream_limit,
        }
    }

    /// 获取 API Key 级别的下游并发限制
    fn get_api_key_downstream_limit(&self, api_key: &str) -> Option<usize> {
        self.api_key_configs
            .get(api_key)
            .and_then(|c| c.downstream_max_inflight)
    }

    pub fn acquire_downstream(&self) -> Result<Option<OwnedSemaphorePermit>, ConcurrencyError> {
        let Some(semaphore) = &self.downstream_semaphore else {
            return Ok(None);
        };

        semaphore
            .clone()
            .try_acquire_owned()
            .map(Some)
            .map_err(map_acquire_error_to_downstream)
    }

    /// 获取下游并发许可（支持 API Key 级别限制）
    pub async fn acquire_downstream_for_key(
        &self,
        api_key: &str,
    ) -> Result<Option<OwnedSemaphorePermit>, ConcurrencyError> {
        // 优先使用 API Key 级别的限制
        if let Some(limit) = self.get_api_key_downstream_limit(api_key) {
            let semaphore_key = format!("downstream:{api_key}");
            let semaphore = {
                let mut semaphores = self.upstream_semaphores.lock().await;
                semaphores
                    .entry(semaphore_key)
                    .or_insert_with(|| Arc::new(Semaphore::new(limit)))
                    .clone()
            };

            return semaphore
                .try_acquire_owned()
                .map(Some)
                .map_err(map_acquire_error_to_downstream);
        }

        // 回退到全局限制
        self.acquire_downstream()
    }

    pub async fn acquire_upstream(
        &self,
        route: &RouteConfig,
    ) -> Result<Option<OwnedSemaphorePermit>, ConcurrencyError> {
        let Some(limit) = route
            .upstream
            .upstream_key_max_inflight
            .or(self.upstream_default_limit)
        else {
            return Ok(None);
        };

        let key_material = extract_upstream_key_from_injected_headers(route)
            .unwrap_or_else(|| "default".to_string());
        let key_fingerprint = fingerprint(&key_material);
        let semaphore_key = format!("{}:{key_fingerprint:016x}", route.id);

        let semaphore = {
            let mut semaphores = self.upstream_semaphores.lock().await;
            semaphores
                .entry(semaphore_key)
                .or_insert_with(|| Arc::new(Semaphore::new(limit)))
                .clone()
        };

        semaphore
            .try_acquire_owned()
            .map(Some)
            .map_err(map_acquire_error_to_upstream)
    }

    /// 获取上游并发许可（支持 API Key 级别限制）
    pub async fn acquire_upstream_for_key(
        &self,
        api_key: &str,
        route: &RouteConfig,
    ) -> Result<Option<OwnedSemaphorePermit>, ConcurrencyError> {
        // 配置继承：api_key级 > 路由级 > 全局级
        let limit = self
            .api_key_configs
            .get(api_key)
            .and_then(|c| c.upstream_per_key_max_inflight)
            .or_else(|| route.upstream.upstream_key_max_inflight)
            .or(self.upstream_default_limit);

        let Some(limit) = limit else {
            return Ok(None);
        };

        // 使用 API Key 作为信号量 key 的一部分
        let key_material = extract_upstream_key_from_injected_headers(route)
            .unwrap_or_else(|| api_key.to_string());
        let key_fingerprint = fingerprint(&key_material);
        let semaphore_key = format!("{}:{}:{key_fingerprint:016x}", route.id, api_key);

        let semaphore = {
            let mut semaphores = self.upstream_semaphores.lock().await;
            semaphores
                .entry(semaphore_key)
                .or_insert_with(|| Arc::new(Semaphore::new(limit)))
                .clone()
        };

        semaphore
            .try_acquire_owned()
            .map(Some)
            .map_err(map_acquire_error_to_upstream)
    }
}

fn map_acquire_error_to_downstream(_: TryAcquireError) -> ConcurrencyError {
    ConcurrencyError::DownstreamLimitExceeded
}

fn map_acquire_error_to_upstream(_: TryAcquireError) -> ConcurrencyError {
    ConcurrencyError::UpstreamLimitExceeded
}

fn extract_upstream_key_from_injected_headers(route: &RouteConfig) -> Option<String> {
    for header_name in upstream_key_header_names() {
        let Some(text) = find_injected_header_value(route, header_name) else {
            continue;
        };

        if header_name.eq_ignore_ascii_case(AUTHORIZATION.as_str()) {
            let bearer = parse_bearer_token(text).unwrap_or(text);
            return Some(format!("authorization:{bearer}"));
        }

        return Some(format!("{header_name}:{text}"));
    }

    None
}

fn find_injected_header_value<'a>(route: &'a RouteConfig, header_name: &str) -> Option<&'a str> {
    route
        .upstream
        .inject_headers
        .iter()
        .rev()
        .find(|header| header.name.trim().eq_ignore_ascii_case(header_name))
        .map(|header| header.value.trim())
        .filter(|value| !value.is_empty())
}

fn upstream_key_header_names() -> &'static [&'static str] {
    &["authorization", "x-api-key"]
}

fn parse_bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    Some(token)
}

fn fingerprint(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{ConcurrencyController, ConcurrencyError};
    use crate::config::{
        ApiKeyConcurrencyConfig, ApiKeyConfig, ApiKeysGlobalConfig, AppConfig, ConcurrencyConfig,
        GatewayAuthConfig, HeaderInjection, RouteConfig, TokenSourceConfig, UpstreamConfig,
    };

    #[test]
    fn downstream_limit_rejects_when_full() {
        let controller = ConcurrencyController::new(&config_with_limits(
            Some(1),
            Some(1),
            None,
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "key-a".to_string(),
            }],
        ))
        .expect("controller should exist");

        let first = controller
            .acquire_downstream()
            .expect("first downstream permit should succeed")
            .expect("permit should exist");
        let second = controller.acquire_downstream();

        assert!(matches!(
            second,
            Err(ConcurrencyError::DownstreamLimitExceeded)
        ));
        drop(first);
    }

    #[tokio::test]
    async fn upstream_limit_is_per_key() {
        let mut config = config_with_limits(
            None,
            Some(1),
            None,
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "key-a".to_string(),
            }],
        );
        config.routes.push(RouteConfig {
            id: "anthropic".to_string(),
            prefix: "/claude".to_string(),
            upstream: UpstreamConfig {
                base_url: "https://api.anthropic.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 10_000,
                request_timeout_ms: 60_000,
                inject_headers: vec![HeaderInjection {
                    name: "x-api-key".to_string(),
                    value: "key-b".to_string(),
                }],
                remove_headers: Vec::new(),
                forward_xff: false,
                proxy: None,
                upstream_key_max_inflight: None,
                user_agent: None,
            },
        });
        let controller = ConcurrencyController::new(&config).expect("controller should exist");
        let route_a = config.routes.remove(0);
        let route_b = config.routes.remove(0);

        let first = controller
            .acquire_upstream(&route_a)
            .await
            .expect("first key-a permit should succeed")
            .expect("permit should exist");

        let second_same_key = controller.acquire_upstream(&route_a).await;
        assert!(matches!(
            second_same_key,
            Err(ConcurrencyError::UpstreamLimitExceeded)
        ));

        let second_different_key = controller
            .acquire_upstream(&route_b)
            .await
            .expect("key-b permit should succeed")
            .expect("permit should exist");

        drop(first);
        drop(second_different_key);
    }

    #[tokio::test]
    async fn route_override_limit_works_without_global_upstream_limit() {
        let mut config = config_with_limits(
            None,
            None,
            Some(1),
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "route-key".to_string(),
            }],
        );
        let controller = ConcurrencyController::new(&config).expect("controller should exist");
        let route = config.routes.remove(0);

        let first = controller
            .acquire_upstream(&route)
            .await
            .expect("first permit should succeed")
            .expect("permit should exist");
        let second = controller.acquire_upstream(&route).await;
        assert!(matches!(
            second,
            Err(ConcurrencyError::UpstreamLimitExceeded)
        ));
        drop(first);
    }

    #[tokio::test]
    async fn api_key_level_downstream_limit() {
        let mut config = config_with_api_key_limits(
            None,
            None,
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "upstream-key".to_string(),
            }],
            vec![(
                "api-key-1",
                ApiKeyConcurrencyConfig {
                    downstream_max_inflight: Some(1),
                    upstream_per_key_max_inflight: None,
                },
            )],
        );

        let controller = ConcurrencyController::new(&config).expect("controller should exist");

        // api-key-1 有 1 个并发限制
        let first = controller
            .acquire_downstream_for_key("api-key-1")
            .await
            .expect("first permit should succeed")
            .expect("permit should exist");

        let second = controller.acquire_downstream_for_key("api-key-1").await;
        assert!(matches!(
            second,
            Err(ConcurrencyError::DownstreamLimitExceeded)
        ));

        // 其他 key 不受限制
        let other = controller.acquire_downstream_for_key("other-key").await;
        assert!(other.is_ok());

        drop(first);
    }

    #[tokio::test]
    async fn api_key_level_upstream_limit() {
        let mut config = config_with_api_key_limits(
            None,
            None,
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "upstream-key".to_string(),
            }],
            vec![(
                "api-key-1",
                ApiKeyConcurrencyConfig {
                    downstream_max_inflight: None,
                    upstream_per_key_max_inflight: Some(1),
                },
            )],
        );

        let controller = ConcurrencyController::new(&config).expect("controller should exist");
        let route = config.routes.remove(0);

        // api-key-1 有 1 个上游并发限制
        let first = controller
            .acquire_upstream_for_key("api-key-1", &route)
            .await
            .expect("first permit should succeed")
            .expect("permit should exist");

        let second = controller
            .acquire_upstream_for_key("api-key-1", &route)
            .await;
        assert!(matches!(second, Err(ConcurrencyError::UpstreamLimitExceeded)));

        drop(first);
    }

    #[test]
    fn config_resolution_priority() {
        let config = config_with_api_key_limits(
            Some(10), // 全局下游限制
            Some(10), // 全局上游限制
            vec![HeaderInjection {
                name: "x-api-key".to_string(),
                value: "upstream-key".to_string(),
            }],
            vec![(
                "api-key-1",
                ApiKeyConcurrencyConfig {
                    downstream_max_inflight: Some(5),
                    upstream_per_key_max_inflight: Some(5),
                },
            )],
        );

        let controller = ConcurrencyController::new(&config).expect("controller should exist");
        let route = config.routes.first().unwrap();

        // 测试 API Key 级别配置优先
        let resolved = controller.resolve_config(Some("api-key-1"), route);
        assert_eq!(resolved.downstream_limit, Some(5));
        assert_eq!(resolved.upstream_limit, Some(5));

        // 测试其他 key 使用全局配置
        let resolved = controller.resolve_config(Some("other-key"), route);
        assert_eq!(resolved.downstream_limit, Some(10));
        assert_eq!(resolved.upstream_limit, Some(10));
    }

    fn config_with_limits(
        downstream_limit: Option<usize>,
        upstream_limit: Option<usize>,
        route_upstream_limit: Option<usize>,
        inject_headers: Vec<HeaderInjection>,
    ) -> AppConfig {
        AppConfig {
            listen: "127.0.0.1:8080".to_string(),
            gateway_auth: GatewayAuthConfig {
                api_keys: vec!["gw_token".to_string()],
                token_sources: vec![TokenSourceConfig::AuthorizationBearer],
            },
            routes: vec![RouteConfig {
                id: "openai".to_string(),
                prefix: "/openai".to_string(),
                upstream: UpstreamConfig {
                    base_url: "https://api.openai.com".to_string(),
                    strip_prefix: true,
                    connect_timeout_ms: 10_000,
                    request_timeout_ms: 60_000,
                    inject_headers,
                    remove_headers: Vec::new(),
                    forward_xff: false,
                    proxy: None,
                    upstream_key_max_inflight: route_upstream_limit,
                    user_agent: None,
                },
            }],
            api_keys: None,
            inbound_tls: None,
            cors: None,
            rate_limit: None,
            concurrency: Some(ConcurrencyConfig {
                downstream_max_inflight: downstream_limit,
                upstream_per_key_max_inflight: upstream_limit,
            }),
            observability: None,
            admin: None,
        }
    }

    fn config_with_api_key_limits(
        downstream_limit: Option<usize>,
        upstream_limit: Option<usize>,
        inject_headers: Vec<HeaderInjection>,
        api_key_limits: Vec<(&str, ApiKeyConcurrencyConfig)>,
    ) -> AppConfig {
        let api_key_configs: Vec<ApiKeyConfig> = api_key_limits
            .into_iter()
            .map(|(key, concurrency)| ApiKeyConfig {
                id: format!("ak-{key}"),
                route_id: None,
                key: key.to_string(),
                enabled: true,
                remark: String::new(),
                rate_limit: None,
                concurrency: Some(concurrency),
                ban_rules: Vec::new(),
                ban_status: None,
            })
            .collect();

        AppConfig {
            listen: "127.0.0.1:8080".to_string(),
            gateway_auth: GatewayAuthConfig {
                api_keys: vec!["gw_token".to_string()],
                token_sources: vec![TokenSourceConfig::AuthorizationBearer],
            },
            routes: vec![RouteConfig {
                id: "openai".to_string(),
                prefix: "/openai".to_string(),
                upstream: UpstreamConfig {
                    base_url: "https://api.openai.com".to_string(),
                    strip_prefix: true,
                    connect_timeout_ms: 10_000,
                    request_timeout_ms: 60_000,
                    inject_headers,
                    remove_headers: Vec::new(),
                    forward_xff: false,
                    proxy: None,
                    upstream_key_max_inflight: None,
                    user_agent: None,
                },
            }],
            api_keys: Some(ApiKeysGlobalConfig {
                keys: api_key_configs,
                ban_rules: vec![],
                sqlite: None,
            }),
            inbound_tls: None,
            cors: None,
            rate_limit: None,
            concurrency: Some(ConcurrencyConfig {
                downstream_max_inflight: downstream_limit,
                upstream_per_key_max_inflight: upstream_limit,
            }),
            observability: None,
            admin: None,
        }
    }
}

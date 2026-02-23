use crate::config::{AppConfig, RouteConfig};
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

pub struct ConcurrencyController {
    downstream_semaphore: Option<Arc<Semaphore>>,
    upstream_default_limit: Option<usize>,
    upstream_semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
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

        if downstream_limit.is_none() && upstream_default_limit.is_none() && !has_route_override {
            return None;
        }

        Some(Self {
            downstream_semaphore: downstream_limit.map(|limit| Arc::new(Semaphore::new(limit))),
            upstream_default_limit,
            upstream_semaphores: Mutex::new(HashMap::new()),
        })
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
        AppConfig, ConcurrencyConfig, GatewayAuthConfig, HeaderInjection, RouteConfig,
        TokenSourceConfig, UpstreamConfig,
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

    fn config_with_limits(
        downstream_limit: Option<usize>,
        upstream_limit: Option<usize>,
        route_upstream_limit: Option<usize>,
        inject_headers: Vec<HeaderInjection>,
    ) -> AppConfig {
        AppConfig {
            listen: "127.0.0.1:8080".to_string(),
            gateway_auth: GatewayAuthConfig {
                tokens: vec!["gw_token".to_string()],
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
                },
            }],
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

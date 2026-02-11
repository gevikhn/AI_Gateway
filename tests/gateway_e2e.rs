use ai_gw_lite::config::{
    AppConfig, ConcurrencyConfig, CorsConfig, GatewayAuthConfig, HeaderInjection, LogFormat,
    LoggingConfig, MetricsConfig, ObservabilityConfig, ProxyProtocol, RateLimitConfig, RouteConfig,
    TokenSourceConfig, TracingConfig, UpstreamConfig, UpstreamProxyConfig,
};
use ai_gw_lite::observability;
use ai_gw_lite::server::build_app;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode};
use axum::routing::{any, get, post};
use futures_util::stream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct UpstreamCapture {
    authorization: Arc<Mutex<Option<String>>>,
    x_forwarded_for: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Default)]
struct ProxyCapture {
    proxy_authorization: Arc<Mutex<Option<String>>>,
    target_uri: Arc<Mutex<Option<String>>>,
}

#[tokio::test]
async fn proxy_rewrites_and_injects_headers() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(capture.clone());
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 2_000);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway_addr}/openai/v1/echo?mode=test"))
        .header("authorization", "Bearer gw_token")
        .header("x-forwarded-for", "1.2.3.4")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        capture
            .authorization
            .lock()
            .expect("lock should succeed")
            .as_deref(),
        Some("Bearer injected-upstream-token")
    );
    assert!(
        capture
            .x_forwarded_for
            .lock()
            .expect("lock should succeed")
            .is_none()
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_passes_sse_response() {
    let upstream = Router::new().route("/v1/sse", get(upstream_sse));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 2_000);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/sse"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let bytes = response.bytes().await.expect("body should be readable");
    let body = String::from_utf8(bytes.to_vec()).expect("body should be utf8");
    assert!(body.contains("data: hello"));
    assert!(body.contains("data: world"));

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn sse_is_not_cut_by_request_timeout() {
    let upstream = Router::new().route("/v1/sse-slow", get(upstream_sse_slow));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 80);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/sse-slow"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let body_result = tokio::time::timeout(Duration::from_millis(300), response.text()).await;
    assert!(
        body_result.is_ok(),
        "sse response should continue despite small request timeout"
    );
    let body = body_result
        .expect("sse body future should complete")
        .expect("sse body should be readable");
    assert!(body.contains("data: delayed"));

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn timeout_is_mapped_to_504() {
    let upstream = Router::new().route("/v1/slow", get(upstream_slow));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 80);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/slow"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        response
            .text()
            .await
            .expect("body should be readable")
            .as_str(),
        r#"{"error":"upstream_timeout"}"#
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn rate_limit_rejects_excess_downstream_requests() {
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(UpstreamCapture::default());
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let mut config = gateway_config(upstream_addr.to_string(), 2_000);
    config.rate_limit = Some(RateLimitConfig { per_minute: 1 });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let client = reqwest::Client::new();
    let first = client
        .post(format!("http://{gateway_addr}/openai/v1/echo"))
        .header("authorization", "Bearer gw_token")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(first.status(), StatusCode::OK);

    let second = client
        .post(format!("http://{gateway_addr}/openai/v1/echo"))
        .header("authorization", "Bearer gw_token")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        second.headers().contains_key("retry-after"),
        "rate limited response should include Retry-After"
    );
    assert_eq!(
        second
            .text()
            .await
            .expect("body should be readable")
            .as_str(),
        r#"{"error":"rate_limited"}"#
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn downstream_concurrency_limit_rejects_when_inflight_is_full() {
    let upstream = Router::new().route("/v1/stall-body", get(upstream_stall_body));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let mut config = gateway_config(upstream_addr.to_string(), 2_000);
    config.concurrency = Some(ConcurrencyConfig {
        downstream_max_inflight: Some(1),
        upstream_per_key_max_inflight: None,
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let client = reqwest::Client::new();
    let first_response = client
        .get(format!("http://{gateway_addr}/openai/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("first request should succeed");
    assert_eq!(first_response.status(), StatusCode::OK);

    let second_response = client
        .get(format!("http://{gateway_addr}/openai/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("second request should succeed");
    assert_eq!(second_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        second_response
            .text()
            .await
            .expect("body should be readable")
            .as_str(),
        r#"{"error":"downstream_concurrency_exceeded"}"#
    );

    drop(first_response);
    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn upstream_concurrency_limit_is_scoped_by_upstream_key() {
    let upstream = Router::new().route("/v1/stall-body", get(upstream_stall_body));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let mut config = gateway_config(upstream_addr.to_string(), 2_000);
    config.routes[0].id = "openai-a".to_string();
    config.routes[0].prefix = "/openai-a".to_string();
    config.routes[0].upstream.inject_headers = vec![HeaderInjection {
        name: "x-api-key".to_string(),
        value: "key-a".to_string(),
    }];

    let mut route_b = config.routes[0].clone();
    route_b.id = "openai-b".to_string();
    route_b.prefix = "/openai-b".to_string();
    route_b.upstream.inject_headers = vec![HeaderInjection {
        name: "x-api-key".to_string(),
        value: "key-b".to_string(),
    }];
    config.routes.push(route_b);

    config.concurrency = Some(ConcurrencyConfig {
        downstream_max_inflight: None,
        upstream_per_key_max_inflight: Some(1),
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let client = reqwest::Client::new();
    let first_response = client
        .get(format!("http://{gateway_addr}/openai-a/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("first request should succeed");
    assert_eq!(first_response.status(), StatusCode::OK);

    let same_key_response = client
        .get(format!("http://{gateway_addr}/openai-a/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("same-key request should succeed");
    assert_eq!(same_key_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        same_key_response
            .text()
            .await
            .expect("body should be readable")
            .as_str(),
        r#"{"error":"upstream_concurrency_exceeded"}"#
    );

    let different_key_response = client
        .get(format!("http://{gateway_addr}/openai-b/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("different-key request should succeed");
    assert_eq!(different_key_response.status(), StatusCode::OK);

    drop(first_response);
    drop(different_key_response);
    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn connect_error_is_mapped_to_502() {
    let unused = unused_local_addr();
    let config = gateway_config(unused.to_string(), 2_000);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/models"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    gateway_handle.abort();
}

#[tokio::test]
async fn stalled_non_sse_body_is_bounded_by_request_timeout() {
    let upstream = Router::new().route("/v1/stall-body", get(upstream_stall_body));
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 80);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/stall-body"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);

    let read_result = tokio::time::timeout(Duration::from_millis(200), response.bytes()).await;
    assert!(
        read_result.is_ok(),
        "non-sse body should not hang past request timeout budget"
    );
    assert!(
        read_result.expect("body future should complete").is_err(),
        "stalled non-sse body should be interrupted by gateway timeout control"
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn cors_preflight_returns_allow_headers_without_auth() {
    let unused = unused_local_addr();
    let mut config = gateway_config(unused.to_string(), 2_000);
    config.cors = Some(CorsConfig {
        enabled: true,
        allow_origins: vec!["https://fy.ciallo.fans".to_string()],
        allow_headers: vec!["authorization".to_string()],
        allow_methods: vec!["POST".to_string()],
        expose_headers: vec![],
    });

    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{gateway_addr}/openai/v1/chat/completions"),
        )
        .header("origin", "https://fy.ciallo.fans")
        .header("access-control-request-method", "POST")
        .header(
            "access-control-request-headers",
            "authorization,content-type",
        )
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("https://fy.ciallo.fans")
    );
    let allow_methods = response
        .headers()
        .get("access-control-allow-methods")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_uppercase();
    assert!(allow_methods.contains("POST"));
    let allow_headers = response
        .headers()
        .get("access-control-allow-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(allow_headers.contains("authorization"));
    assert!(allow_headers.contains("content-type"));

    gateway_handle.abort();
}

#[tokio::test]
async fn cors_allows_origin_without_scheme_config() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(capture.clone());
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let mut config = gateway_config(upstream_addr.to_string(), 2_000);
    config.cors = Some(CorsConfig {
        enabled: true,
        allow_origins: vec!["fy.ciallo.fans".to_string()],
        allow_headers: vec![],
        allow_methods: vec![],
        expose_headers: vec!["x-request-id".to_string()],
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway_addr}/openai/v1/echo"))
        .header("authorization", "Bearer gw_token")
        .header("origin", "https://fy.ciallo.fans")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("https://fy.ciallo.fans")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-expose-headers")
            .and_then(|value| value.to_str().ok()),
        Some("x-request-id")
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn http_proxy_with_auth_is_used_for_upstream() {
    let proxy_capture = ProxyCapture::default();
    let proxy_server = Router::new()
        .fallback(any(proxy_observer))
        .with_state(proxy_capture.clone());
    let (proxy_addr, proxy_handle) = spawn_router(proxy_server).await;

    let mut config = gateway_config("proxy-target.local".to_string(), 2_000);
    config.routes[0].upstream.base_url = "http://proxy-target.local".to_string();
    config.routes[0].upstream.proxy = Some(UpstreamProxyConfig {
        protocol: ProxyProtocol::Http,
        address: proxy_addr.to_string(),
        username: Some("proxy-user".to_string()),
        password: Some("proxy-pass".to_string()),
    });

    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/openai/v1/models?mode=proxy"))
        .header("authorization", "Bearer gw_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("body should be readable"),
        r#"{"via":"proxy"}"#
    );
    assert_eq!(
        proxy_capture
            .proxy_authorization
            .lock()
            .expect("lock should succeed")
            .as_deref(),
        Some("Basic cHJveHktdXNlcjpwcm94eS1wYXNz")
    );
    assert_eq!(
        proxy_capture
            .target_uri
            .lock()
            .expect("lock should succeed")
            .as_deref(),
        Some("http://proxy-target.local/v1/models?mode=proxy")
    );

    gateway_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn proxy_response_preserves_request_id() {
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(UpstreamCapture::default());
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 2_000);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway_addr}/openai/v1/echo"))
        .header("authorization", "Bearer gw_token")
        .header("x-request-id", "req-123")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("req-123")
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_response_generates_request_id_when_missing() {
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(UpstreamCapture::default());
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let config = gateway_config(upstream_addr.to_string(), 2_000);
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{gateway_addr}/openai/v1/echo"))
        .header("authorization", "Bearer gw_token")
        .body("hello")
        .send()
        .await
        .expect("request should succeed");

    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("x-request-id should exist");
    assert!(!request_id.is_empty());

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn metrics_endpoint_requires_dedicated_token() {
    let mut config = gateway_config(unused_local_addr().to_string(), 2_000);
    config.observability = Some(ObservabilityConfig {
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            to_stdout: true,
            file: None,
        },
        metrics: MetricsConfig {
            enabled: true,
            path: "/metrics".to_string(),
            token: "metrics_token".to_string(),
        },
        tracing: TracingConfig {
            enabled: false,
            sample_ratio: 0.05,
            otlp: None,
        },
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let unauthorized = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/metrics"))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let wrong_token = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/metrics"))
        .header("authorization", "Bearer wrong_token")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(wrong_token.status(), StatusCode::UNAUTHORIZED);

    gateway_handle.abort();
}

#[tokio::test]
async fn metrics_endpoint_exposes_prometheus_metrics_with_valid_token() {
    let mut config = gateway_config(unused_local_addr().to_string(), 2_000);
    config.observability = Some(ObservabilityConfig {
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            to_stdout: true,
            file: None,
        },
        metrics: MetricsConfig {
            enabled: true,
            path: "/metrics".to_string(),
            token: "metrics_token".to_string(),
        },
        tracing: TracingConfig {
            enabled: false,
            sample_ratio: 0.05,
            otlp: None,
        },
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/metrics"))
        .header("authorization", "Bearer metrics_token")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4")
    );
    assert!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .is_some(),
        "metrics response should include request id"
    );
    let body = response
        .text()
        .await
        .expect("metrics body should be readable");
    assert!(body.contains("gateway_requests_total"));
    assert!(body.contains("gateway_inflight_requests"));

    gateway_handle.abort();
}

#[tokio::test]
async fn metrics_summary_endpoint_requires_metrics_token() {
    let mut config = gateway_config(unused_local_addr().to_string(), 2_000);
    config.observability = Some(ObservabilityConfig {
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            to_stdout: true,
            file: None,
        },
        metrics: MetricsConfig {
            enabled: true,
            path: "/metrics".to_string(),
            token: "metrics_token".to_string(),
        },
        tracing: TracingConfig {
            enabled: false,
            sample_ratio: 0.05,
            otlp: None,
        },
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let unauthorized = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/metrics/summary"))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    gateway_handle.abort();
}

#[tokio::test]
async fn metrics_summary_endpoint_reports_route_and_token_windows() {
    let capture = UpstreamCapture::default();
    let upstream = Router::new()
        .route("/v1/echo", post(upstream_echo))
        .with_state(capture);
    let (upstream_addr, upstream_handle) = spawn_router(upstream).await;

    let mut config = gateway_config(upstream_addr.to_string(), 2_000);
    config.observability = Some(ObservabilityConfig {
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            to_stdout: true,
            file: None,
        },
        metrics: MetricsConfig {
            enabled: true,
            path: "/metrics".to_string(),
            token: "metrics_token".to_string(),
        },
        tracing: TracingConfig {
            enabled: false,
            sample_ratio: 0.05,
            otlp: None,
        },
    });

    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;
    let client = reqwest::Client::new();

    for _ in 0..2 {
        let response = client
            .post(format!("http://{gateway_addr}/openai/v1/echo"))
            .header("authorization", "Bearer gw_token")
            .body("hello")
            .send()
            .await
            .expect("proxy request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let summary = client
        .get(format!("http://{gateway_addr}/metrics/summary"))
        .header("authorization", "Bearer metrics_token")
        .send()
        .await
        .expect("summary request should succeed");
    assert_eq!(summary.status(), StatusCode::OK);
    assert!(
        summary
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .is_some(),
        "summary response should include request id"
    );
    let body = summary
        .text()
        .await
        .expect("summary body should be readable");
    assert!(body.contains("\"route_id\":\"openai\""));
    assert!(body.contains("\"requests_1h\":2"));
    assert!(body.contains("\"requests_24h\":2"));
    let token_label = observability::token_label("gw_token");
    assert!(body.contains(format!("\"token\":\"{token_label}\"").as_str()));

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn metrics_ui_endpoint_serves_html_dashboard() {
    let mut config = gateway_config(unused_local_addr().to_string(), 2_000);
    config.observability = Some(ObservabilityConfig {
        logging: LoggingConfig {
            level: "info".to_string(),
            format: LogFormat::Json,
            to_stdout: true,
            file: None,
        },
        metrics: MetricsConfig {
            enabled: true,
            path: "/metrics".to_string(),
            token: "metrics_token".to_string(),
        },
        tracing: TracingConfig {
            enabled: false,
            sample_ratio: 0.05,
            otlp: None,
        },
    });
    let app = build_app(Arc::new(config)).expect("gateway app should build");
    let (gateway_addr, gateway_handle) = spawn_router(app).await;

    let response = reqwest::Client::new()
        .get(format!("http://{gateway_addr}/metrics/ui"))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    let body = response.text().await.expect("body should be readable");
    assert!(body.contains("Gateway Observability"));
    assert!(body.contains("/metrics/summary"));

    gateway_handle.abort();
}

async fn spawn_router(router: Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("server should run");
    });

    (addr, handle)
}

fn unused_local_addr() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind should succeed");
    let addr = listener
        .local_addr()
        .expect("local addr should be available");
    drop(listener);
    addr
}

fn gateway_config(upstream_addr: String, request_timeout_ms: u64) -> AppConfig {
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
                base_url: format!("http://{upstream_addr}"),
                strip_prefix: true,
                connect_timeout_ms: 1_000,
                request_timeout_ms,
                inject_headers: vec![HeaderInjection {
                    name: "authorization".to_string(),
                    value: "Bearer injected-upstream-token".to_string(),
                }],
                remove_headers: vec![
                    "authorization".to_string(),
                    "x-forwarded-for".to_string(),
                    "forwarded".to_string(),
                    "cf-connecting-ip".to_string(),
                    "true-client-ip".to_string(),
                ],
                forward_xff: false,
                proxy: None,
                upstream_key_max_inflight: None,
            },
        }],
        inbound_tls: None,
        cors: None,
        rate_limit: None,
        concurrency: None,
        observability: None,
    }
}

async fn upstream_echo(
    State(capture): State<UpstreamCapture>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let xff = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    *capture.authorization.lock().expect("lock should succeed") = auth;
    *capture.x_forwarded_for.lock().expect("lock should succeed") = xff;

    if body == Bytes::from_static(b"hello") {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    }
}

async fn upstream_sse() -> Response<Body> {
    let events = stream::iter(vec![
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: hello\n\n")),
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: world\n\n")),
    ]);

    let mut response = Response::new(Body::from_stream(events));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
}

async fn upstream_slow() -> StatusCode {
    tokio::time::sleep(Duration::from_millis(120)).await;
    StatusCode::OK
}

async fn upstream_stall_body() -> Response<Body> {
    let body = stream::once(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"late-body"))
    });

    let mut response = Response::new(Body::from_stream(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
}

async fn upstream_sse_slow() -> Response<Body> {
    let events = stream::once(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: delayed\n\n"))
    });

    let mut response = Response::new(Body::from_stream(events));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
}

async fn proxy_observer(
    State(capture): State<ProxyCapture>,
    request: Request<Body>,
) -> Response<Body> {
    let proxy_auth = request
        .headers()
        .get("proxy-authorization")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    *capture
        .proxy_authorization
        .lock()
        .expect("lock should succeed") = proxy_auth;
    *capture.target_uri.lock().expect("lock should succeed") = Some(request.uri().to_string());

    let mut response = Response::new(Body::from(r#"{"via":"proxy"}"#));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

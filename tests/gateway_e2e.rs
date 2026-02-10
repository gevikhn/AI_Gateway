use ai_gw_lite::config::{
    AppConfig, GatewayAuthConfig, HeaderInjection, RouteConfig, TokenSourceConfig, UpstreamConfig,
};
use ai_gw_lite::server::build_app;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};
use axum::routing::{get, post};
use futures_util::stream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct UpstreamCapture {
    authorization: Arc<Mutex<Option<String>>>,
    x_forwarded_for: Arc<Mutex<Option<String>>>,
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

    let config = gateway_config(upstream_addr.to_string(), 20);
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

    let config = gateway_config(upstream_addr.to_string(), 20);
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

    let config = gateway_config(upstream_addr.to_string(), 20);
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
            },
        }],
        cors: None,
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

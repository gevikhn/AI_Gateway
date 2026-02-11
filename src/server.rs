use crate::auth;
use crate::concurrency::ConcurrencyController;
use crate::config::{
    AppConfig, CorsConfig, ProxyProtocol, RouteConfig, UpstreamConfig, UpstreamProxyConfig,
};
use crate::proxy;
use crate::ratelimit::{RateLimitDecision, RateLimiter};
use crate::tls;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use axum::routing::{any, get};
use axum::{Router, response::IntoResponse};
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::header::CONTENT_TYPE;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub upstream_clients: Arc<HashMap<String, reqwest::Client>>,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub concurrency: Option<Arc<ConcurrencyController>>,
}

pub fn build_app(config: Arc<AppConfig>) -> Result<Router, String> {
    let upstream_clients = Arc::new(build_upstream_clients(&config)?);
    let rate_limiter = config
        .rate_limit
        .as_ref()
        .map(|rate_limit| Arc::new(RateLimiter::new(rate_limit.per_minute)));
    let concurrency = ConcurrencyController::new(&config).map(Arc::new);
    Ok(Router::new()
        .route("/healthz", get(healthz_handler))
        .fallback(any(proxy_handler))
        .with_state(AppState {
            config,
            upstream_clients,
            rate_limiter,
            concurrency,
        }))
}

pub async fn run_server(config: Arc<AppConfig>) -> Result<(), String> {
    let listen_addr: SocketAddr = config
        .listen
        .parse()
        .map_err(|err| format!("invalid listen address `{}`: {err}", config.listen))?;
    let app = build_app(config.clone())?;

    if let Some(tls_config) = &config.inbound_tls {
        install_rustls_crypto_provider();
        let (tls_paths, _) = tls::resolve_tls_paths(tls_config, listen_addr)?;
        let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            &tls_paths.cert_path,
            &tls_paths.key_path,
        )
        .await
        .map_err(|err| {
            format!(
                "failed to load inbound tls cert/key (`{}` / `{}`): {err}",
                tls_paths.cert_path.display(),
                tls_paths.key_path.display()
            )
        })?;

        axum_server::bind_rustls(listen_addr, rustls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|err| format!("server error: {err}"))
    } else {
        let listener = tokio::net::TcpListener::bind(listen_addr)
            .await
            .map_err(|err| format!("failed to bind `{listen_addr}`: {err}"))?;

        axum::serve(listener, app)
            .await
            .map_err(|err| format!("server error: {err}"))
    }
}

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

async fn healthz_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
}

async fn proxy_handler(State(state): State<AppState>, request: Request<Body>) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(ToString::to_string);
    let request_origin = extract_origin(request.headers());
    let cors_config = state.config.cors.as_ref().filter(|cors| cors.enabled);

    let Some(route) = proxy::match_route(&path, &state.config.routes) else {
        return finalize_response_with_cors(
            json_error(StatusCode::NOT_FOUND, "route_not_found"),
            cors_config,
            request_origin.as_deref(),
        );
    };

    if let Some(cors) = cors_config
        && is_cors_preflight(&method, request.headers())
    {
        return build_preflight_response(cors, request_origin.as_deref(), request.headers());
    }

    let Some(token) = auth::extract_authorized_token(request.headers(), &state.config.gateway_auth)
    else {
        return finalize_response_with_cors(
            json_error(StatusCode::UNAUTHORIZED, "unauthorized"),
            cors_config,
            request_origin.as_deref(),
        );
    };

    if let Some(rate_limiter) = &state.rate_limiter {
        match rate_limiter.check(&token, &route.id) {
            RateLimitDecision::Allowed => {}
            RateLimitDecision::Rejected { retry_after_secs } => {
                let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
                set_header(
                    response.headers_mut(),
                    "retry-after",
                    &retry_after_secs.to_string(),
                );
                return finalize_response_with_cors(
                    response,
                    cors_config,
                    request_origin.as_deref(),
                );
            }
        }
    }

    let downstream_permit = if let Some(concurrency) = &state.concurrency {
        match concurrency.acquire_downstream() {
            Ok(permit) => permit,
            Err(_) => {
                return finalize_response_with_cors(
                    json_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "downstream_concurrency_exceeded",
                    ),
                    cors_config,
                    request_origin.as_deref(),
                );
            }
        }
    } else {
        None
    };

    let Some(upstream_url) = proxy::build_upstream_url_for_route(route, &path, query.as_deref())
    else {
        return finalize_response_with_cors(
            json_error(StatusCode::BAD_REQUEST, "invalid_upstream_path"),
            cors_config,
            request_origin.as_deref(),
        );
    };

    let upstream_headers = match proxy::prepare_upstream_headers(request.headers(), &route.upstream)
    {
        Ok(headers) => headers,
        Err(_) => {
            return finalize_response_with_cors(
                json_error(StatusCode::BAD_GATEWAY, "upstream_header_error"),
                cors_config,
                request_origin.as_deref(),
            );
        }
    };

    let Some(upstream_client) = state.upstream_clients.get(&route.id) else {
        return finalize_response_with_cors(
            json_error(StatusCode::BAD_GATEWAY, "upstream_client_not_found"),
            cors_config,
            request_origin.as_deref(),
        );
    };

    let upstream_permit = if let Some(concurrency) = &state.concurrency {
        match concurrency.acquire_upstream(route).await {
            Ok(permit) => permit,
            Err(_) => {
                return finalize_response_with_cors(
                    json_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "upstream_concurrency_exceeded",
                    ),
                    cors_config,
                    request_origin.as_deref(),
                );
            }
        }
    } else {
        None
    };

    let response_guards = ResponseGuards {
        downstream_permit,
        upstream_permit,
    };

    let response = match forward_to_upstream(
        upstream_client,
        route,
        request,
        upstream_url,
        upstream_headers,
    )
    .await
    {
        Ok(response) => attach_response_guards(response, response_guards),
        Err(error) => error_response(error),
    };
    finalize_response_with_cors(response, cors_config, request_origin.as_deref())
}

async fn forward_to_upstream(
    upstream_client: &reqwest::Client,
    route: &RouteConfig,
    request: Request<Body>,
    upstream_url: String,
    upstream_headers: http::HeaderMap,
) -> Result<Response<Body>, UpstreamError> {
    let mut upstream_request = upstream_client.request(request.method().clone(), upstream_url);

    for (name, value) in &upstream_headers {
        upstream_request = upstream_request.header(name, value);
    }

    let request_stream =
        futures_util::TryStreamExt::map_err(request.into_body().into_data_stream(), |err| {
            io::Error::other(err.to_string())
        });
    upstream_request = upstream_request.body(reqwest::Body::wrap_stream(request_stream));

    let request_timeout = Duration::from_millis(route.upstream.request_timeout_ms);
    let deadline = tokio::time::Instant::now() + request_timeout;
    let upstream_response = match tokio::time::timeout_at(deadline, upstream_request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => return Err(UpstreamError::Request(err)),
        Err(_) => return Err(UpstreamError::Timeout),
    };

    let is_sse = is_sse_response(upstream_response.headers());
    Ok(response_from_upstream(upstream_response, is_sse, deadline))
}

type ProxyBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

struct ResponseGuards {
    downstream_permit: Option<OwnedSemaphorePermit>,
    upstream_permit: Option<OwnedSemaphorePermit>,
}

impl ResponseGuards {
    fn is_empty(&self) -> bool {
        self.downstream_permit.is_none() && self.upstream_permit.is_none()
    }
}

fn response_from_upstream(
    upstream_response: reqwest::Response,
    is_sse: bool,
    deadline: tokio::time::Instant,
) -> Response<Body> {
    let status = upstream_response.status();
    let headers = proxy::sanitize_response_headers(upstream_response.headers());
    let stream: ProxyBodyStream = Box::pin(
        upstream_response
            .bytes_stream()
            .map_err(|err| io::Error::other(err.to_string())),
    );
    let stream = if is_sse {
        stream
    } else {
        enforce_response_deadline(stream, deadline)
    };

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn attach_response_guards(response: Response<Body>, guards: ResponseGuards) -> Response<Body> {
    if guards.is_empty() {
        return response;
    }

    let (parts, body) = response.into_parts();
    let stream = body
        .into_data_stream()
        .map_err(|err| io::Error::other(err.to_string()))
        .map(move |item| {
            let _ = &guards;
            item
        });
    Response::from_parts(parts, Body::from_stream(stream))
}

fn json_error(status: StatusCode, code: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(format!(r#"{{"error":"{code}"}}"#)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    response
}

fn error_response(error: UpstreamError) -> Response<Body> {
    match error {
        UpstreamError::Timeout => json_error(StatusCode::GATEWAY_TIMEOUT, "upstream_timeout"),
        UpstreamError::Request(err) => {
            if err.is_timeout() {
                json_error(StatusCode::GATEWAY_TIMEOUT, "upstream_timeout")
            } else if err.is_connect() {
                json_error(StatusCode::BAD_GATEWAY, "upstream_connect_error")
            } else {
                json_error(StatusCode::BAD_GATEWAY, "upstream_request_failed")
            }
        }
    }
}

fn finalize_response_with_cors(
    mut response: Response<Body>,
    cors: Option<&CorsConfig>,
    request_origin: Option<&str>,
) -> Response<Body> {
    if let Some(cors) = cors {
        apply_cors_response_headers(&mut response, cors, request_origin);
    }
    response
}

fn build_preflight_response(
    cors: &CorsConfig,
    request_origin: Option<&str>,
    request_headers: &HeaderMap,
) -> Response<Body> {
    let Some(allow_origin) = resolve_allow_origin(cors, request_origin) else {
        return json_error(StatusCode::FORBIDDEN, "cors_origin_not_allowed");
    };

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;

    set_header(
        response.headers_mut(),
        "access-control-allow-origin",
        &allow_origin,
    );
    set_header(
        response.headers_mut(),
        "vary",
        "Origin, Access-Control-Request-Method, Access-Control-Request-Headers",
    );

    let allow_methods = build_allow_methods(cors, request_headers);
    if !allow_methods.is_empty() {
        set_header(
            response.headers_mut(),
            "access-control-allow-methods",
            &allow_methods,
        );
    }

    let allow_headers = build_allow_headers(cors, request_headers);
    if !allow_headers.is_empty() {
        set_header(
            response.headers_mut(),
            "access-control-allow-headers",
            &allow_headers,
        );
    }

    response
}

fn apply_cors_response_headers(
    response: &mut Response<Body>,
    cors: &CorsConfig,
    request_origin: Option<&str>,
) {
    if let Some(allow_origin) = resolve_allow_origin(cors, request_origin) {
        set_header(
            response.headers_mut(),
            "access-control-allow-origin",
            &allow_origin,
        );
        set_header(response.headers_mut(), "vary", "Origin");
    }

    if !cors.expose_headers.is_empty() {
        let expose_headers = cors
            .expose_headers
            .iter()
            .map(|header| header.trim())
            .filter(|header| !header.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        if !expose_headers.is_empty() {
            set_header(
                response.headers_mut(),
                "access-control-expose-headers",
                &expose_headers,
            );
        }
    }
}

fn resolve_allow_origin(cors: &CorsConfig, request_origin: Option<&str>) -> Option<String> {
    let request_origin = request_origin?.trim();
    if request_origin.is_empty() {
        return None;
    }

    for allowed in &cors.allow_origins {
        let allowed = allowed.trim();
        if allowed.is_empty() {
            continue;
        }
        if allowed == "*" {
            return Some("*".to_string());
        }
        if allowed.eq_ignore_ascii_case(request_origin) {
            return Some(request_origin.to_string());
        }
        if !allowed.contains("://")
            && let Some(origin_host) = extract_origin_host(request_origin)
            && allowed.eq_ignore_ascii_case(&origin_host)
        {
            return Some(request_origin.to_string());
        }
    }

    None
}

fn extract_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn extract_origin_host(origin: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(origin).ok()?;
    let host = parsed.host_str()?;
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn is_cors_preflight(method: &Method, headers: &HeaderMap) -> bool {
    method == Method::OPTIONS
        && headers.contains_key("origin")
        && headers.contains_key("access-control-request-method")
}

fn build_allow_methods(cors: &CorsConfig, request_headers: &HeaderMap) -> String {
    let mut methods = cors
        .allow_methods
        .iter()
        .map(|method| method.trim().to_ascii_uppercase())
        .filter(|method| !method.is_empty())
        .collect::<Vec<_>>();

    if let Some(request_method) = request_headers
        .get("access-control-request-method")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        && !methods.iter().any(|method| method == &request_method)
    {
        methods.push(request_method);
    }

    methods.join(", ")
}

fn build_allow_headers(cors: &CorsConfig, request_headers: &HeaderMap) -> String {
    let mut headers = cors
        .allow_headers
        .iter()
        .map(|header| header.trim().to_ascii_lowercase())
        .filter(|header| !header.is_empty())
        .collect::<Vec<_>>();

    if let Some(requested_headers) = request_headers
        .get("access-control-request-headers")
        .and_then(|value| value.to_str().ok())
    {
        for header in requested_headers.split(',') {
            let header = header.trim().to_ascii_lowercase();
            if !header.is_empty() && !headers.iter().any(|existing| existing == &header) {
                headers.push(header);
            }
        }
    }

    headers.join(", ")
}

fn set_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(header_value) = http::HeaderValue::from_str(value) {
        headers.insert(http::HeaderName::from_static(name), header_value);
    }
}

enum UpstreamError {
    Timeout,
    Request(reqwest::Error),
}

fn build_upstream_clients(config: &AppConfig) -> Result<HashMap<String, reqwest::Client>, String> {
    let mut clients = HashMap::with_capacity(config.routes.len());
    for route in &config.routes {
        let client = build_upstream_client(&route.upstream).map_err(|err| {
            format!(
                "failed to build upstream client for route `{}`: {err}",
                route.id
            )
        })?;
        clients.insert(route.id.clone(), client);
    }
    Ok(clients)
}

fn build_upstream_client(upstream: &UpstreamConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(upstream.connect_timeout_ms));

    if let Some(proxy) = &upstream.proxy {
        let proxy_url = build_proxy_url(proxy)?;
        let reqwest_proxy = reqwest::Proxy::all(proxy_url.as_str())
            .map_err(|err| format!("invalid upstream.proxy config: {err}"))?;
        builder = builder.proxy(reqwest_proxy);
    }

    builder
        .build()
        .map_err(|err| format!("failed to build reqwest client: {err}"))
}

fn build_proxy_url(proxy: &UpstreamProxyConfig) -> Result<reqwest::Url, String> {
    let scheme = match proxy.protocol {
        ProxyProtocol::Http => "http",
        ProxyProtocol::Https => "https",
        ProxyProtocol::Socks => "socks5h",
    };

    let mut url = reqwest::Url::parse(&format!("{scheme}://{}", proxy.address.trim()))
        .map_err(|err| format!("invalid upstream.proxy.address: {err}"))?;

    if let (Some(username), Some(password)) = (&proxy.username, &proxy.password) {
        url.set_username(username)
            .map_err(|_| "invalid upstream.proxy.username".to_string())?;
        url.set_password(Some(password))
            .map_err(|_| "invalid upstream.proxy.password".to_string())?;
    }

    Ok(url)
}

fn is_sse_response(headers: &http::HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|media_type| media_type.trim().eq_ignore_ascii_case("text/event-stream"))
        .unwrap_or(false)
}

fn enforce_response_deadline(
    stream: ProxyBodyStream,
    deadline: tokio::time::Instant,
) -> ProxyBodyStream {
    Box::pin(futures_util::stream::unfold(
        stream,
        move |mut stream| async move {
            match tokio::time::timeout_at(deadline, stream.as_mut().try_next()).await {
                Ok(Ok(Some(chunk))) => Some((Ok(chunk), stream)),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some((Err(error), stream)),
                Err(_) => Some((
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "upstream response exceeded request timeout",
                    )),
                    stream,
                )),
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{build_app, build_proxy_url, build_upstream_clients};
    use crate::config::{
        AppConfig, GatewayAuthConfig, ProxyProtocol, RouteConfig, TokenSourceConfig,
        UpstreamConfig, UpstreamProxyConfig,
    };
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use std::sync::Arc;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = build_app(Arc::new(test_config())).expect("app should build");
        let request = Request::builder()
            .method(Method::GET)
            .uri("/healthz")
            .body(Body::empty())
            .expect("request should build");

        let response = app.oneshot(request).await.expect("request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = build_app(Arc::new(test_config())).expect("app should build");
        let request = Request::builder()
            .method(Method::GET)
            .uri("/unknown")
            .header("authorization", "Bearer gw_token")
            .body(Body::empty())
            .expect("request should build");

        let response = app.oneshot(request).await.expect("request should succeed");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        assert_eq!(&body[..], br#"{"error":"route_not_found"}"#);
    }

    #[tokio::test]
    async fn auth_failure_returns_401() {
        let app = build_app(Arc::new(test_config())).expect("app should build");
        let request = Request::builder()
            .method(Method::GET)
            .uri("/openai/v1/models")
            .body(Body::empty())
            .expect("request should build");

        let response = app.oneshot(request).await.expect("request should succeed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn build_upstream_clients_once_per_route() {
        let mut config = test_config();
        config.routes.push(RouteConfig {
            id: "anthropic".to_string(),
            prefix: "/claude".to_string(),
            upstream: UpstreamConfig {
                base_url: "https://api.anthropic.com".to_string(),
                strip_prefix: true,
                connect_timeout_ms: 10_000,
                request_timeout_ms: 60_000,
                inject_headers: Vec::new(),
                remove_headers: Vec::new(),
                forward_xff: false,
                proxy: Some(UpstreamProxyConfig {
                    protocol: ProxyProtocol::Https,
                    address: "127.0.0.1:8443".to_string(),
                    username: None,
                    password: None,
                }),
                upstream_key_max_inflight: None,
            },
        });

        let clients = build_upstream_clients(&config).expect("clients should build");
        assert_eq!(clients.len(), 2);
        assert!(clients.contains_key("openai"));
        assert!(clients.contains_key("anthropic"));
    }

    #[test]
    fn build_proxy_url_uses_expected_scheme_and_auth() {
        let proxy = UpstreamProxyConfig {
            protocol: ProxyProtocol::Socks,
            address: "127.0.0.1:1080".to_string(),
            username: Some("proxy-user".to_string()),
            password: Some("proxy-pass".to_string()),
        };

        let proxy_url = build_proxy_url(&proxy).expect("proxy url should build");
        assert_eq!(proxy_url.scheme(), "socks5h");
        assert_eq!(proxy_url.username(), "proxy-user");
        assert_eq!(proxy_url.password(), Some("proxy-pass"));
        assert_eq!(proxy_url.host_str(), Some("127.0.0.1"));
        assert_eq!(proxy_url.port_or_known_default(), Some(1080));
    }

    fn test_config() -> AppConfig {
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
                    inject_headers: Vec::new(),
                    remove_headers: Vec::new(),
                    forward_xff: false,
                    proxy: None,
                    upstream_key_max_inflight: None,
                },
            }],
            inbound_tls: None,
            cors: None,
            rate_limit: None,
            concurrency: None,
        }
    }
}

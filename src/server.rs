use crate::auth;
use crate::config::{AppConfig, ProxyProtocol, RouteConfig, UpstreamConfig, UpstreamProxyConfig};
use crate::proxy;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Request, Response, StatusCode};
use axum::routing::{any, get};
use axum::{Router, response::IntoResponse};
use futures_util::Stream;
use futures_util::TryStreamExt;
use http::header::CONTENT_TYPE;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub upstream_clients: Arc<HashMap<String, reqwest::Client>>,
}

pub fn build_app(config: Arc<AppConfig>) -> Result<Router, String> {
    let upstream_clients = Arc::new(build_upstream_clients(&config)?);
    Ok(Router::new()
        .route("/healthz", get(healthz_handler))
        .fallback(any(proxy_handler))
        .with_state(AppState {
            config,
            upstream_clients,
        }))
}

pub async fn run_server(config: Arc<AppConfig>) -> Result<(), String> {
    let listen_addr: SocketAddr = config
        .listen
        .parse()
        .map_err(|err| format!("invalid listen address `{}`: {err}", config.listen))?;
    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .map_err(|err| format!("failed to bind `{listen_addr}`: {err}"))?;

    let app = build_app(config)?;

    axum::serve(listener, app)
        .await
        .map_err(|err| format!("server error: {err}"))
}

async fn healthz_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json")],
        r#"{"status":"ok"}"#,
    )
}

async fn proxy_handler(State(state): State<AppState>, request: Request<Body>) -> Response<Body> {
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(ToString::to_string);

    let Some(route) = proxy::match_route(&path, &state.config.routes) else {
        return json_error(StatusCode::NOT_FOUND, "route_not_found");
    };

    if !auth::is_authorized(request.headers(), &state.config.gateway_auth) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let Some(upstream_url) = proxy::build_upstream_url_for_route(route, &path, query.as_deref())
    else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_upstream_path");
    };

    let upstream_headers = match proxy::prepare_upstream_headers(request.headers(), &route.upstream)
    {
        Ok(headers) => headers,
        Err(_) => return json_error(StatusCode::BAD_GATEWAY, "upstream_header_error"),
    };

    let Some(upstream_client) = state.upstream_clients.get(&route.id) else {
        return json_error(StatusCode::BAD_GATEWAY, "upstream_client_not_found");
    };

    match forward_to_upstream(
        upstream_client,
        route,
        request,
        upstream_url,
        upstream_headers,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => error_response(error),
    }
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
                },
            }],
            cors: None,
        }
    }
}

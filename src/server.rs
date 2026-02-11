use crate::auth;
use crate::concurrency::ConcurrencyController;
use crate::config::{
    AppConfig, CorsConfig, ProxyProtocol, RouteConfig, UpstreamConfig, UpstreamProxyConfig,
};
use crate::observability;
use crate::proxy;
use crate::ratelimit::{RateLimitDecision, RateLimiter};
use crate::tls;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode};
use axum::routing::{any, get};
use axum::{Json, Router, response::IntoResponse};
use futures_util::{Stream, StreamExt, TryStreamExt};
use http::header::CONTENT_TYPE;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub upstream_clients: Arc<HashMap<String, reqwest::Client>>,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub concurrency: Option<Arc<ConcurrencyController>>,
    pub observability: Arc<observability::ObservabilityRuntime>,
}

pub fn build_app(config: Arc<AppConfig>) -> Result<Router, String> {
    let upstream_clients = Arc::new(build_upstream_clients(&config)?);
    let rate_limiter = config
        .rate_limit
        .as_ref()
        .map(|rate_limit| Arc::new(RateLimiter::new(rate_limit.per_minute)));
    let concurrency = ConcurrencyController::new(&config).map(Arc::new);
    let observability = Arc::new(observability::ObservabilityRuntime::from_config(
        config.observability.as_ref(),
    ));
    let mut router = Router::new().route("/healthz", get(healthz_handler));
    if let Some(metrics_path) = observability.metrics_path() {
        router = router.route(metrics_path, get(metrics_handler));
    }
    if let Some(metrics_ui_path) = observability.metrics_ui_path() {
        router = router.route(metrics_ui_path, get(metrics_ui_handler));
    }
    if let Some(metrics_summary_path) = observability.metrics_summary_path() {
        router = router.route(metrics_summary_path, get(metrics_summary_handler));
    }
    Ok(router.fallback(any(proxy_handler)).with_state(AppState {
        config,
        upstream_clients,
        rate_limiter,
        concurrency,
        observability,
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

async fn healthz_handler(headers: HeaderMap) -> impl IntoResponse {
    let request_id = observability::extract_or_generate_request_id(&headers);
    let mut response = Response::new(Body::from(r#"{"status":"ok"}"#));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    observability::insert_request_id_header(response.headers_mut(), &request_id);
    response
}

async fn metrics_handler(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    let request_id = observability::extract_or_generate_request_id(&headers);
    let mut response = if !state.observability.is_metrics_request_authorized(&headers) {
        json_error(StatusCode::UNAUTHORIZED, "unauthorized")
    } else if let Some(body) = state.observability.encode_metrics() {
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain; version=0.0.4"),
        );
        response
    } else {
        json_error(StatusCode::NOT_FOUND, "route_not_found")
    };
    observability::insert_request_id_header(response.headers_mut(), &request_id);
    response
}

async fn metrics_ui_handler(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    let request_id = observability::extract_or_generate_request_id(&headers);
    let summary_path = state
        .observability
        .metrics_summary_path()
        .unwrap_or("/metrics/summary");
    let mut response = Response::new(Body::from(metrics_dashboard_html(summary_path)));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    observability::insert_request_id_header(response.headers_mut(), &request_id);
    response
}

async fn metrics_summary_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    let request_id = observability::extract_or_generate_request_id(&headers);
    let mut response = if !state.observability.is_metrics_request_authorized(&headers) {
        json_error(StatusCode::UNAUTHORIZED, "unauthorized")
    } else if let Some(summary) = state.observability.snapshot_summary() {
        Json(summary).into_response()
    } else {
        json_error(StatusCode::NOT_FOUND, "route_not_found")
    };
    observability::insert_request_id_header(response.headers_mut(), &request_id);
    response
}

fn metrics_dashboard_html(summary_path: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>AI Gateway Observability</title>
  <style>
    :root {{
      --bg: #f5f7fb;
      --card: #ffffff;
      --text: #0f172a;
      --muted: #475569;
      --line: #dbe2ea;
      --accent: #0f766e;
      --danger: #b91c1c;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      font-family: "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
      background: radial-gradient(circle at top right, #e2f6f3, var(--bg) 55%);
      color: var(--text);
    }}
    main {{
      max-width: 1100px;
      margin: 0 auto;
      padding: 18px;
    }}
    h1 {{ margin: 0 0 8px 0; font-size: 28px; }}
    .muted {{ color: var(--muted); }}
    .toolbar {{
      margin-top: 14px;
      display: grid;
      grid-template-columns: 1fr auto auto;
      gap: 10px;
      align-items: center;
    }}
    input {{
      width: 100%;
      border: 1px solid var(--line);
      border-radius: 10px;
      padding: 10px 12px;
      font-size: 14px;
    }}
    button {{
      border: 0;
      border-radius: 10px;
      background: var(--accent);
      color: #fff;
      padding: 10px 14px;
      font-weight: 600;
      cursor: pointer;
    }}
    .status {{ margin-top: 8px; min-height: 20px; }}
    .error {{ color: var(--danger); }}
    .cards {{
      margin-top: 14px;
      display: grid;
      gap: 12px;
      grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    }}
    .card {{
      background: var(--card);
      border: 1px solid var(--line);
      border-radius: 12px;
      padding: 14px;
    }}
    .card .label {{ color: var(--muted); font-size: 13px; }}
    .card .value {{ margin-top: 6px; font-size: 26px; font-weight: 700; }}
    .panel {{
      margin-top: 14px;
      background: var(--card);
      border: 1px solid var(--line);
      border-radius: 12px;
      padding: 14px;
      overflow: auto;
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      font-size: 13px;
    }}
    th, td {{
      text-align: left;
      border-bottom: 1px solid var(--line);
      padding: 8px 6px;
      white-space: nowrap;
    }}
    th {{ color: var(--muted); font-weight: 600; }}
  </style>
</head>
<body>
  <main>
    <h1>Gateway Observability</h1>
    <div class="muted">窗口统计：最近 1 小时 / 24 小时；按 route 和 GW_TOKEN 聚合。</div>
    <div class="toolbar">
      <input id="token" placeholder="输入 metrics token（不会上传到其他服务）" />
      <button id="save">保存并刷新</button>
      <button id="refresh">立即刷新</button>
    </div>
    <div id="status" class="status muted">等待拉取数据</div>
    <section class="cards">
      <article class="card"><div class="label">总请求（1h）</div><div id="total1h" class="value">-</div></article>
      <article class="card"><div class="label">总请求（24h）</div><div id="total24h" class="value">-</div></article>
      <article class="card"><div class="label">Route 数</div><div id="routeCount" class="value">-</div></article>
      <article class="card"><div class="label">Token 数</div><div id="tokenCount" class="value">-</div></article>
    </section>
    <section class="panel">
      <h3>Route 维度</h3>
      <table id="routeTable">
        <thead><tr><th>Route</th><th>Req 1h</th><th>Req 24h</th><th>Inflight Now</th><th>Inflight Peak 1h</th><th>Inflight Peak 24h</th></tr></thead>
        <tbody></tbody>
      </table>
    </section>
    <section class="panel">
      <h3>GW_TOKEN 维度</h3>
      <table id="tokenTable">
        <thead><tr><th>Token</th><th>Req 1h</th><th>Req 24h</th></tr></thead>
        <tbody></tbody>
      </table>
    </section>
  </main>
  <script>
    const SUMMARY_PATH = {summary_path:?};
    const TOKEN_KEY = "ai_gw_metrics_token";
    const tokenInput = document.getElementById("token");
    const statusEl = document.getElementById("status");
    const total1hEl = document.getElementById("total1h");
    const total24hEl = document.getElementById("total24h");
    const routeCountEl = document.getElementById("routeCount");
    const tokenCountEl = document.getElementById("tokenCount");

    tokenInput.value = localStorage.getItem(TOKEN_KEY) || "";
    document.getElementById("save").addEventListener("click", () => {{
      localStorage.setItem(TOKEN_KEY, tokenInput.value.trim());
      loadData();
    }});
    document.getElementById("refresh").addEventListener("click", () => loadData());

    function setStatus(text, isError = false) {{
      statusEl.textContent = text;
      statusEl.className = isError ? "status error" : "status muted";
    }}

    function renderRows(tableId, rows, columns) {{
      const tbody = document.querySelector(`#${{tableId}} tbody`);
      tbody.innerHTML = "";
      if (!rows.length) {{
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = columns.length;
        td.textContent = "暂无数据";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }}
      for (const row of rows) {{
        const tr = document.createElement("tr");
        for (const key of columns) {{
          const td = document.createElement("td");
          td.textContent = String(row[key] ?? "");
          tr.appendChild(td);
        }}
        tbody.appendChild(tr);
      }}
    }}

    async function loadData() {{
      const token = tokenInput.value.trim() || localStorage.getItem(TOKEN_KEY) || "";
      if (!token) {{
        setStatus("请先填写 metrics token", true);
        return;
      }}
      try {{
        const res = await fetch(SUMMARY_PATH, {{
          headers: {{ Authorization: `Bearer ${{token}}` }},
          cache: "no-store"
        }});
        if (!res.ok) {{
          setStatus(`拉取失败: HTTP ${{res.status}}`, true);
          return;
        }}
        const data = await res.json();
        setStatus(`最近刷新: ${{new Date(data.generated_at_unix_ms).toLocaleString()}}`);
        total1hEl.textContent = String(data.total_requests_1h ?? 0);
        total24hEl.textContent = String(data.total_requests_24h ?? 0);
        routeCountEl.textContent = String((data.routes || []).length);
        tokenCountEl.textContent = String((data.tokens || []).length);
        renderRows("routeTable", data.routes || [], [
          "route_id", "requests_1h", "requests_24h", "inflight_current", "inflight_peak_1h", "inflight_peak_24h"
        ]);
        renderRows("tokenTable", data.tokens || [], ["token", "requests_1h", "requests_24h"]);
      }} catch (err) {{
        setStatus(`拉取失败: ${{err}}`, true);
      }}
    }}

    setInterval(loadData, 5000);
    loadData();
  </script>
</body>
</html>
"#
    )
}

async fn proxy_handler(State(state): State<AppState>, request: Request<Body>) -> Response<Body> {
    let request_started_at = tokio::time::Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(ToString::to_string);
    let request_origin = extract_origin(request.headers());
    let request_id = observability::extract_or_generate_request_id(request.headers());
    let cors_config = state.config.cors.as_ref().filter(|cors| cors.enabled);
    let metrics = state.observability.metrics.clone();
    let request_span = tracing::info_span!(
        "gateway_request",
        request_id = request_id.as_str(),
        method = method.as_str(),
        path = path.as_str(),
        route_id = tracing::field::Empty
    );
    let _span_entered = request_span.enter();

    let Some(route) = proxy::match_route(&path, &state.config.routes) else {
        tracing::Span::current().record("route_id", "__unmatched__");
        return finalize_observed_proxy_response(
            json_error(StatusCode::NOT_FOUND, "route_not_found"),
            cors_config,
            request_origin.as_deref(),
            request_observation(
                metrics.as_ref(),
                "__unmatched__",
                &method,
                &path,
                &request_id,
                request_started_at,
            ),
            "route_not_found",
        );
    };
    tracing::Span::current().record("route_id", route.id.as_str());

    if let Some(cors) = cors_config
        && is_cors_preflight(&method, request.headers())
    {
        return finalize_observed_proxy_response(
            build_preflight_response(cors, request_origin.as_deref(), request.headers()),
            cors_config,
            request_origin.as_deref(),
            request_observation(
                metrics.as_ref(),
                route.id.as_str(),
                &method,
                &path,
                &request_id,
                request_started_at,
            ),
            "preflight",
        );
    }

    let Some(token) = auth::extract_authorized_token(request.headers(), &state.config.gateway_auth)
    else {
        return finalize_observed_proxy_response(
            json_error(StatusCode::UNAUTHORIZED, "unauthorized"),
            cors_config,
            request_origin.as_deref(),
            request_observation(
                metrics.as_ref(),
                route.id.as_str(),
                &method,
                &path,
                &request_id,
                request_started_at,
            ),
            "unauthorized",
        );
    };
    let token_label = observability::token_label(&token);

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
                return finalize_observed_proxy_response(
                    response,
                    cors_config,
                    request_origin.as_deref(),
                    request_observation_with_token(
                        metrics.as_ref(),
                        route.id.as_str(),
                        &method,
                        &path,
                        Some(token_label.as_str()),
                        &request_id,
                        request_started_at,
                    ),
                    "rate_limited",
                );
            }
        }
    }

    let downstream_permit = if let Some(concurrency) = &state.concurrency {
        match concurrency.acquire_downstream() {
            Ok(permit) => permit,
            Err(_) => {
                return finalize_observed_proxy_response(
                    json_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "downstream_concurrency_exceeded",
                    ),
                    cors_config,
                    request_origin.as_deref(),
                    request_observation_with_token(
                        metrics.as_ref(),
                        route.id.as_str(),
                        &method,
                        &path,
                        Some(token_label.as_str()),
                        &request_id,
                        request_started_at,
                    ),
                    "concurrency",
                );
            }
        }
    } else {
        None
    };

    let Some(upstream_url) = proxy::build_upstream_url_for_route(route, &path, query.as_deref())
    else {
        return finalize_observed_proxy_response(
            json_error(StatusCode::BAD_REQUEST, "invalid_upstream_path"),
            cors_config,
            request_origin.as_deref(),
            request_observation_with_token(
                metrics.as_ref(),
                route.id.as_str(),
                &method,
                &path,
                Some(token_label.as_str()),
                &request_id,
                request_started_at,
            ),
            "gateway_error",
        );
    };

    let upstream_headers = match proxy::prepare_upstream_headers(request.headers(), &route.upstream)
    {
        Ok(headers) => headers,
        Err(_) => {
            return finalize_observed_proxy_response(
                json_error(StatusCode::BAD_GATEWAY, "upstream_header_error"),
                cors_config,
                request_origin.as_deref(),
                request_observation_with_token(
                    metrics.as_ref(),
                    route.id.as_str(),
                    &method,
                    &path,
                    Some(token_label.as_str()),
                    &request_id,
                    request_started_at,
                ),
                "gateway_error",
            );
        }
    };

    let Some(upstream_client) = state.upstream_clients.get(&route.id) else {
        return finalize_observed_proxy_response(
            json_error(StatusCode::BAD_GATEWAY, "upstream_client_not_found"),
            cors_config,
            request_origin.as_deref(),
            request_observation_with_token(
                metrics.as_ref(),
                route.id.as_str(),
                &method,
                &path,
                Some(token_label.as_str()),
                &request_id,
                request_started_at,
            ),
            "gateway_error",
        );
    };

    let upstream_permit = if let Some(concurrency) = &state.concurrency {
        match concurrency.acquire_upstream(route).await {
            Ok(permit) => permit,
            Err(_) => {
                return finalize_observed_proxy_response(
                    json_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "upstream_concurrency_exceeded",
                    ),
                    cors_config,
                    request_origin.as_deref(),
                    request_observation_with_token(
                        metrics.as_ref(),
                        route.id.as_str(),
                        &method,
                        &path,
                        Some(token_label.as_str()),
                        &request_id,
                        request_started_at,
                    ),
                    "concurrency",
                );
            }
        }
    } else {
        None
    };

    if let Some(metrics) = &metrics {
        metrics.inc_inflight(route.id.as_str());
    }

    match forward_to_upstream(
        upstream_client,
        route,
        request,
        upstream_url,
        upstream_headers,
        route.id.as_str(),
        metrics.as_deref(),
    )
    .await
    {
        Ok(ForwardSuccess { response, is_sse }) => {
            if let Some(metrics) = &metrics
                && is_sse
            {
                metrics.inc_sse_inflight(route.id.as_str());
            }

            let bytes_sent = Arc::new(AtomicU64::new(0));
            let completion_guard = ResponseCompletionGuard {
                metrics: metrics.clone(),
                route_id: route.id.clone(),
                token_label: Some(token_label.clone()),
                method: method.clone(),
                path: path.clone(),
                outcome: "success",
                status: response.status(),
                request_started_at,
                request_id: request_id.clone(),
                bytes_sent: bytes_sent.clone(),
                track_inflight: metrics.is_some(),
                track_sse: is_sse,
            };
            let response_guards = ResponseGuards {
                downstream_permit,
                upstream_permit,
                completion_guard: Some(completion_guard),
                bytes_sent: Some(bytes_sent),
            };
            let mut response = attach_response_guards(response, response_guards);
            observability::insert_request_id_header(response.headers_mut(), &request_id);
            finalize_response_with_cors(response, cors_config, request_origin.as_deref())
        }
        Err(error) => {
            if let Some(metrics) = &metrics {
                metrics.dec_inflight(route.id.as_str());
            }
            finalize_observed_proxy_response(
                error_response(error),
                cors_config,
                request_origin.as_deref(),
                request_observation_with_token(
                    metrics.as_ref(),
                    route.id.as_str(),
                    &method,
                    &path,
                    Some(token_label.as_str()),
                    &request_id,
                    request_started_at,
                ),
                "upstream_error",
            )
        }
    }
}

#[derive(Clone, Copy)]
struct RequestObservation<'a> {
    metrics: Option<&'a Arc<observability::GatewayMetrics>>,
    route_id: &'a str,
    token_label: Option<&'a str>,
    method: &'a Method,
    path: &'a str,
    request_id: &'a str,
    request_started_at: tokio::time::Instant,
}

fn request_observation<'a>(
    metrics: Option<&'a Arc<observability::GatewayMetrics>>,
    route_id: &'a str,
    method: &'a Method,
    path: &'a str,
    request_id: &'a str,
    request_started_at: tokio::time::Instant,
) -> RequestObservation<'a> {
    request_observation_with_token(
        metrics,
        route_id,
        method,
        path,
        None,
        request_id,
        request_started_at,
    )
}

fn request_observation_with_token<'a>(
    metrics: Option<&'a Arc<observability::GatewayMetrics>>,
    route_id: &'a str,
    method: &'a Method,
    path: &'a str,
    token_label: Option<&'a str>,
    request_id: &'a str,
    request_started_at: tokio::time::Instant,
) -> RequestObservation<'a> {
    RequestObservation {
        metrics,
        route_id,
        token_label,
        method,
        path,
        request_id,
        request_started_at,
    }
}

fn finalize_observed_proxy_response(
    mut response: Response<Body>,
    cors_config: Option<&CorsConfig>,
    request_origin: Option<&str>,
    observation: RequestObservation<'_>,
    outcome: &'static str,
) -> Response<Body> {
    observe_and_log_request_completion(observation, outcome, response.status(), 0);
    observability::insert_request_id_header(response.headers_mut(), observation.request_id);
    finalize_response_with_cors(response, cors_config, request_origin)
}

fn observe_and_log_request_completion(
    observation: RequestObservation<'_>,
    outcome: &str,
    status: StatusCode,
    bytes_sent: u64,
) {
    let duration = observation.request_started_at.elapsed();
    if let Some(metrics) = observation.metrics {
        metrics.observe_request(
            observation.route_id,
            observation.token_label,
            observation.method,
            outcome,
            status,
            duration,
        );
    }

    let duration_ms = duration.as_millis() as u64;
    if status.is_server_error() {
        error!(
            request_id = observation.request_id,
            method = observation.method.as_str(),
            route_id = observation.route_id,
            path = observation.path,
            outcome = outcome,
            status = status.as_u16(),
            duration_ms = duration_ms,
            bytes_sent = bytes_sent,
            "request completed"
        );
    } else if status.is_client_error() {
        warn!(
            request_id = observation.request_id,
            method = observation.method.as_str(),
            route_id = observation.route_id,
            path = observation.path,
            outcome = outcome,
            status = status.as_u16(),
            duration_ms = duration_ms,
            bytes_sent = bytes_sent,
            "request completed"
        );
    } else {
        info!(
            request_id = observation.request_id,
            method = observation.method.as_str(),
            route_id = observation.route_id,
            path = observation.path,
            outcome = outcome,
            status = status.as_u16(),
            duration_ms = duration_ms,
            bytes_sent = bytes_sent,
            "request completed"
        );
    }
}

async fn forward_to_upstream(
    upstream_client: &reqwest::Client,
    route: &RouteConfig,
    request: Request<Body>,
    upstream_url: String,
    upstream_headers: http::HeaderMap,
    route_id: &str,
    metrics: Option<&observability::GatewayMetrics>,
) -> Result<ForwardSuccess, UpstreamError> {
    let upstream_host = upstream_host_label(&upstream_url);
    let upstream_started_at = tokio::time::Instant::now();
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
        Ok(Ok(response)) => {
            if let Some(metrics) = metrics {
                metrics.observe_upstream_duration(
                    route_id,
                    upstream_host.as_str(),
                    "ok",
                    upstream_started_at.elapsed(),
                );
            }
            response
        }
        Ok(Err(err)) => {
            if let Some(metrics) = metrics {
                let result = if err.is_connect() {
                    "connect_error"
                } else if err.is_timeout() {
                    "timeout"
                } else {
                    "request_error"
                };
                metrics.observe_upstream_duration(
                    route_id,
                    upstream_host.as_str(),
                    result,
                    upstream_started_at.elapsed(),
                );
            }
            return Err(UpstreamError::Request(err));
        }
        Err(_) => {
            if let Some(metrics) = metrics {
                metrics.observe_upstream_duration(
                    route_id,
                    upstream_host.as_str(),
                    "timeout",
                    upstream_started_at.elapsed(),
                );
            }
            return Err(UpstreamError::Timeout);
        }
    };

    let is_sse = is_sse_response(upstream_response.headers());
    Ok(ForwardSuccess {
        response: response_from_upstream(upstream_response, is_sse, deadline),
        is_sse,
    })
}

fn upstream_host_label(upstream_url: &str) -> String {
    reqwest::Url::parse(upstream_url)
        .ok()
        .and_then(|url| url.host_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

type ProxyBodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>;

struct ForwardSuccess {
    response: Response<Body>,
    is_sse: bool,
}

struct ResponseCompletionGuard {
    metrics: Option<Arc<observability::GatewayMetrics>>,
    route_id: String,
    token_label: Option<String>,
    method: Method,
    path: String,
    outcome: &'static str,
    status: StatusCode,
    request_started_at: tokio::time::Instant,
    request_id: String,
    bytes_sent: Arc<AtomicU64>,
    track_inflight: bool,
    track_sse: bool,
}

impl Drop for ResponseCompletionGuard {
    fn drop(&mut self) {
        let bytes_sent = self.bytes_sent.load(Ordering::Relaxed);
        if let Some(metrics) = &self.metrics {
            if self.track_inflight {
                metrics.dec_inflight(self.route_id.as_str());
            }
            if self.track_sse {
                metrics.dec_sse_inflight(self.route_id.as_str());
            }
        }
        observe_and_log_request_completion(
            RequestObservation {
                metrics: self.metrics.as_ref(),
                route_id: self.route_id.as_str(),
                token_label: self.token_label.as_deref(),
                method: &self.method,
                path: self.path.as_str(),
                request_id: self.request_id.as_str(),
                request_started_at: self.request_started_at,
            },
            self.outcome,
            self.status,
            bytes_sent,
        );
    }
}

struct ResponseGuards {
    downstream_permit: Option<OwnedSemaphorePermit>,
    upstream_permit: Option<OwnedSemaphorePermit>,
    completion_guard: Option<ResponseCompletionGuard>,
    bytes_sent: Option<Arc<AtomicU64>>,
}

impl ResponseGuards {
    fn is_empty(&self) -> bool {
        self.downstream_permit.is_none()
            && self.upstream_permit.is_none()
            && self.completion_guard.is_none()
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
            if let (Some(bytes_sent), Ok(chunk)) = (&guards.bytes_sent, &item) {
                bytes_sent.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
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
        assert!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .is_some(),
            "healthz response should contain x-request-id"
        );
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
        assert!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .is_some(),
            "not found response should contain x-request-id"
        );

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
        assert!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .is_some(),
            "unauthorized response should contain x-request-id"
        );
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
            observability: None,
        }
    }
}

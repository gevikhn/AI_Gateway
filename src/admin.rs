use crate::config::AppConfig;
use crate::server::{AppState, build_runtime_state};
use axum::Router;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::{get, post};
use http::header::CONTENT_TYPE;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{error, info, warn};

pub fn register_admin_routes(router: Router<AppState>, prefix: &str) -> Router<AppState> {
    let prefix = prefix.trim_end_matches('/');
    router
        .route(&format!("{prefix}/ui"), get(admin_ui_handler))
        .route(&format!("{prefix}/login"), get(admin_login_handler))
        .route(
            &format!("{prefix}/api/config"),
            get(admin_config_get_handler).put(admin_config_apply_handler),
        )
        .route(
            &format!("{prefix}/api/config/save"),
            post(admin_config_save_handler),
        )
        .route(&format!("{prefix}/api/metrics"), get(admin_metrics_handler))
        .route(
            &format!("{prefix}/api/metrics/ip"),
            get(admin_ip_metrics_handler),
        )
}

fn is_admin_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected_token) = &state.admin_token else {
        return false;
    };
    let Some(auth_header) = headers.get(http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = auth_header.to_str() else {
        return false;
    };
    let Some((scheme, token)) = value.trim().split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    token.trim() == expected_token.as_str()
}

fn json_response(status: StatusCode, body: String) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    response
}

fn json_error(status: StatusCode, code: &str) -> Response<Body> {
    json_response(
        status,
        format!(r#"{{"error":{}}}"#, serde_json::json!(code)),
    )
}

fn json_ok(data: &impl serde::Serialize) -> Response<Body> {
    match serde_json::to_string(data) {
        Ok(json) => json_response(StatusCode::OK, json),
        Err(err) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("serialization_error: {err}"),
        ),
    }
}

/// IP 监控查询参数
#[derive(Debug, Deserialize)]
#[serde(default)]
struct IpMetricsQuery {
    /// 时间窗口: 5m, 1h, 24h, 1w, 1m
    window: String,
    /// 按 IP 筛选（支持部分匹配）
    ip: Option<String>,
    /// 排序字段: requests, errors, bytes_in, bytes_out, latency_avg
    sort_by: String,
    /// 排序方向: asc, desc
    order: String,
    /// 返回数量限制
    limit: usize,
}

impl Default for IpMetricsQuery {
    fn default() -> Self {
        Self {
            window: "1h".to_string(),
            ip: None,
            sort_by: "requests".to_string(),
            order: "desc".to_string(),
            limit: 100,
        }
    }
}

/// IP 监控响应结构
#[derive(Debug, serde::Serialize)]
struct IpMetricsResponse {
    generated_at_unix_ms: u64,
    window: String,
    window_seconds: u64,
    total_ips: usize,
    total_requests: u64,
    total_errors: u64,
    ips: Vec<IpMetricsEntry>,
}

/// 单个 IP 的监控数据
#[derive(Debug, serde::Serialize)]
struct IpMetricsEntry {
    ip: String,
    requests: u64,
    errors: u64,
    bytes_in: u64,
    bytes_out: u64,
    latency_avg_ms: u64,
    latency_p99_ms: u64,
    routes: Vec<String>,
    tokens: Vec<String>,
    first_seen_unix_ms: u64,
    last_seen_unix_ms: u64,
}

async fn admin_ui_handler(State(state): State<AppState>) -> Response<Body> {
    let prefix = state.admin_path_prefix.as_deref().unwrap_or("/admin");
    let mut response = Response::new(Body::from(admin_dashboard_html(prefix)));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

async fn admin_config_get_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let runtime = state.runtime.load();
    json_ok(runtime.config.as_ref())
}

async fn admin_config_apply_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let new_config: AppConfig = match serde_json::from_slice(&body) {
        Ok(config) => config,
        Err(err) => {
            warn!(error = %err, "admin: invalid config JSON");
            return json_error(StatusCode::BAD_REQUEST, &format!("invalid_json: {err}"));
        }
    };

    if let Err(err) = new_config.validate() {
        warn!(error = %err, "admin: config validation failed");
        return json_error(StatusCode::BAD_REQUEST, &format!("validation_error: {err}"));
    }

    let new_config = Arc::new(new_config);
    let new_runtime = match build_runtime_state(new_config.clone()) {
        Ok(runtime) => runtime,
        Err(err) => {
            error!(error = %err, "admin: failed to build runtime state");
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("runtime_build_error: {err}"),
            );
        }
    };

    state.runtime.store(Arc::new(new_runtime));
    info!("admin: config applied successfully (hot-swapped)");

    json_ok(&serde_json::json!({
        "status": "applied",
        "message": "Configuration applied successfully. Changes are effective immediately."
    }))
}

async fn admin_config_save_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let Some(config_path) = &state.config_path else {
        return json_error(StatusCode::BAD_REQUEST, "config_path_unknown");
    };

    let runtime = state.runtime.load();
    let yaml = match serde_yaml::to_string(runtime.config.as_ref()) {
        Ok(yaml) => yaml,
        Err(err) => {
            error!(error = %err, "admin: failed to serialize config to YAML");
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("serialization_error: {err}"),
            );
        }
    };

    let temp_path = config_path.with_extension("yaml.tmp");
    if let Err(err) = std::fs::write(&temp_path, &yaml) {
        error!(error = %err, path = %temp_path.display(), "admin: failed to write temp config file");
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write_error: {err}"),
        );
    }
    if let Err(err) = std::fs::rename(&temp_path, config_path) {
        error!(error = %err, "admin: failed to rename temp config file");
        let _ = std::fs::remove_file(&temp_path);
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("rename_error: {err}"),
        );
    }

    info!(path = %config_path.display(), "admin: config saved to file");

    json_ok(&serde_json::json!({
        "status": "saved",
        "path": config_path.display().to_string(),
        "message": "Configuration persisted to YAML file."
    }))
}

async fn admin_metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if let Some(summary) = state.observability.snapshot_summary() {
        json_ok(&summary)
    } else {
        json_error(StatusCode::NOT_FOUND, "metrics_not_available")
    }
}

/// IP 监控 API 处理函数
/// GET /admin/api/metrics/ip?window=1h&ip=192.168&sort_by=requests&order=desc&limit=50
async fn admin_ip_metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IpMetricsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    // 验证时间窗口参数
    let window_seconds = match query.window.as_str() {
        "5m" => 5 * 60,
        "1h" => 60 * 60,
        "24h" => 24 * 60 * 60,
        "1w" => 7 * 24 * 60 * 60,
        "1m" => 30 * 24 * 60 * 60,
        _ => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_window: supported values are 5m, 1h, 24h, 1w, 1m",
            );
        }
    };

    // 验证排序字段
    let valid_sort_fields = ["requests", "errors", "bytes_in", "bytes_out", "latency_avg"];
    if !valid_sort_fields.contains(&query.sort_by.as_str()) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_sort_by: supported values are requests, errors, bytes_in, bytes_out, latency_avg",
        );
    }

    // 验证排序方向
    let sort_desc = match query.order.as_str() {
        "asc" => false,
        "desc" => true,
        _ => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid_order: supported values are asc, desc",
            );
        }
    };

    // 限制返回数量
    let limit = query.limit.min(1000).max(1);

    // 从 observability 模块获取 IP 监控数据
    let summary = match state.observability.snapshot_summary() {
        Some(s) => s,
        None => {
            return json_error(StatusCode::NOT_FOUND, "metrics_not_available");
        }
    };

    // 转换 IP 统计数据为响应格式
    let mut ips: Vec<IpMetricsEntry> = summary
        .ip_stats
        .ips
        .into_iter()
        .filter(|ip| {
            // IP 筛选
            if let Some(ref filter) = query.ip {
                if !filter.is_empty() && !ip.ip.contains(filter) {
                    return false;
                }
            }
            true
        })
        .map(|ip| {
            // 根据时间窗口获取对应的请求数
            let requests = match query.window.as_str() {
                "5m" => ip.requests_5m,
                "1h" => ip.requests_1h,
                "24h" => ip.requests_24h,
                "1w" => ip.requests_7d,
                "1m" => ip.requests_30d,
                _ => ip.requests_1h,
            };

            // 从 urls 中提取路由信息
            let routes: Vec<String> = ip.urls.iter().map(|u| u.url.clone()).collect();

            // 从 tokens 中提取 token 信息
            let tokens: Vec<String> = ip.tokens.iter().map(|t| t.token.clone()).collect();

            IpMetricsEntry {
                ip: ip.ip,
                requests,
                // 目前 observability 模块不记录错误数、字节数和延迟，使用默认值
                errors: 0,
                bytes_in: 0,
                bytes_out: 0,
                latency_avg_ms: 0,
                latency_p99_ms: 0,
                routes,
                tokens,
                // 目前不记录首次/最后访问时间，使用当前时间
                first_seen_unix_ms: summary.generated_at_unix_ms,
                last_seen_unix_ms: summary.generated_at_unix_ms,
            }
        })
        .collect();

    // 排序
    ips.sort_by(|a, b| {
        let cmp = match query.sort_by.as_str() {
            "requests" => a.requests.cmp(&b.requests),
            "errors" => a.errors.cmp(&b.errors),
            "bytes_in" => a.bytes_in.cmp(&b.bytes_in),
            "bytes_out" => a.bytes_out.cmp(&b.bytes_out),
            "latency_avg" => a.latency_avg_ms.cmp(&b.latency_avg_ms),
            _ => a.requests.cmp(&b.requests),
        };
        if sort_desc {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // 限制数量
    let total_ips = ips.len();
    let total_requests: u64 = ips.iter().map(|ip| ip.requests).sum();
    let total_errors: u64 = ips.iter().map(|ip| ip.errors).sum();
    ips.truncate(limit);

    let response = IpMetricsResponse {
        generated_at_unix_ms: summary.generated_at_unix_ms,
        window: query.window,
        window_seconds: window_seconds as u64,
        total_ips,
        total_requests,
        total_errors,
        ips,
    };

    json_ok(&response)
}

async fn admin_login_handler(State(state): State<AppState>) -> Response<Body> {
    let prefix = state.admin_path_prefix.as_deref().unwrap_or("/admin");
    let api_config_url = format!("{prefix}/api/config");
    let api_save_url = format!("{prefix}/api/config/save");

    let html = LOGIN_TEMPLATE
        .replace("{{CSS}}", CSS_STYLES)
        .replace("{{JS}}", LOGIN_JS)
        .replace("{{API_CONFIG_URL}}", &api_config_url)
        .replace("{{API_SAVE_URL}}", &api_save_url)
        .replace("{{ADMIN_PREFIX}}", prefix);

    let mut response = Response::new(Body::from(html));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

// 静态文件嵌入 - 使用 include_str! 在编译时嵌入
// 注意：路径相对于 src/admin.rs 所在位置
const HTML_TEMPLATE: &str = include_str!("./admin/static/index.html");
const LOGIN_TEMPLATE: &str = include_str!("./admin/static/login.html");
const CSS_STYLES: &str = include_str!("./admin/static/styles.css");
const JS_APP: &str = include_str!("./admin/static/app.js");
const LOGIN_JS: &str = include_str!("./admin/static/login.js");

fn admin_dashboard_html(prefix: &str) -> String {
    let api_config_url = format!("{prefix}/api/config");
    let api_save_url = format!("{prefix}/api/config/save");
    let api_metrics_url = format!("{prefix}/api/metrics");

    HTML_TEMPLATE
        .replace("{{CSS}}", CSS_STYLES)
        .replace("{{JS}}", JS_APP)
        .replace("{{API_CONFIG_URL}}", &api_config_url)
        .replace("{{API_SAVE_URL}}", &api_save_url)
        .replace("{{API_METRICS_URL}}", &api_metrics_url)
        .replace("{{ADMIN_PREFIX}}", prefix)
}
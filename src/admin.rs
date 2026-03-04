use crate::config::{AppConfig, BanRule};
use crate::server::{AppState, build_runtime_state};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::{get, post};
use http::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

pub fn register_admin_routes(router: Router<AppState>, prefix: &str) -> Router<AppState> {
    let prefix = prefix.trim_end_matches('/');
    router
        .route(&format!("{prefix}/ui"), get(admin_ui_handler))
        .route(&format!("{prefix}/login"), get(admin_login_handler))
        .route(&format!("{prefix}/favicon.ico"), get(admin_favicon_handler))
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
        // API Key 管理路由
        .route(&format!("{prefix}/api/keys"), get(admin_list_api_keys))
        .route(
            &format!("{prefix}/api/keys/{{id}}/ban"),
            post(admin_ban_api_key),
        )
        .route(
            &format!("{prefix}/api/keys/{{id}}/unban"),
            post(admin_unban_api_key),
        )
        .route(
            &format!("{prefix}/api/keys/{{id}}/ban-logs"),
            get(admin_get_ban_logs),
        )
        // 获取所有封禁日志（不指定 API Key）
        .route(&format!("{prefix}/api/ban-logs"), get(admin_get_all_ban_logs))
        // Token统计路由
        .route(&format!("{prefix}/api/token-stats/summary"), get(admin_token_stats_summary))
        .route(&format!("{prefix}/api/token-stats/keys"), get(admin_list_api_key_token_stats))
        .route(&format!("{prefix}/api/token-stats/keys/{{id}}"), get(admin_get_api_key_token_stats))
        .route(&format!("{prefix}/api/token-stats/routes"), get(admin_list_route_token_stats))
        .route(&format!("{prefix}/api/token-stats/routes/{{id}}"), get(admin_get_route_token_stats))
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
    // 获取当前 runtime 用于迁移封禁状态
    let old_runtime = state.runtime.load();
    // 从旧的 runtime 获取 token_quota_checker
    let token_quota_checker = old_runtime._token_quota_checker.clone();
    let new_runtime = match build_runtime_state(new_config.clone(), Some(&old_runtime), token_quota_checker).await {
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

    // Sync config to database
    match state.config_storage.sync_config(&new_config).await {
        Ok(result) => {
            info!(
                routes_added = result.routes_added,
                routes_updated = result.routes_updated,
                routes_deleted = result.routes_deleted,
                api_keys_added = result.api_keys_added,
                api_keys_updated = result.api_keys_updated,
                api_keys_deleted = result.api_keys_deleted,
                "admin: config synced to database"
            );
        }
        Err(err) => {
            error!(error = %err, "admin: failed to sync config to database");
            // Don't fail the request, just log the error
        }
    }

    info!("admin: config applied successfully (hot-swapped)");

    json_ok(&serde_json::json!({
        "status": "applied",
        "message": "Configuration applied successfully. Changes are effective immediately."
    }))
}

/// 主配置结构（不包含 routes 和 api_keys）
#[derive(Debug, Serialize)]
struct MainConfigOnly {
    pub listen: String,
    pub gateway_auth: crate::config::GatewayAuthConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbound_tls: Option<crate::config::InboundTlsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<crate::config::CorsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<crate::config::RateLimitConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<crate::config::ConcurrencyConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observability: Option<crate::config::ObservabilityConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<crate::config::AdminConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_stats: Option<crate::config::TokenStatsConfig>,
}

/// Ban Rules 独立配置文件结构
#[derive(Debug, Serialize, Deserialize)]
struct BanRulesFileConfig {
    pub rules: Vec<BanRule>,
}

/// 将 ID 转换为安全的文件名
fn sanitize_filename(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

/// 清理目录中不再需要的文件
fn cleanup_old_files<T, F>(dir: &Path, items: &[T], get_id: F) -> Result<(), String>
where
    F: Fn(&T) -> &str,
{
    let valid_ids: HashSet<String> = items
        .iter()
        .map(|item| sanitize_filename(get_id(item)))
        .collect();

    for entry in fs::read_dir(dir).map_err(|e| format!("读取目录失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
        let path = entry.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if !valid_ids.contains(stem) {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

/// 将配置保存到多个文件
fn save_config_to_files(config: &AppConfig, config_path: &Path) -> Result<(), String> {
    // 1. 确定 data 目录路径
    let base_dir = config_path.parent().unwrap_or(Path::new("."));
    let data_dir = config.data_dir.as_deref().unwrap_or("./data");
    let data_path = base_dir.join(data_dir);

    // 2. 创建目录结构
    fs::create_dir_all(&data_path).map_err(|e| format!("创建 data 目录失败: {}", e))?;
    fs::create_dir_all(data_path.join("routes"))
        .map_err(|e| format!("创建 routes 目录失败: {}", e))?;
    fs::create_dir_all(data_path.join("apikeys"))
        .map_err(|e| format!("创建 apikeys 目录失败: {}", e))?;

    // 3. 保存主配置（不包含 routes 和 api_keys）
    let main_config = MainConfigOnly {
        listen: config.listen.clone(),
        gateway_auth: config.gateway_auth.clone(),
        data_dir: config.data_dir.clone(),
        inbound_tls: config.inbound_tls.clone(),
        cors: config.cors.clone(),
        rate_limit: config.rate_limit.clone(),
        concurrency: config.concurrency.clone(),
        observability: config.observability.clone(),
        admin: config.admin.clone(),
        token_stats: config.token_stats.clone(),
    };
    let main_yaml =
        serde_yaml::to_string(&main_config).map_err(|e| format!("序列化主配置失败: {}", e))?;

    let temp_path = config_path.with_extension("yaml.tmp");
    fs::write(&temp_path, &main_yaml).map_err(|e| format!("写入主配置失败: {}", e))?;
    fs::rename(&temp_path, config_path).map_err(|e| format!("重命名主配置失败: {}", e))?;

    // 4. 保存 routes
    if let Some(routes) = &config.routes {
        // 清理已删除的 route 文件
        let routes_dir = data_path.join("routes");
        cleanup_old_files(&routes_dir, routes, |r| &r.id)?;

        for route in routes {
            let file_name = format!("{}.yaml", sanitize_filename(&route.id));
            let route_path = routes_dir.join(&file_name);
            let route_yaml = serde_yaml::to_string(route)
                .map_err(|e| format!("序列化 route {} 失败: {}", route.id, e))?;
            fs::write(&route_path, &route_yaml)
                .map_err(|e| format!("写入 route {} 失败: {}", route.id, e))?;
        }
    }

    // 5. 保存 api_keys
    if let Some(api_keys) = &config.api_keys {
        // 清理已删除的 apikey 文件
        let keys_dir = data_path.join("apikeys");
        cleanup_old_files(&keys_dir, &api_keys.keys, |k| &k.id)?;

        for key in &api_keys.keys {
            let file_name = format!("{}.yaml", sanitize_filename(&key.id));
            let key_path = keys_dir.join(&file_name);
            let key_yaml = serde_yaml::to_string(key)
                .map_err(|e| format!("序列化 apikey {} 失败: {}", key.id, e))?;
            fs::write(&key_path, &key_yaml)
                .map_err(|e| format!("写入 apikey {} 失败: {}", key.id, e))?;
        }

        // 6. 保存 ban_rules
        let ban_rules_path = data_path.join("ban_rules.yaml");
        let ban_config = BanRulesFileConfig {
            rules: api_keys.ban_rules.clone(),
        };
        let ban_yaml = serde_yaml::to_string(&ban_config)
            .map_err(|e| format!("序列化 ban_rules 失败: {}", e))?;
        fs::write(&ban_rules_path, &ban_yaml)
            .map_err(|e| format!("写入 ban_rules 失败: {}", e))?;
    }

    Ok(())
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

    // 使用新的分散保存函数
    match save_config_to_files(runtime.config.as_ref(), config_path) {
        Ok(()) => {
            info!(path = %config_path.display(), "admin: config saved to files");
            json_ok(&serde_json::json!({
                "status": "saved",
                "path": config_path.display().to_string(),
                "message": "Configuration saved to multiple files."
            }))
        }
        Err(err) => {
            error!(error = %err, "admin: failed to save config files");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, &err)
        }
    }
}

async fn admin_metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if let Some(mut summary) = state.observability.snapshot_summary() {
        // Get valid route IDs from config_storage
        let valid_routes = state.config_storage.get_valid_route_ids().await;

        // Filter routes to only include valid ones
        summary.routes.retain(|r| valid_routes.contains(&r.route_id));

        // Filter tokens to only include valid API keys
        // Note: summary.tokens contains token values (original key), not key IDs
        // We need to check each token value against the config_storage
        let valid_token_hashes: std::collections::HashSet<String> = state
            .config_storage
            .get_valid_api_key_hashes()
            .await;
        summary.tokens.retain(|t| {
            // Compute hash of the token value and check if it's in valid hashes
            let mut hasher = Sha256::new();
            hasher.update(t.token.as_bytes());
            let token_hash = format!("{:x}", hasher.finalize());
            valid_token_hashes.contains(&token_hash)
        });

        // 重新计算总请求数，使其与过滤后的路由列表一致
        summary.total_requests_1h = summary
            .routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_1h));
        summary.total_requests_24h = summary
            .routes
            .iter()
            .fold(0_u64, |acc, route| acc.saturating_add(route.requests_24h));

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

/// Favicon ICO 处理函数
/// 返回 admin-logo SVG 图标
async fn admin_favicon_handler() -> Response<Body> {
    // 使用与 admin-logo 相同的 SVG 图标
    // 使用 rgb() 格式避免 # 字符问题
    const FAVICON_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="rgb(20,184,166)" stroke-width="2"><path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"/></svg>"#;

    let mut response = Response::new(Body::from(FAVICON_SVG));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("image/svg+xml"),
    );
    // 缓存 1 天
    response.headers_mut().insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("public, max-age=86400"),
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
    let api_keys_url = format!("{prefix}/api/keys");
    let api_token_stats_url = format!("{prefix}/api/token-stats");

    HTML_TEMPLATE
        .replace("{{CSS}}", CSS_STYLES)
        .replace("{{JS}}", JS_APP)
        .replace("{{API_CONFIG_URL}}", &api_config_url)
        .replace("{{API_SAVE_URL}}", &api_save_url)
        .replace("{{API_METRICS_URL}}", &api_metrics_url)
        .replace("{{API_KEYS_URL}}", &api_keys_url)
        .replace("{{API_TOKEN_STATS_URL}}", &api_token_stats_url)
        .replace("{{ADMIN_PREFIX}}", prefix)
}

// ===== API Key 管理 API =====

/// API Key 列表响应
#[derive(Debug, Serialize)]
struct ApiKeyListResponse {
    keys: Vec<ApiKeyInfo>,
}

/// API Key 信息
#[derive(Debug, Serialize)]
struct ApiKeyInfo {
    id: String,
    key: String,
    route_id: Option<String>,
    enabled: bool,
    remark: String,
    is_banned: bool,
    banned_at: Option<u64>,
    ban_expires_at: Option<u64>,
    triggered_rule_id: Option<String>,
    ban_reason: Option<String>,
    ban_count: u32,
    token_quota: Option<crate::config::TokenQuotaConfig>,
}

/// 封禁请求
#[derive(Debug, Deserialize)]
struct BanRequest {
    duration_secs: u64,
    reason: String,
}

/// 封禁日志列表响应
#[derive(Debug, Serialize)]
struct BanLogListResponse {
    logs: Vec<BanLogInfo>,
}

/// 封禁日志信息
#[derive(Debug, Serialize)]
struct BanLogInfo {
    id: String,
    api_key_id: String,
    rule_id: String,
    reason: String,
    banned_at: u64,
    banned_until: u64,
    unbanned_at: Option<u64>,
    metrics_requests: u64,
    metrics_errors: u64,
    metrics_error_rate: f64,
}

/// 列出所有 API Keys
async fn admin_list_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let runtime = state.runtime.load();
    let api_key_manager = match runtime.api_key_manager.as_ref() {
        Some(manager) => manager,
        None => return json_ok(&ApiKeyListResponse { keys: vec![] }),
    };

    let keys = api_key_manager.get_all_keys().await;
    let key_infos: Vec<ApiKeyInfo> = keys
        .into_iter()
        .map(|key| {
            let (is_banned, banned_at, ban_expires_at, triggered_rule_id, ban_reason, ban_count) = key
                .ban_status
                .as_ref()
                .map(|s| {
                    (
                        s.is_banned,
                        s.banned_at,
                        s.banned_until,
                        s.triggered_rule_id.clone(),
                        s.reason.clone(),
                        s.ban_count,
                    )
                })
                .unwrap_or((false, None, None, None, None, 0));

            ApiKeyInfo {
                id: key.id,
                key: key.key,
                route_id: key.route_id,
                enabled: key.enabled,
                remark: key.remark,
                is_banned,
                banned_at,
                ban_expires_at,
                triggered_rule_id,
                ban_reason,
                ban_count,
                token_quota: key.token_quota,
            }
        })
        .collect();

    json_ok(&ApiKeyListResponse { keys: key_infos })
}

/// 手动封禁 API Key
async fn admin_ban_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    body: axum::body::Bytes,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let runtime = state.runtime.load();
    let api_key_manager = match runtime.api_key_manager.as_ref() {
        Some(manager) => manager,
        None => return json_error(StatusCode::NOT_FOUND, "api_key_manager_not_available"),
    };

    let req: BanRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(err) => {
            return json_error(StatusCode::BAD_REQUEST, &format!("invalid_json: {err}"));
        }
    };

    // 通过 ID 获取 key 值
    let key_value = match api_key_manager.get_key_by_id(&id).await {
        Some(key) => key,
        None => return json_error(StatusCode::NOT_FOUND, "api_key_not_found"),
    };

    match api_key_manager
        .ban_key(&key_value, req.duration_secs, req.reason)
        .await
    {
        Ok(status) => json_ok(&serde_json::json!({
            "status": "banned",
            "banned_until": status.banned_until,
            "ban_count": status.ban_count,
        })),
        Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
}

/// 手动解封 API Key
async fn admin_unban_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let runtime = state.runtime.load();
    let api_key_manager = match runtime.api_key_manager.as_ref() {
        Some(manager) => manager,
        None => return json_error(StatusCode::NOT_FOUND, "api_key_manager_not_available"),
    };

    // 通过 ID 获取 key 值
    let key_value = match api_key_manager.get_key_by_id(&id).await {
        Some(key) => key,
        None => return json_error(StatusCode::NOT_FOUND, "api_key_not_found"),
    };

    match api_key_manager.unban_key(&key_value).await {
        Ok(()) => json_ok(&serde_json::json!({
            "status": "unbanned",
        })),
        Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()),
    }
}

/// 获取 API Key 的封禁日志
async fn admin_get_ban_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<BanLogQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let runtime = state.runtime.load();
    let api_key_manager = match runtime.api_key_manager.as_ref() {
        Some(manager) => manager,
        None => return json_ok(&BanLogListResponse { logs: vec![] }),
    };

    let Some(ban_log_store) = api_key_manager.ban_log_store() else {
        return json_ok(&BanLogListResponse { logs: vec![] });
    };

    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0);

    match ban_log_store.query_by_api_key(&id, limit, offset).await {
        Ok(entries) => {
            let logs: Vec<BanLogInfo> = entries
                .into_iter()
                .map(|entry| BanLogInfo {
                    id: entry.id,
                    api_key_id: entry.api_key_id,
                    rule_id: entry.rule_id,
                    reason: entry.reason,
                    banned_at: entry.banned_at,
                    banned_until: entry.banned_until,
                    unbanned_at: entry.unbanned_at,
                    metrics_requests: entry.metrics_snapshot.requests,
                    metrics_errors: entry.metrics_snapshot.errors,
                    metrics_error_rate: entry.metrics_snapshot.error_rate,
                })
                .collect();
            json_ok(&BanLogListResponse { logs })
        }
        Err(err) => {
            error!("Failed to query ban logs: {}", err);
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

/// 获取所有封禁日志
async fn admin_get_all_ban_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BanLogQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let runtime = state.runtime.load();
    let api_key_manager = match runtime.api_key_manager.as_ref() {
        Some(manager) => manager,
        None => {
            info!("admin: api_key_manager not available, returning empty ban logs");
            return json_ok(&BanLogListResponse { logs: vec![] });
        }
    };

    let Some(ban_log_store) = api_key_manager.ban_log_store() else {
        info!("admin: ban_log_store not available, returning empty ban logs");
        return json_ok(&BanLogListResponse { logs: vec![] });
    };

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    match ban_log_store.query_recent(limit, offset).await {
        Ok(entries) => {
            info!("admin: queried {} ban logs", entries.len());
            let logs: Vec<BanLogInfo> = entries
                .into_iter()
                .map(|entry| BanLogInfo {
                    id: entry.id,
                    api_key_id: entry.api_key_id,
                    rule_id: entry.rule_id,
                    reason: entry.reason,
                    banned_at: entry.banned_at,
                    banned_until: entry.banned_until,
                    unbanned_at: entry.unbanned_at,
                    metrics_requests: entry.metrics_snapshot.requests,
                    metrics_errors: entry.metrics_snapshot.errors,
                    metrics_error_rate: entry.metrics_snapshot.error_rate,
                })
                .collect();
            json_ok(&BanLogListResponse { logs })
        }
        Err(err) => {
            error!("Failed to query all ban logs: {}", err);
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

/// 封禁日志查询参数
#[derive(Debug, Deserialize)]
struct BanLogQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

/// Token统计查询参数
#[derive(Debug, Deserialize)]
#[serde(default)]
struct TokenStatsQuery {
    /// 时间窗口: day, week, month
    window: String,
}

impl Default for TokenStatsQuery {
    fn default() -> Self {
        Self {
            window: "day".to_string(),
        }
    }
}

/// Token统计摘要响应
#[derive(Debug, Serialize)]
struct TokenStatsSummaryResponse {
    generated_at_unix_ms: u64,
    window: String,
    api_keys: Vec<ApiKeyTokenSummary>,
    routes: Vec<RouteTokenSummary>,
    /// 整体时间序列数据（今日：小时级，本周/本月：天级）
    time_series: Vec<TimeSeriesDataPoint>,
}

/// 时间序列数据点
#[derive(Debug, Serialize)]
struct TimeSeriesDataPoint {
    timestamp: u64,
    label: String,
    input_tokens: u64,
    output_tokens: u64,
    request_count: u64,
}

/// API Key Token统计摘要
#[derive(Debug, Serialize)]
struct ApiKeyTokenSummary {
    api_key_id: String,
    api_key: String, // 实际的API Key值
    today_input_tokens: u64,
    today_output_tokens: u64,
    today_total_tokens: u64,
    week_input_tokens: u64,
    week_output_tokens: u64,
    week_total_tokens: u64,
    month_input_tokens: u64,
    month_output_tokens: u64,
    month_total_tokens: u64,
    request_count_today: u64,
    request_count_week: u64,
    request_count_month: u64,
}

/// Route Token统计摘要
#[derive(Debug, Serialize)]
struct RouteTokenSummary {
    route_id: String,
    today_input_tokens: u64,
    today_output_tokens: u64,
    today_total_tokens: u64,
    week_input_tokens: u64,
    week_output_tokens: u64,
    week_total_tokens: u64,
    month_input_tokens: u64,
    month_output_tokens: u64,
    month_total_tokens: u64,
    request_count_today: u64,
    request_count_week: u64,
    request_count_month: u64,
}

/// API Key Token统计详情响应
#[derive(Debug, Serialize)]
struct ApiKeyTokenStatsResponse {
    api_key_id: String,
    window: String,
    summary: ApiKeyTokenSummary,
    hourly_breakdown: Vec<HourlyTokenStats>,
    quota: Option<TokenQuotaInfo>,
}

/// Route Token统计详情响应
#[derive(Debug, Serialize)]
struct RouteTokenStatsResponse {
    route_id: String,
    window: String,
    summary: RouteTokenSummary,
    hourly_breakdown: Vec<HourlyTokenStats>,
}

/// 小时级Token统计
#[derive(Debug, Serialize)]
struct HourlyTokenStats {
    timestamp: u64,
    input_tokens: u64,
    output_tokens: u64,
    request_count: u64,
}

/// Token配额信息
#[derive(Debug, Serialize)]
struct TokenQuotaInfo {
    daily_total_limit: Option<u64>,
    daily_input_limit: Option<u64>,
    daily_output_limit: Option<u64>,
    weekly_total_limit: Option<u64>,
    weekly_input_limit: Option<u64>,
    weekly_output_limit: Option<u64>,
    daily_used_total: u64,
    daily_used_input: u64,
    daily_used_output: u64,
    weekly_used_total: u64,
    weekly_used_input: u64,
    weekly_used_output: u64,
}

/// Token统计汇总
async fn admin_token_stats_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TokenStatsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let token_stats = match &state.observability.token_stats {
        Some(stats) => stats,
        None => {
            return json_ok(&TokenStatsSummaryResponse {
                generated_at_unix_ms: current_unix_ms(),
                window: query.window.clone(),
                api_keys: vec![],
                routes: vec![],
                time_series: vec![],
            });
        }
    };

    let mut api_keys = Vec::new();
    let mut routes = Vec::new();

    // 加载配置以获取API Key的实际key值
    let config = state.config_path.as_ref()
        .and_then(|path| crate::config::AppConfig::load_from_file(path).ok());

    // 获取所有API Key统计
    for (api_key_id, summary) in token_stats.get_all_api_key_stats() {
        // 从配置中查找实际的API Key值
        let api_key = config.as_ref()
            .and_then(|c| c.api_keys.as_ref())
            .and_then(|ak| ak.keys.iter().find(|k| k.id == api_key_id))
            .map(|k| k.key.clone())
            .unwrap_or_else(|| api_key_id.clone());

        api_keys.push(ApiKeyTokenSummary {
            api_key_id,
            api_key,
            today_input_tokens: summary.today_input,
            today_output_tokens: summary.today_output,
            today_total_tokens: summary.today_total,
            week_input_tokens: summary.week_input,
            week_output_tokens: summary.week_output,
            week_total_tokens: summary.week_total,
            month_input_tokens: summary.month_input,
            month_output_tokens: summary.month_output,
            month_total_tokens: summary.month_total,
            request_count_today: summary.request_count_today,
            request_count_week: summary.request_count_week,
            request_count_month: summary.request_count_month,
        });
    }

    // 获取所有Route统计
    for (route_id, summary) in token_stats.get_all_route_stats() {
        routes.push(RouteTokenSummary {
            route_id,
            today_input_tokens: summary.today_input,
            today_output_tokens: summary.today_output,
            today_total_tokens: summary.today_total,
            week_input_tokens: summary.week_input,
            week_output_tokens: summary.week_output,
            week_total_tokens: summary.week_total,
            month_input_tokens: summary.month_input,
            month_output_tokens: summary.month_output,
            month_total_tokens: summary.month_total,
            request_count_today: summary.request_count_today,
            request_count_week: summary.request_count_week,
            request_count_month: summary.request_count_month,
        });
    }

    // 生成时间序列数据
    let time_series = if let Some(storage) = token_stats.storage() {
        let window_enum = match query.window.as_str() {
            "day" => crate::token_stats_storage::TimeWindow::Day,
            "week" => crate::token_stats_storage::TimeWindow::Week,
            "month" => crate::token_stats_storage::TimeWindow::Month,
            _ => crate::token_stats_storage::TimeWindow::Day,
        };

        match storage.query_time_series(window_enum).await {
            Ok(rows) => rows
                .into_iter()
                .map(|row| {
                    // 后端只返回原始时间戳，前端根据用户时区格式化
                    TimeSeriesDataPoint {
                        timestamp: row.time_bucket as u64,
                        label: String::new(), // 前端根据时区生成
                        input_tokens: row.input_tokens,
                        output_tokens: row.output_tokens,
                        request_count: row.request_count,
                    }
                })
                .collect(),
            Err(e) => {
                tracing::warn!("Failed to query time series: {}", e);
                vec![]
            }
        }
    } else {
        vec![]
    };

    json_ok(&TokenStatsSummaryResponse {
        generated_at_unix_ms: current_unix_ms(),
        window: query.window.clone(),
        api_keys,
        routes,
        time_series,
    })
}

/// 列出所有API Key的Token统计
async fn admin_list_api_key_token_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<TokenStatsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let token_stats = match &state.observability.token_stats {
        Some(stats) => stats,
        None => return json_ok(&serde_json::json!({"api_keys": []})),
    };

    // 加载配置以获取API Key的实际key值
    let config = state.config_path.as_ref()
        .and_then(|path| crate::config::AppConfig::load_from_file(path).ok());

    let api_keys: Vec<ApiKeyTokenSummary> = token_stats
        .get_all_api_key_stats()
        .into_iter()
        .map(|(api_key_id, summary)| {
            // 从配置中查找实际的API Key值
            let api_key = config.as_ref()
                .and_then(|c| c.api_keys.as_ref())
                .and_then(|ak| ak.keys.iter().find(|k| k.id == api_key_id))
                .map(|k| k.key.clone())
                .unwrap_or_else(|| api_key_id.clone());

            ApiKeyTokenSummary {
                api_key_id,
                api_key,
                today_input_tokens: summary.today_input,
                today_output_tokens: summary.today_output,
                today_total_tokens: summary.today_total,
                week_input_tokens: summary.week_input,
                week_output_tokens: summary.week_output,
                week_total_tokens: summary.week_total,
                month_input_tokens: summary.month_input,
                month_output_tokens: summary.month_output,
                month_total_tokens: summary.month_total,
                request_count_today: summary.request_count_today,
                request_count_week: summary.request_count_week,
                request_count_month: summary.request_count_month,
            }
        })
        .collect();

    json_ok(&serde_json::json!({ "api_keys": api_keys }))
}

/// 获取单个API Key的Token统计详情
async fn admin_get_api_key_token_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenStatsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let _ = query; // query参数当前未使用，但保留用于未来扩展

    let token_stats = match &state.observability.token_stats {
        Some(stats) => stats,
        None => return json_error(StatusCode::NOT_FOUND, "token_stats_not_available"),
    };

    let summary = match token_stats.get_api_key_summary(&id) {
        Some(s) => s,
        None => return json_error(StatusCode::NOT_FOUND, "api_key_not_found"),
    };

    // 获取配额信息
    let quota = state.observability.token_quota_manager.as_ref().and_then(|qm| {
        let check_result = qm.check_quota(&id);
        Some(TokenQuotaInfo {
            daily_total_limit: check_result.daily_limit_total,
            daily_input_limit: check_result.daily_limit_input,
            daily_output_limit: check_result.daily_limit_output,
            weekly_total_limit: check_result.weekly_limit_total,
            weekly_input_limit: check_result.weekly_limit_input,
            weekly_output_limit: check_result.weekly_limit_output,
            daily_used_total: check_result.daily_used_total,
            daily_used_input: check_result.daily_used_input,
            daily_used_output: check_result.daily_used_output,
            weekly_used_total: check_result.weekly_used_total,
            weekly_used_input: check_result.weekly_used_input,
            weekly_used_output: check_result.weekly_used_output,
        })
    });

    // 加载配置以获取API Key的实际key值
    let config = state.config_path.as_ref()
        .and_then(|path| crate::config::AppConfig::load_from_file(path).ok());

    // 从配置中查找实际的API Key值
    let api_key = config.as_ref()
        .and_then(|c| c.api_keys.as_ref())
        .and_then(|ak| ak.keys.iter().find(|k| k.id == id))
        .map(|k| k.key.clone())
        .unwrap_or_else(|| id.clone());

    let response = ApiKeyTokenStatsResponse {
        api_key_id: id,
        window: query.window.clone(),
        summary: ApiKeyTokenSummary {
            api_key_id: String::new(),
            api_key,
            today_input_tokens: summary.today_input,
            today_output_tokens: summary.today_output,
            today_total_tokens: summary.today_total,
            week_input_tokens: summary.week_input,
            week_output_tokens: summary.week_output,
            week_total_tokens: summary.week_total,
            month_input_tokens: summary.month_input,
            month_output_tokens: summary.month_output,
            month_total_tokens: summary.month_total,
            request_count_today: summary.request_count_today,
            request_count_week: summary.request_count_week,
            request_count_month: summary.request_count_month,
        },
        hourly_breakdown: vec![], // TODO: 从SQLite查询详细数据
        quota,
    };

    json_ok(&response)
}

/// 列出所有Route的Token统计
async fn admin_list_route_token_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<TokenStatsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let token_stats = match &state.observability.token_stats {
        Some(stats) => stats,
        None => return json_ok(&serde_json::json!({"routes": []})),
    };

    let routes: Vec<RouteTokenSummary> = token_stats
        .get_all_route_stats()
        .into_iter()
        .map(|(route_id, summary)| RouteTokenSummary {
            route_id,
            today_input_tokens: summary.today_input,
            today_output_tokens: summary.today_output,
            today_total_tokens: summary.today_total,
            week_input_tokens: summary.week_input,
            week_output_tokens: summary.week_output,
            week_total_tokens: summary.week_total,
            month_input_tokens: summary.month_input,
            month_output_tokens: summary.month_output,
            month_total_tokens: summary.month_total,
            request_count_today: summary.request_count_today,
            request_count_week: summary.request_count_week,
            request_count_month: summary.request_count_month,
        })
        .collect();

    json_ok(&serde_json::json!({ "routes": routes }))
}

/// 获取单个Route的Token统计详情
async fn admin_get_route_token_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TokenStatsQuery>,
) -> Response<Body> {
    if !is_admin_authorized(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let _ = query; // query参数当前未使用，但保留用于未来扩展

    let token_stats = match &state.observability.token_stats {
        Some(stats) => stats,
        None => return json_error(StatusCode::NOT_FOUND, "token_stats_not_available"),
    };

    let summary = match token_stats.get_route_summary(&id) {
        Some(s) => s,
        None => return json_error(StatusCode::NOT_FOUND, "route_not_found"),
    };

    let response = RouteTokenStatsResponse {
        route_id: id,
        window: query.window.clone(),
        summary: RouteTokenSummary {
            route_id: String::new(),
            today_input_tokens: summary.today_input,
            today_output_tokens: summary.today_output,
            today_total_tokens: summary.today_total,
            week_input_tokens: summary.week_input,
            week_output_tokens: summary.week_output,
            week_total_tokens: summary.week_total,
            month_input_tokens: summary.month_input,
            month_output_tokens: summary.month_output,
            month_total_tokens: summary.month_total,
            request_count_today: summary.request_count_today,
            request_count_week: summary.request_count_week,
            request_count_month: summary.request_count_month,
        },
        hourly_breakdown: vec![], // TODO: 从SQLite查询详细数据
    };

    json_ok(&response)
}

/// 获取当前Unix时间戳（毫秒）
fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
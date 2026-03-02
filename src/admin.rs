use crate::config::AppConfig;
use crate::server::{AppState, build_runtime_state};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Response, StatusCode};
use axum::routing::{get, post};
use http::header::CONTENT_TYPE;
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
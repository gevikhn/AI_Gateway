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
        .route(
            &format!("{prefix}/api/config"),
            get(admin_config_get_handler).put(admin_config_apply_handler),
        )
        .route(
            &format!("{prefix}/api/config/save"),
            post(admin_config_save_handler),
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

fn admin_dashboard_html(prefix: &str) -> String {
    let api_config_url = format!("{prefix}/api/config");
    let api_save_url = format!("{prefix}/api/config/save");
    format!(
        r##"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>AI Gateway Admin</title>
  <style>
    :root {{
      --bg: #f5f7fb; --card: #ffffff; --text: #0f172a;
      --muted: #475569; --line: #dbe2ea; --accent: #0f766e;
      --danger: #b91c1c; --warning: #d97706; --success: #15803d;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
      background: radial-gradient(circle at top right, #e2f6f3, var(--bg) 55%);
      color: var(--text);
    }}
    main {{ max-width: 1000px; margin: 0 auto; padding: 18px; }}
    h1 {{ margin: 0 0 4px 0; font-size: 26px; }}
    .muted {{ color: var(--muted); font-size: 13px; }}
    .toolbar {{
      margin-top: 12px; display: flex; gap: 8px; align-items: center;
    }}
    .toolbar input {{
      flex: 1; border: 1px solid var(--line); border-radius: 8px;
      padding: 8px 12px; font-size: 14px;
    }}
    button {{
      border: 0; border-radius: 8px; padding: 8px 14px;
      font-weight: 600; cursor: pointer; font-size: 13px;
    }}
    .btn-primary {{ background: var(--accent); color: #fff; }}
    .btn-secondary {{ background: #e2e8f0; color: var(--text); }}
    .btn-danger {{ background: var(--danger); color: #fff; }}
    .btn-small {{ padding: 4px 10px; font-size: 12px; }}
    .status {{ margin-top: 8px; min-height: 20px; font-size: 13px; }}
    .status.error {{ color: var(--danger); }}
    .status.success {{ color: var(--success); }}

    .tabs {{
      margin-top: 14px; display: flex; gap: 2px;
      border-bottom: 2px solid var(--line);
    }}
    .tab {{
      background: transparent; color: var(--muted); border: none;
      padding: 8px 16px; border-radius: 8px 8px 0 0; font-size: 13px;
    }}
    .tab.active {{ background: var(--card); color: var(--text); border: 1px solid var(--line); border-bottom: 2px solid var(--card); margin-bottom: -2px; }}
    .tab-panel {{ display: none; background: var(--card); border: 1px solid var(--line); border-top: none; border-radius: 0 0 12px 12px; padding: 16px; }}
    .tab-panel.active {{ display: block; }}

    .restart-badge {{
      display: inline-block; background: var(--warning); color: #fff;
      font-size: 10px; padding: 1px 5px; border-radius: 4px; margin-left: 4px;
      vertical-align: middle;
    }}
    .field {{ margin-bottom: 12px; }}
    .field label {{ display: block; font-size: 12px; color: var(--muted); margin-bottom: 3px; font-weight: 600; }}
    .field input, .field select, .field textarea {{
      width: 100%; border: 1px solid var(--line); border-radius: 6px;
      padding: 6px 10px; font-size: 13px; font-family: inherit;
    }}
    .field textarea {{ min-height: 60px; resize: vertical; font-family: "Cascadia Code", "Fira Code", monospace; }}
    .field input:read-only {{ background: #f1f5f9; color: var(--muted); }}

    .route-card {{
      border: 1px solid var(--line); border-radius: 10px; padding: 12px;
      margin-bottom: 10px; background: #fafbfd;
    }}
    .route-card .route-header {{
      display: flex; justify-content: space-between; align-items: center;
      margin-bottom: 8px;
    }}
    .route-card .route-header strong {{ font-size: 14px; }}
    .route-fields {{ display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }}
    .route-fields .full {{ grid-column: 1 / -1; }}

    .actions {{
      margin-top: 16px; display: flex; gap: 10px; justify-content: flex-end;
    }}

    .pair-list {{ margin-top: 4px; }}
    .pair-row {{ display: flex; gap: 4px; margin-bottom: 4px; align-items: center; }}
    .pair-row input {{ flex: 1; }}
    .pair-row button {{ flex-shrink: 0; }}

    .token-list {{ margin-top: 4px; }}
    .token-row {{ display: flex; gap: 4px; margin-bottom: 4px; align-items: center; }}
    .token-row input {{ flex: 1; }}

    .checkbox-field {{ display: flex; align-items: center; gap: 6px; margin-bottom: 8px; }}
    .checkbox-field input {{ width: auto; }}
    .checkbox-field label {{ margin-bottom: 0; }}
  </style>
</head>
<body>
  <main>
    <h1>Gateway Admin</h1>
    <div class="muted">管理网关配置，修改后点击"应用"即时生效，点击"保存到文件"持久化。</div>

    <div class="toolbar">
      <input id="token" type="password" placeholder="输入 Admin Token" />
      <button class="btn-primary" onclick="loadConfig()">加载配置</button>
    </div>
    <div id="status" class="status muted">输入 Admin Token 后点击加载</div>

    <nav class="tabs" id="tabNav"></nav>
    <div id="tabPanels"></div>

    <div class="actions" id="actionBar" style="display:none;">
      <button class="btn-primary" onclick="applyConfig()">应用 (即时生效)</button>
      <button class="btn-secondary" onclick="saveConfig()">保存到文件</button>
    </div>
  </main>

<script>
const CONFIG_URL = {api_config_url:?};
const SAVE_URL = {api_save_url:?};
const TOKEN_KEY = "ai_gw_admin_token";
let cfg = null;

const tokenInput = document.getElementById("token");
tokenInput.value = localStorage.getItem(TOKEN_KEY) || "";

function getToken() {{
  const t = tokenInput.value.trim();
  if (t) localStorage.setItem(TOKEN_KEY, t);
  return t;
}}

function setStatus(text, type) {{
  const el = document.getElementById("status");
  el.textContent = text;
  el.className = "status " + (type || "muted");
}}

// ---- Tab System ----
const TABS = [
  {{ id: "routes", label: "Routes" }},
  {{ id: "auth", label: "Auth" }},
  {{ id: "cors", label: "CORS" }},
  {{ id: "ratelimit", label: "Rate Limit" }},
  {{ id: "concurrency", label: "Concurrency" }},
  {{ id: "advanced", label: "Advanced", badge: "需重启" }},
];

function initTabs() {{
  const nav = document.getElementById("tabNav");
  const panels = document.getElementById("tabPanels");
  nav.innerHTML = "";
  panels.innerHTML = "";
  TABS.forEach((tab, i) => {{
    const btn = document.createElement("button");
    btn.className = "tab" + (i === 0 ? " active" : "");
    btn.dataset.tab = tab.id;
    btn.innerHTML = tab.label + (tab.badge ? ` <span class="restart-badge">${{tab.badge}}</span>` : "");
    btn.onclick = () => switchTab(tab.id);
    nav.appendChild(btn);

    const panel = document.createElement("div");
    panel.id = "tab-" + tab.id;
    panel.className = "tab-panel" + (i === 0 ? " active" : "");
    panels.appendChild(panel);
  }});
}}
initTabs();

function switchTab(id) {{
  document.querySelectorAll(".tab").forEach(t => t.classList.toggle("active", t.dataset.tab === id));
  document.querySelectorAll(".tab-panel").forEach(p => p.classList.toggle("active", p.id === "tab-" + id));
}}

// ---- Load Config ----
async function loadConfig() {{
  const token = getToken();
  if (!token) {{ setStatus("请先填写 Admin Token", "error"); return; }}
  try {{
    const res = await fetch(CONFIG_URL, {{ headers: {{ Authorization: "Bearer " + token }}, cache: "no-store" }});
    if (!res.ok) {{ setStatus("加载失败: HTTP " + res.status, "error"); return; }}
    cfg = await res.json();
    renderAll();
    document.getElementById("actionBar").style.display = "flex";
    setStatus("配置已加载", "success");
  }} catch (e) {{
    setStatus("加载失败: " + e, "error");
  }}
}}

// ---- Render Functions ----
function renderAll() {{
  renderRoutes();
  renderAuth();
  renderCors();
  renderRateLimit();
  renderConcurrency();
  renderAdvanced();
}}

// -- Routes --
function renderRoutes() {{
  const panel = document.getElementById("tab-routes");
  let html = '<div style="margin-bottom:10px;"><button class="btn-secondary btn-small" onclick="addRoute()">+ 添加路由</button></div>';
  html += '<div id="routeList">';
  (cfg.routes || []).forEach((r, i) => {{
    html += routeCardHtml(r, i);
  }});
  html += '</div>';
  panel.innerHTML = html;
}}

function routeCardHtml(r, i) {{
  const u = r.upstream || {{}};
  const injectHeaders = (u.inject_headers || []).map(h => `${{h.name}}: ${{h.value}}`).join("\n");
  const removeHeaders = (u.remove_headers || []).join("\n");
  const proxy = u.proxy || {{}};
  return `<div class="route-card" data-idx="${{i}}">
    <div class="route-header">
      <strong>#${{i + 1}}: ${{r.id || "(new)"}}</strong>
      <button class="btn-danger btn-small" onclick="removeRoute(${{i}})">删除</button>
    </div>
    <div class="route-fields">
      <div class="field"><label>ID</label><input value="${{esc(r.id)}}" onchange="cfg.routes[${{i}}].id=this.value"/></div>
      <div class="field"><label>Prefix</label><input value="${{esc(r.prefix)}}" onchange="cfg.routes[${{i}}].prefix=this.value"/></div>
      <div class="field"><label>Base URL</label><input value="${{esc(u.base_url)}}" onchange="cfg.routes[${{i}}].upstream.base_url=this.value"/></div>
      <div class="field">
        <label>Strip Prefix</label>
        <select onchange="cfg.routes[${{i}}].upstream.strip_prefix=(this.value==='true')">
          <option value="true" ${{u.strip_prefix !== false ? 'selected' : ''}}>true</option>
          <option value="false" ${{u.strip_prefix === false ? 'selected' : ''}}>false</option>
        </select>
      </div>
      <div class="field"><label>Connect Timeout (ms)</label><input type="number" value="${{u.connect_timeout_ms || 10000}}" onchange="cfg.routes[${{i}}].upstream.connect_timeout_ms=+this.value"/></div>
      <div class="field"><label>Request Timeout (ms)</label><input type="number" value="${{u.request_timeout_ms || 60000}}" onchange="cfg.routes[${{i}}].upstream.request_timeout_ms=+this.value"/></div>
      <div class="field full">
        <label>Inject Headers (每行一个, 格式: name: value)</label>
        <textarea onchange="parseInjectHeaders(${{i}}, this.value)">${{esc(injectHeaders)}}</textarea>
      </div>
      <div class="field full">
        <label>Remove Headers (每行一个)</label>
        <textarea onchange="parseRemoveHeaders(${{i}}, this.value)">${{esc(removeHeaders)}}</textarea>
      </div>
      <div class="checkbox-field">
        <input type="checkbox" id="xff_${{i}}" ${{u.forward_xff ? 'checked' : ''}} onchange="cfg.routes[${{i}}].upstream.forward_xff=this.checked"/>
        <label for="xff_${{i}}">Forward X-Forwarded-For</label>
      </div>
      <div class="field"><label>Proxy Protocol</label>
        <select onchange="setProxyField(${{i}}, 'protocol', this.value)">
          <option value="">(无代理)</option>
          <option value="http" ${{proxy.protocol==='http'?'selected':''}}>http</option>
          <option value="https" ${{proxy.protocol==='https'?'selected':''}}>https</option>
          <option value="socks" ${{proxy.protocol==='socks'?'selected':''}}>socks</option>
        </select>
      </div>
      <div class="field"><label>Proxy Address</label><input value="${{esc(proxy.address||'')}}" onchange="setProxyField(${{i}}, 'address', this.value)"/></div>
      <div class="field"><label>Upstream Key Max Inflight</label><input type="number" value="${{u.upstream_key_max_inflight||''}}" placeholder="(未设置)" onchange="cfg.routes[${{i}}].upstream.upstream_key_max_inflight=this.value?+this.value:null"/></div>
    </div>
  </div>`;
}}

function parseInjectHeaders(i, text) {{
  cfg.routes[i].upstream.inject_headers = text.split("\n").map(l => l.trim()).filter(l => l).map(l => {{
    const idx = l.indexOf(":");
    if (idx < 0) return {{ name: l, value: "" }};
    return {{ name: l.substring(0, idx).trim(), value: l.substring(idx + 1).trim() }};
  }});
}}

function parseRemoveHeaders(i, text) {{
  cfg.routes[i].upstream.remove_headers = text.split("\n").map(l => l.trim()).filter(l => l);
}}

function setProxyField(i, field, value) {{
  if (field === 'protocol' && !value) {{
    cfg.routes[i].upstream.proxy = null;
    return;
  }}
  if (!cfg.routes[i].upstream.proxy) {{
    cfg.routes[i].upstream.proxy = {{ protocol: "http", address: "" }};
  }}
  cfg.routes[i].upstream.proxy[field] = value;
}}

function addRoute() {{
  if (!cfg.routes) cfg.routes = [];
  cfg.routes.push({{
    id: "", prefix: "/",
    upstream: {{
      base_url: "", strip_prefix: true, connect_timeout_ms: 10000,
      request_timeout_ms: 60000, inject_headers: [], remove_headers: [],
      forward_xff: false, proxy: null, upstream_key_max_inflight: null
    }}
  }});
  renderRoutes();
}}

function removeRoute(i) {{
  cfg.routes.splice(i, 1);
  renderRoutes();
}}

// -- Auth --
function renderAuth() {{
  const panel = document.getElementById("tab-auth");
  const auth = cfg.gateway_auth || {{ tokens: [], token_sources: [] }};
  let tokensHtml = (auth.tokens || []).map((t, i) =>
    `<div class="token-row">
      <input value="${{esc(t)}}" onchange="cfg.gateway_auth.tokens[${{i}}]=this.value"/>
      <button class="btn-danger btn-small" onclick="cfg.gateway_auth.tokens.splice(${{i}},1);renderAuth()">删除</button>
    </div>`
  ).join("");

  let srcHtml = (auth.token_sources || []).map((s, i) => {{
    if (s.type === "authorization_bearer") return `<div class="token-row"><input value="authorization_bearer" readonly/><button class="btn-danger btn-small" onclick="cfg.gateway_auth.token_sources.splice(${{i}},1);renderAuth()">删除</button></div>`;
    return `<div class="token-row"><input value="header: ${{esc(s.name||'')}}" onchange="parseTokenSource(${{i}}, this.value)"/><button class="btn-danger btn-small" onclick="cfg.gateway_auth.token_sources.splice(${{i}},1);renderAuth()">删除</button></div>`;
  }}).join("");

  panel.innerHTML = `
    <div class="field"><label>Gateway Tokens</label>
      <div class="token-list">${{tokensHtml}}</div>
      <button class="btn-secondary btn-small" style="margin-top:4px" onclick="cfg.gateway_auth.tokens.push('');renderAuth()">+ 添加 Token</button>
    </div>
    <div class="field"><label>Token Sources</label>
      <div class="token-list">${{srcHtml}}</div>
      <div style="display:flex;gap:4px;margin-top:4px">
        <button class="btn-secondary btn-small" onclick="cfg.gateway_auth.token_sources.push({{type:'authorization_bearer'}});renderAuth()">+ Bearer</button>
        <button class="btn-secondary btn-small" onclick="cfg.gateway_auth.token_sources.push({{type:'header',name:'x-gw-token'}});renderAuth()">+ Header</button>
      </div>
    </div>
  `;
}}

function parseTokenSource(i, value) {{
  if (value.startsWith("header:")) {{
    cfg.gateway_auth.token_sources[i] = {{ type: "header", name: value.substring(7).trim() }};
  }}
}}

// -- CORS --
function renderCors() {{
  const panel = document.getElementById("tab-cors");
  const cors = cfg.cors || {{ enabled: false, allow_origins: [], allow_headers: [], allow_methods: [], expose_headers: [] }};
  if (!cfg.cors) cfg.cors = cors;

  panel.innerHTML = `
    <div class="checkbox-field">
      <input type="checkbox" id="cors_enabled" ${{cors.enabled?'checked':''}} onchange="cfg.cors.enabled=this.checked"/>
      <label for="cors_enabled">启用 CORS</label>
    </div>
    <div class="field"><label>Allow Origins (每行一个, * 代表全部)</label>
      <textarea onchange="cfg.cors.allow_origins=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${{esc((cors.allow_origins||[]).join("\n"))}}</textarea>
    </div>
    <div class="field"><label>Allow Headers (每行一个)</label>
      <textarea onchange="cfg.cors.allow_headers=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${{esc((cors.allow_headers||[]).join("\n"))}}</textarea>
    </div>
    <div class="field"><label>Allow Methods (每行一个)</label>
      <textarea onchange="cfg.cors.allow_methods=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${{esc((cors.allow_methods||[]).join("\n"))}}</textarea>
    </div>
    <div class="field"><label>Expose Headers (每行一个)</label>
      <textarea onchange="cfg.cors.expose_headers=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${{esc((cors.expose_headers||[]).join("\n"))}}</textarea>
    </div>
  `;
}}

// -- Rate Limit --
function renderRateLimit() {{
  const panel = document.getElementById("tab-ratelimit");
  const rl = cfg.rate_limit;
  const enabled = !!rl;
  const perMinute = rl ? rl.per_minute : 120;

  panel.innerHTML = `
    <div class="checkbox-field">
      <input type="checkbox" id="rl_enabled" ${{enabled?'checked':''}} onchange="toggleRateLimit(this.checked)"/>
      <label for="rl_enabled">启用限流</label>
    </div>
    <div class="field"><label>Per Minute (每 token+route)</label>
      <input type="number" value="${{perMinute}}" onchange="if(cfg.rate_limit) cfg.rate_limit.per_minute=+this.value"/>
    </div>
  `;
}}

function toggleRateLimit(enabled) {{
  if (enabled) {{
    cfg.rate_limit = {{ per_minute: 120 }};
  }} else {{
    cfg.rate_limit = null;
  }}
  renderRateLimit();
}}

// -- Concurrency --
function renderConcurrency() {{
  const panel = document.getElementById("tab-concurrency");
  const cc = cfg.concurrency;
  const enabled = !!cc;
  const ds = cc ? cc.downstream_max_inflight : null;
  const us = cc ? cc.upstream_per_key_max_inflight : null;

  panel.innerHTML = `
    <div class="checkbox-field">
      <input type="checkbox" id="cc_enabled" ${{enabled?'checked':''}} onchange="toggleConcurrency(this.checked)"/>
      <label for="cc_enabled">启用并发控制</label>
    </div>
    <div class="field"><label>Downstream Max Inflight (全局入站上限)</label>
      <input type="number" value="${{ds||''}}" placeholder="(未设置)" onchange="if(cfg.concurrency) cfg.concurrency.downstream_max_inflight=this.value?+this.value:null"/>
    </div>
    <div class="field"><label>Upstream Per-Key Max Inflight</label>
      <input type="number" value="${{us||''}}" placeholder="(未设置)" onchange="if(cfg.concurrency) cfg.concurrency.upstream_per_key_max_inflight=this.value?+this.value:null"/>
    </div>
  `;
}}

function toggleConcurrency(enabled) {{
  if (enabled) {{
    cfg.concurrency = {{ downstream_max_inflight: null, upstream_per_key_max_inflight: null }};
  }} else {{
    cfg.concurrency = null;
  }}
  renderConcurrency();
}}

// -- Advanced (read-only indicators) --
function renderAdvanced() {{
  const panel = document.getElementById("tab-advanced");
  panel.innerHTML = `
    <div class="muted" style="margin-bottom:12px">以下配置修改后需要重启服务才能生效。可在此查看当前值。</div>
    <div class="field"><label>Listen Address</label><input value="${{esc(cfg.listen||'')}}" readonly/></div>
    <div class="field"><label>Inbound TLS</label><input value="${{cfg.inbound_tls ? '已配置' : '未配置'}}" readonly/></div>
    <div class="field"><label>Observability - Log Level</label><input value="${{esc(cfg.observability?.logging?.level||'info')}}" readonly/></div>
    <div class="field"><label>Observability - Metrics</label><input value="${{cfg.observability?.metrics?.enabled ? '已启用 (' + (cfg.observability?.metrics?.path||'/metrics') + ')' : '未启用'}}" readonly/></div>
    <div class="field"><label>Observability - Tracing</label><input value="${{cfg.observability?.tracing?.enabled ? '已启用 (采样率: ' + (cfg.observability?.tracing?.sample_ratio||0.05) + ')' : '未启用'}}" readonly/></div>
    <div class="field"><label>Admin</label><input value="${{cfg.admin?.enabled ? '已启用 (' + (cfg.admin?.path_prefix||'/admin') + ')' : '未启用'}}" readonly/></div>
  `;
}}

// ---- Apply & Save ----
async function applyConfig() {{
  const token = getToken();
  if (!token) {{ setStatus("请先填写 Admin Token", "error"); return; }}
  if (!cfg) {{ setStatus("请先加载配置", "error"); return; }}
  try {{
    setStatus("正在应用配置...", "muted");
    const res = await fetch(CONFIG_URL, {{
      method: "PUT",
      headers: {{ Authorization: "Bearer " + token, "Content-Type": "application/json" }},
      body: JSON.stringify(cfg)
    }});
    const data = await res.json();
    if (!res.ok) {{
      setStatus("应用失败: " + (data.error || res.status), "error");
      return;
    }}
    setStatus("配置已应用，即时生效！", "success");
  }} catch (e) {{
    setStatus("应用失败: " + e, "error");
  }}
}}

async function saveConfig() {{
  const token = getToken();
  if (!token) {{ setStatus("请先填写 Admin Token", "error"); return; }}
  try {{
    setStatus("正在保存到文件...", "muted");
    const res = await fetch(SAVE_URL, {{
      method: "POST",
      headers: {{ Authorization: "Bearer " + token }}
    }});
    const data = await res.json();
    if (!res.ok) {{
      setStatus("保存失败: " + (data.error || res.status), "error");
      return;
    }}
    setStatus("配置已保存到 " + (data.path || "文件"), "success");
  }} catch (e) {{
    setStatus("保存失败: " + e, "error");
  }}
}}

function esc(s) {{ return (s||"").replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;"); }}
</script>
</body>
</html>
"##
    )
}

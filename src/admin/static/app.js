// AI Gateway Admin UI - JavaScript

const CONFIG = {
  apiUrl: window.CONFIG?.apiUrl || '{{API_CONFIG_URL}}',
  saveUrl: window.CONFIG?.saveUrl || '{{API_SAVE_URL}}',
  metricsUrl: window.CONFIG?.metricsUrl || '{{API_METRICS_URL}}',
  adminPrefix: window.CONFIG?.adminPrefix || '/admin'
};

const TOKEN_KEY = 'ai_gateway_admin_token';
let cfg = null;
let metricsData = null;
let loadingStates = new Map();

// ===== Toast 通知系统 =====
class Toast {
  static container = null;

  static init() {
    if (!this.container) {
      this.container = document.createElement('div');
      this.container.className = 'toast-container';
      document.body.appendChild(this.container);
    }
  }

  static show(message, type = 'info', duration = 3000) {
    this.init();

    const icons = {
      success: '<svg class="toast-icon" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2"><path d="M20 6 9 17l-5-5"/></svg>',
      error: '<svg class="toast-icon" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="m15 9-6 6M9 9l6 6"/></svg>',
      warning: '<svg class="toast-icon" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>',
      info: '<svg class="toast-icon" viewBox="0 0 24 24" fill="none" stroke="#3b82f6" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/></svg>'
    };

    const titles = {
      success: '成功',
      error: '错误',
      warning: '警告',
      info: '提示'
    };

    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.innerHTML = `
      ${icons[type]}
      <div class="toast-content">
        <div class="toast-title">${titles[type]}</div>
        <div class="toast-message">${message}</div>
      </div>
      <button class="toast-close" onclick="this.parentElement.remove()">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M18 6 6 18M6 6l12 12"/>
        </svg>
      </button>
    `;

    this.container.appendChild(toast);

    if (duration > 0) {
      setTimeout(() => {
        toast.style.animation = 'slideIn 0.3s ease reverse';
        setTimeout(() => toast.remove(), 300);
      }, duration);
    }

    return toast;
  }
}

// ===== 加载状态管理 =====
class LoadingState {
  constructor(button) {
    this.button = button;
    this.originalText = button.innerHTML;
  }

  start() {
    this.button.disabled = true;
    this.button.classList.add('btn-loading');
  }

  stop() {
    this.button.disabled = false;
    this.button.classList.remove('btn-loading');
  }
}

// ===== 确认对话框 =====
function confirmDelete(title, message, onConfirm) {
  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.innerHTML = `
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-header">
        <h3 class="modal-title">${title}</h3>
        <button class="btn btn-ghost btn-sm" onclick="this.closest('.modal-overlay').remove()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="modal-body">
        <p style="color: var(--text-secondary);">${message}</p>
      </div>
      <div class="modal-footer">
        <button class="btn btn-secondary" onclick="this.closest('.modal-overlay').remove()">取消</button>
        <button class="btn btn-danger" id="confirm-btn">删除</button>
      </div>
    </div>
  `;

  document.body.appendChild(modal);

  modal.querySelector('#confirm-btn').addEventListener('click', () => {
    onConfirm();
    modal.remove();
  });

  modal.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      modal.remove();
    }
  });

  modal.addEventListener('click', (e) => {
    if (e.target === modal) {
      modal.remove();
    }
  });

  modal.querySelector('button').focus();
}

// ===== 表单验证 =====
class FormValidator {
  constructor(form) {
    this.form = form;
    this.fields = form.querySelectorAll('[data-validate]');
    this.init();
  }

  init() {
    this.fields.forEach(field => {
      field.addEventListener('blur', () => this.validateField(field));
      field.addEventListener('input', () => this.clearError(field));
    });
  }

  validateField(field) {
    const rules = field.dataset.validate.split(',');
    const value = field.value.trim();

    for (const rule of rules) {
      const [ruleName, param] = rule.split(':');
      const error = this.checkRule(ruleName, value, param, field);

      if (error) {
        this.showError(field, error);
        return false;
      }
    }

    this.showSuccess(field);
    return true;
  }

  checkRule(rule, value, param, field) {
    switch (rule) {
      case 'required':
        return value ? null : '此字段为必填项';
      case 'url':
        return !value || /^https?:\/\/.+/.test(value) ? null : '请输入有效的URL';
      case 'number':
        return !value || !isNaN(value) ? null : '请输入数字';
      case 'min':
        return !value || Number(value) >= Number(param) ? null : `最小值为 ${param}`;
      default:
        return null;
    }
  }

  showError(field, message) {
    field.classList.add('input-error');
    field.classList.remove('input-success');

    let errorEl = field.parentElement.querySelector('.field-error');
    if (!errorEl) {
      errorEl = document.createElement('div');
      errorEl.className = 'field-error';
      field.parentElement.appendChild(errorEl);
    }
    errorEl.textContent = message;
  }

  showSuccess(field) {
    field.classList.remove('input-error');
    field.classList.add('input-success');

    const errorEl = field.parentElement.querySelector('.field-error');
    if (errorEl) {
      errorEl.remove();
    }
  }

  clearError(field) {
    field.classList.remove('input-error');
    const errorEl = field.parentElement.querySelector('.field-error');
    if (errorEl) {
      errorEl.remove();
    }
  }
}

// ===== Token 管理 =====
function getToken() {
  return localStorage.getItem(TOKEN_KEY);
}

function logout() {
  localStorage.removeItem(TOKEN_KEY);
  window.location.href = CONFIG.adminPrefix + '/login';
}

// 检查登录状态
function checkAuth() {
  const token = getToken();
  if (!token) {
    // 未登录，跳转到登录页
    window.location.href = CONFIG.adminPrefix + '/login';
    return false;
  }
  return true;
}

// ===== 标签页系统 =====
const TABS = [
  { id: 'routes', label: '路由配置' },
  { id: 'metrics', label: '监控' },
  { id: 'auth', label: '认证' },
  { id: 'cors', label: 'CORS' },
  { id: 'ratelimit', label: '限流' },
  { id: 'concurrency', label: '并发控制' },
  { id: 'advanced', label: '高级', badge: '需重启' }
];

function initTabs() {
  const nav = document.getElementById('tabNav');
  const panels = document.getElementById('tabPanels');
  if (!nav || !panels) return;

  nav.innerHTML = '';
  panels.innerHTML = '';

  TABS.forEach((tab, i) => {
    const btn = document.createElement('button');
    btn.className = 'tab' + (i === 0 ? ' active' : '');
    btn.dataset.tab = tab.id;
    btn.innerHTML = tab.label + (tab.badge ? ` <span class="badge badge-warning">${tab.badge}</span>` : '');
    btn.onclick = () => switchTab(tab.id);
    nav.appendChild(btn);

    const panel = document.createElement('div');
    panel.id = 'tab-' + tab.id;
    panel.className = 'tab-panel' + (i === 0 ? ' active' : '');
    panels.appendChild(panel);
  });
}

function switchTab(id) {
  document.querySelectorAll('.tab').forEach(t => {
    t.classList.toggle('active', t.dataset.tab === id);
  });
  document.querySelectorAll('.tab-panel').forEach(p => {
    p.classList.toggle('active', p.id === 'tab-' + id);
  });
}

// ===== 加载配置 =====
async function loadConfig() {
  const token = getToken();
  if (!token) {
    logout();
    return;
  }

  try {
    const res = await fetch(CONFIG.apiUrl, {
      headers: { Authorization: 'Bearer ' + token },
      cache: 'no-store'
    });

    if (res.status === 401) {
      // Token 无效，清除并跳转
      localStorage.removeItem(TOKEN_KEY);
      logout();
      return;
    }

    if (!res.ok) {
      throw new Error('HTTP ' + res.status);
    }

    cfg = await res.json();
    renderAll();
    const actionBar = document.getElementById('actionBar');
    if (actionBar) actionBar.style.display = '';
    Toast.show('配置已加载', 'success');
  } catch (e) {
    Toast.show('加载失败: ' + e.message, 'error');
  }
}


// ===== 渲染函数 =====
function renderAll() {
  renderRoutes();
  renderMetrics();
  renderAuth();
  renderCors();
  renderRateLimit();
  renderConcurrency();
  renderAdvanced();
}

// -- Routes --
let selectedRouteIndex = 0;

function renderRoutes() {
  const panel = document.getElementById('tab-routes');
  if (!panel) return;

  if (!cfg.routes || cfg.routes.length === 0) {
    panel.innerHTML = `
      <div class="empty-state">
        <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M4 6h16M4 12h16M4 18h16"/>
        </svg>
        <div class="empty-state-title">暂无路由</div>
        <div class="empty-state-description">点击下方按钮添加第一个路由配置</div>
        <button class="btn btn-primary" style="margin-top: var(--space-4);" onclick="addRoute()">+ 添加路由</button>
      </div>
    `;
    return;
  }

  // 确保选中索引有效
  if (selectedRouteIndex >= cfg.routes.length) {
    selectedRouteIndex = cfg.routes.length - 1;
  }

  panel.innerHTML = `
    <div class="route-editor">
      <div class="route-sidebar">
        <div class="route-sidebar-header">
          <span class="route-count">${cfg.routes.length} 个路由</span>
          <button class="btn btn-primary btn-sm" onclick="addRoute()">+ 添加</button>
        </div>
        <div class="route-list-vertical">
          ${cfg.routes.map((r, i) => `
            <div class="route-list-item ${i === selectedRouteIndex ? 'active' : ''}" data-idx="${i}" onclick="selectRoute(${i})">
              <div class="route-item-icon">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
                </svg>
              </div>
              <div class="route-item-info">
                <div class="route-item-name">${esc(r.id) || '(未命名)'}</div>
                <div class="route-item-prefix">${esc(r.prefix) || '/'}</div>
              </div>
              <button class="btn btn-ghost btn-sm route-item-delete" onclick="event.stopPropagation(); confirmDeleteRoute(${i})" title="删除">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M18 6 6 18M6 6l12 12"/>
                </svg>
              </button>
            </div>
          `).join('')}
        </div>
      </div>
      <div class="route-detail-panel">
        ${routeDetailHtml(cfg.routes[selectedRouteIndex], selectedRouteIndex)}
      </div>
    </div>
  `;
}

function selectRoute(index) {
  selectedRouteIndex = index;
  renderRoutes();
}

function routeDetailHtml(r, i) {
  const u = r.upstream || {};
  const injectHeaders = (u.inject_headers || []).map(h => `${h.name}: ${h.value}`).join('\n');
  const removeHeaders = (u.remove_headers || []).join('\n');
  const proxy = u.proxy || {};

  return `
    <div class="route-detail-header">
      <h3 class="route-detail-title">${esc(r.id) || '新路由'}</h3>
      <div class="route-detail-actions">
        <button class="btn btn-danger btn-sm" onclick="confirmDeleteRoute(${i})">删除</button>
      </div>
    </div>
    <div class="route-detail-body">
      <div class="route-fields">
        <!-- 基础配置 -->
        <div class="field">
          <label class="field-label">路由 ID <span class="required">*</span></label>
          <input class="input" value="${esc(r.id)}" data-validate="required" onchange="cfg.routes[${i}].id=this.value; updateRouteItemName(${i}, this.value)" />
        </div>
        <div class="field">
          <label class="field-label">路径前缀 <span class="required">*</span></label>
          <input class="input" value="${esc(r.prefix)}" placeholder="/api" data-validate="required" onchange="cfg.routes[${i}].prefix=this.value; updateRouteItemPrefix(${i}, this.value)" />
        </div>
        <div class="field full-width">
          <label class="field-label">上游服务地址 <span class="required">*</span></label>
          <input class="input" value="${esc(u.base_url)}" placeholder="https://api.example.com" data-validate="required,url" onchange="cfg.routes[${i}].upstream.base_url=this.value" />
        </div>

        <!-- 超时配置 -->
        <div class="field">
          <label class="field-label">连接超时 (ms)</label>
          <input class="input" type="number" value="${u.connect_timeout_ms || 10000}" min="1000" step="1000" onchange="cfg.routes[${i}].upstream.connect_timeout_ms=+this.value" />
        </div>
        <div class="field">
          <label class="field-label">请求超时 (ms)</label>
          <input class="input" type="number" value="${u.request_timeout_ms || 60000}" min="1000" step="1000" onchange="cfg.routes[${i}].upstream.request_timeout_ms=+this.value" />
        </div>

        <!-- 选项开关 -->
        <div class="field">
          <label class="field-label">移除路径前缀</label>
          <label class="toggle">
            <input type="checkbox" class="toggle-input" ${u.strip_prefix !== false ? 'checked' : ''} onchange="cfg.routes[${i}].upstream.strip_prefix=this.checked" />
            <span class="toggle-slider" aria-hidden="true"></span>
            <span class="toggle-label">${u.strip_prefix !== false ? '启用' : '禁用'}</span>
          </label>
        </div>
        <div class="field">
          <label class="field-label">转发 XFF</label>
          <label class="toggle">
            <input type="checkbox" class="toggle-input" ${u.forward_xff ? 'checked' : ''} onchange="cfg.routes[${i}].upstream.forward_xff=this.checked" />
            <span class="toggle-slider" aria-hidden="true"></span>
            <span class="toggle-label">${u.forward_xff ? '启用' : '禁用'}</span>
          </label>
        </div>

        <!-- 高级配置 -->
        <div class="field">
          <label class="field-label">并发限制</label>
          <input class="input" type="number" value="${u.upstream_key_max_inflight || ''}" placeholder="不限" min="1" onchange="cfg.routes[${i}].upstream.upstream_key_max_inflight=this.value?+this.value:null" />
        </div>
        <div class="field">
          <label class="field-label">User-Agent</label>
          <input class="input" value="${esc(u.user_agent || '')}" placeholder="默认" onchange="cfg.routes[${i}].upstream.user_agent=this.value.trim()?this.value:null" />
        </div>

        <!-- 代理配置 -->
        <div class="field">
          <label class="field-label">代理协议</label>
          <select class="input select" onchange="setProxyField(${i}, 'protocol', this.value)">
            <option value="">无</option>
            <option value="http" ${proxy.protocol === 'http' ? 'selected' : ''}>HTTP</option>
            <option value="https" ${proxy.protocol === 'https' ? 'selected' : ''}>HTTPS</option>
            <option value="socks" ${proxy.protocol === 'socks' ? 'selected' : ''}>SOCKS</option>
          </select>
        </div>
        <div class="field">
          <label class="field-label">代理地址</label>
          <input class="input" value="${esc(proxy.address || '')}" placeholder="host:port" onchange="setProxyField(${i}, 'address', this.value)" />
        </div>

        <!-- Headers 配置 -->
        <div class="field full-width">
          <label class="field-label">注入请求头</label>
          <textarea class="input textarea" rows="3" placeholder="Authorization: Bearer xxx" onchange="parseInjectHeaders(${i}, this.value)">${esc(injectHeaders)}</textarea>
          <div class="field-help">每行一个，格式: Name: Value</div>
        </div>
        <div class="field full-width">
          <label class="field-label">移除响应头</label>
          <textarea class="input textarea" rows="2" placeholder="X-Internal-Header" onchange="parseRemoveHeaders(${i}, this.value)">${esc(removeHeaders)}</textarea>
          <div class="field-help">每行一个头部名称</div>
        </div>
      </div>
    </div>
  `;
}

function updateRouteItemName(index, value) {
  const item = document.querySelector(`.route-list-item[data-idx="${index}"] .route-item-name`);
  if (item) {
    item.textContent = value || '(未命名)';
  }
}

function updateRouteItemPrefix(index, value) {
  const item = document.querySelector(`.route-list-item[data-idx="${index}"] .route-item-prefix`);
  if (item) {
    item.textContent = value || '/';
  }
}

function confirmDeleteRoute(index) {
  confirmDelete(
    '删除路由',
    `确定要删除路由 "${esc(cfg.routes[index].id || '(未命名)')}" 吗？此操作不可撤销。`,
    () => {
      cfg.routes.splice(index, 1);
      renderRoutes();
      Toast.show('路由已删除', 'success');
    }
  );
}

function parseInjectHeaders(i, text) {
  cfg.routes[i].upstream.inject_headers = text.split('\n').map(l => l.trim()).filter(l => l).map(l => {
    const idx = l.indexOf(':');
    if (idx < 0) return { name: l, value: '' };
    return { name: l.substring(0, idx).trim(), value: l.substring(idx + 1).trim() };
  });
}

function parseRemoveHeaders(i, text) {
  cfg.routes[i].upstream.remove_headers = text.split('\n').map(l => l.trim()).filter(l => l);
}

function setProxyField(i, field, value) {
  if (field === 'protocol' && !value) {
    cfg.routes[i].upstream.proxy = null;
    return;
  }
  if (!cfg.routes[i].upstream.proxy) {
    cfg.routes[i].upstream.proxy = { protocol: 'http', address: '' };
  }
  cfg.routes[i].upstream.proxy[field] = value;
}

function addRoute() {
  if (!cfg.routes) cfg.routes = [];
  cfg.routes.push({
    id: '',
    prefix: '/',
    upstream: {
      base_url: '',
      strip_prefix: true,
      connect_timeout_ms: 10000,
      request_timeout_ms: 60000,
      inject_headers: [],
      remove_headers: [],
      forward_xff: false,
      proxy: null,
      upstream_key_max_inflight: null,
      user_agent: null
    }
  });
  selectedRouteIndex = cfg.routes.length - 1;
  renderRoutes();
  Toast.show('新路由已添加', 'info');
}

// -- Auth --
function renderAuth() {
  const panel = document.getElementById('tab-auth');
  if (!panel) return;

  const auth = cfg.gateway_auth || { tokens: [], token_sources: [] };

  let tokensHtml = (auth.tokens || []).map((t, i) => `
    <div class="token-row">
      <input class="input" type="password" value="${esc(t)}" onchange="cfg.gateway_auth.tokens[${i}]=this.value" />
      <button class="btn btn-danger btn-sm" onclick="cfg.gateway_auth.tokens.splice(${i},1);renderAuth();Toast.show('Token已删除','success')">删除</button>
    </div>
  `).join('');

  let srcHtml = (auth.token_sources || []).map((s, i) => {
    if (s.type === 'authorization_bearer') {
      return `<div class="token-row">
        <input class="input" value="Authorization Bearer" readonly />
        <button class="btn btn-danger btn-sm" onclick="cfg.gateway_auth.token_sources.splice(${i},1);renderAuth()">删除</button>
      </div>`;
    }
    return `<div class="token-row">
      <input class="input" value="Header: ${esc(s.name || '')}" onchange="parseTokenSource(${i}, this.value)" />
      <button class="btn btn-danger btn-sm" onclick="cfg.gateway_auth.token_sources.splice(${i},1);renderAuth()">删除</button>
    </div>`;
  }).join('');

  panel.innerHTML = `
    <div class="field">
      <label class="field-label">Gateway Tokens</label>
      <div class="token-list">${tokensHtml}</div>
      <button class="btn btn-secondary btn-sm" style="margin-top:var(--space-2)" onclick="cfg.gateway_auth.tokens.push('');renderAuth()">+ 添加 Token</button>
    </div>
    <div class="field">
      <label class="field-label">Token Sources</label>
      <div class="token-list">${srcHtml}</div>
      <div style="display:flex;gap:var(--space-2);margin-top:var(--space-2)">
        <button class="btn btn-secondary btn-sm" onclick="cfg.gateway_auth.token_sources.push({type:'authorization_bearer'});renderAuth()">+ Authorization Bearer</button>
        <button class="btn btn-secondary btn-sm" onclick="cfg.gateway_auth.token_sources.push({type:'header',name:'x-gw-token'});renderAuth()">+ 自定义 Header</button>
      </div>
    </div>
  `;
}

function parseTokenSource(i, value) {
  if (value.startsWith('Header:') || value.startsWith('header:')) {
    cfg.gateway_auth.token_sources[i] = { type: 'header', name: value.substring(7).trim() };
  }
}

// -- CORS --
function renderCors() {
  const panel = document.getElementById('tab-cors');
  if (!panel) return;

  const cors = cfg.cors || { enabled: false, allow_origins: [], allow_headers: [], allow_methods: [], expose_headers: [] };
  if (!cfg.cors) cfg.cors = cors;

  panel.innerHTML = `
    <div class="field">
      <label class="toggle">
        <input type="checkbox" class="toggle-input" ${cors.enabled ? 'checked' : ''} onchange="cfg.cors.enabled=this.checked" />
        <span class="toggle-slider" aria-hidden="true"></span>
        <span class="toggle-label">启用 CORS</span>
      </label>
    </div>
    <div class="field">
      <label class="field-label">Allow Origins (每行一个, * 代表全部)</label>
      <textarea class="input textarea" rows="3" onchange="cfg.cors.allow_origins=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${esc((cors.allow_origins || []).join('\n'))}</textarea>
    </div>
    <div class="field">
      <label class="field-label">Allow Headers (每行一个)</label>
      <textarea class="input textarea" rows="3" onchange="cfg.cors.allow_headers=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${esc((cors.allow_headers || []).join('\n'))}</textarea>
    </div>
    <div class="field">
      <label class="field-label">Allow Methods (每行一个)</label>
      <textarea class="input textarea" rows="2" onchange="cfg.cors.allow_methods=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${esc((cors.allow_methods || []).join('\n'))}</textarea>
    </div>
    <div class="field">
      <label class="field-label">Expose Headers (每行一个)</label>
      <textarea class="input textarea" rows="2" onchange="cfg.cors.expose_headers=this.value.split('\\n').map(s=>s.trim()).filter(s=>s)">${esc((cors.expose_headers || []).join('\n'))}</textarea>
    </div>
  `;
}

// -- Rate Limit --
function renderRateLimit() {
  const panel = document.getElementById('tab-ratelimit');
  if (!panel) return;

  const rl = cfg.rate_limit;
  const enabled = !!rl;
  const perMinute = rl ? rl.per_minute : 120;

  panel.innerHTML = `
    <div class="field">
      <label class="toggle">
        <input type="checkbox" class="toggle-input" ${enabled ? 'checked' : ''} onchange="toggleRateLimit(this.checked)" />
        <span class="toggle-slider" aria-hidden="true"></span>
        <span class="toggle-label">启用限流</span>
      </label>
    </div>
    <div class="field">
      <label class="field-label">Per Minute (每 token+route)</label>
      <input class="input" type="number" value="${perMinute}" min="1" ${enabled ? '' : 'disabled'} onchange="if(cfg.rate_limit) cfg.rate_limit.per_minute=+this.value" />
    </div>
  `;
}

function toggleRateLimit(enabled) {
  if (enabled) {
    cfg.rate_limit = { per_minute: 120 };
  } else {
    cfg.rate_limit = null;
  }
  renderRateLimit();
}

// -- Concurrency --
function renderConcurrency() {
  const panel = document.getElementById('tab-concurrency');
  if (!panel) return;

  const cc = cfg.concurrency;
  const enabled = !!cc;
  const ds = cc ? cc.downstream_max_inflight : null;
  const us = cc ? cc.upstream_per_key_max_inflight : null;

  panel.innerHTML = `
    <div class="field">
      <label class="toggle">
        <input type="checkbox" class="toggle-input" ${enabled ? 'checked' : ''} onchange="toggleConcurrency(this.checked)" />
        <span class="toggle-slider" aria-hidden="true"></span>
        <span class="toggle-label">启用并发控制</span>
      </label>
    </div>
    <div class="field">
      <label class="field-label">Downstream Max Inflight (全局入站上限)</label>
      <input class="input" type="number" value="${ds || ''}" placeholder="无限制" min="1" ${enabled ? '' : 'disabled'} onchange="if(cfg.concurrency) cfg.concurrency.downstream_max_inflight=this.value?+this.value:null" />
    </div>
    <div class="field">
      <label class="field-label">Upstream Per-Key Max Inflight</label>
      <input class="input" type="number" value="${us || ''}" placeholder="无限制" min="1" ${enabled ? '' : 'disabled'} onchange="if(cfg.concurrency) cfg.concurrency.upstream_per_key_max_inflight=this.value?+this.value:null" />
    </div>
  `;
}

function toggleConcurrency(enabled) {
  if (enabled) {
    cfg.concurrency = { downstream_max_inflight: null, upstream_per_key_max_inflight: null };
  } else {
    cfg.concurrency = null;
  }
  renderConcurrency();
}

// -- Advanced --
function renderAdvanced() {
  const panel = document.getElementById('tab-advanced');
  if (!panel) return;

  const formatValue = (val, unit = '') => {
    if (val === null || val === undefined) return '<span style="color:var(--text-tertiary)">未设置</span>';
    return `<code>${val}${unit}</code>`;
  };

  const formatBoolean = (val) => {
    if (val === true) return '<span style="color:var(--success-600)">✓ 已启用</span>';
    if (val === false) return '<span style="color:var(--text-tertiary)">✗ 未启用</span>';
    return '<span style="color:var(--text-tertiary)">未设置</span>';
  };

  panel.innerHTML = `
    <div class="empty-state" style="padding: var(--space-4);">
      <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="10"/>
        <path d="M12 16v-4"/><path d="M12 8h.01"/>
      </svg>
      <div class="empty-state-title">高级配置</div>
      <div class="empty-state-description">以下配置需要重启服务才能生效</div>
    </div>

    <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:var(--space-4);">
      <div class="field">
        <label class="field-label">监听地址</label>
        <input class="input" value="${esc(cfg.listen || '')}" readonly />
      </div>
      <div class="field">
        <label class="field-label">入站 TLS</label>
        <div class="input" style="background:var(--bg-tertiary);display:flex;align-items:center;height:38px;">
          ${formatBoolean(cfg.inbound_tls ? true : false)}
        </div>
      </div>
      <div class="field">
        <label class="field-label">日志级别</label>
        <input class="input" value="${esc(cfg.observability?.logging?.level || 'info')}" readonly />
      </div>
      <div class="field">
        <label class="field-label">Metrics</label>
        <div class="input" style="background:var(--bg-tertiary);display:flex;align-items:center;height:38px;">
          ${cfg.observability?.metrics?.enabled ?
            `<span style="color:var(--success-600)">✓ 已启用 (${esc(cfg.observability?.metrics?.path || '/metrics')})</span>` :
            '<span style="color:var(--text-tertiary)">✗ 未启用</span>'}
        </div>
      </div>
      <div class="field">
        <label class="field-label">Tracing</label>
        <div class="input" style="background:var(--bg-tertiary);display:flex;align-items:center;height:38px;">
          ${cfg.observability?.tracing?.enabled ?
            `<span style="color:var(--success-600)">✓ 已启用 (采样率: ${cfg.observability?.tracing?.sample_ratio || 0.05})</span>` :
            '<span style="color:var(--text-tertiary)">✗ 未启用</span>'}
        </div>
      </div>
      <div class="field">
        <label class="field-label">Admin</label>
        <div class="input" style="background:var(--bg-tertiary);display:flex;align-items:center;height:38px;">
          ${cfg.admin?.enabled ?
            `<span style="color:var(--success-600)">✓ 已启用 (${esc(cfg.admin?.path_prefix || '/admin')})</span>` :
            '<span style="color:var(--text-tertiary)">✗ 未启用</span>'}
        </div>
      </div>
    </div>
  `;
}

// ===== 应用和保存 =====
async function applyConfig() {
  const token = getToken();
  if (!token) {
    logout();
    return;
  }
  if (!cfg) {
    Toast.show('请先加载配置', 'error');
    return;
  }

  const btn = document.querySelector('button[onclick="applyConfig()"]');
  const loading = new LoadingState(btn);
  loading.start();

  try {
    const res = await fetch(CONFIG.apiUrl, {
      method: 'PUT',
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: JSON.stringify(cfg)
    });
    const data = await res.json();

    if (!res.ok) {
      throw new Error(data.error || res.status);
    }

    Toast.show('配置已应用，即时生效！', 'success');
  } catch (e) {
    Toast.show('应用失败: ' + e.message, 'error');
  } finally {
    loading.stop();
  }
}

async function saveConfig() {
  const token = getToken();
  if (!token) {
    logout();
    return;
  }

  const btn = document.querySelector('button[onclick="saveConfig()"]');
  const loading = new LoadingState(btn);
  loading.start();

  try {
    const res = await fetch(CONFIG.saveUrl, {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + token }
    });
    const data = await res.json();

    if (!res.ok) {
      throw new Error(data.error || res.status);
    }

    // 更新保存状态显示
    const saveStatus = document.getElementById('saveStatus');
    if (saveStatus) {
      const now = new Date();
      saveStatus.textContent = `已保存 ${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}`;
    }
    Toast.show('配置已保存', 'success');
  } catch (e) {
    Toast.show('保存失败: ' + e.message, 'error');
  } finally {
    loading.stop();
  }
}

// ===== 工具函数 =====
function esc(s) {
  return (s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// ===== Metrics =====
let metricsRefreshInterval = null;
let ipMetricsData = null;
let ipMetricsWindow = '1h';
let ipMetricsSearch = '';
let ipMetricsSortBy = 'requests';
let ipMetricsOrder = 'desc';

async function loadMetrics() {
  const token = getToken();
  if (!token) {
    logout();
    return;
  }

  try {
    const res = await fetch(CONFIG.metricsUrl, {
      headers: { Authorization: 'Bearer ' + token },
      cache: 'no-store'
    });

    if (res.status === 401) {
      localStorage.removeItem(TOKEN_KEY);
      logout();
      return;
    }

    if (!res.ok) {
      throw new Error('HTTP ' + res.status);
    }

    metricsData = await res.json();
    renderMetrics();
  } catch (e) {
    console.error('加载 metrics 失败:', e);
  }
}

async function loadIPMetrics() {
  const token = getToken();
  if (!token) {
    logout();
    return;
  }

  try {
    const params = new URLSearchParams({
      window: ipMetricsWindow,
      sort_by: ipMetricsSortBy,
      order: ipMetricsOrder,
      limit: '100'
    });
    if (ipMetricsSearch) {
      params.append('ip', ipMetricsSearch);
    }

    const res = await fetch(`${CONFIG.metricsUrl}/ip?${params}`, {
      headers: { Authorization: 'Bearer ' + token },
      cache: 'no-store'
    });

    if (res.status === 401) {
      localStorage.removeItem(TOKEN_KEY);
      logout();
      return;
    }

    if (!res.ok) {
      throw new Error('HTTP ' + res.status);
    }

    ipMetricsData = await res.json();
    renderIPMetrics();
  } catch (e) {
    console.error('加载 IP metrics 失败:', e);
  }
}

function setIPMetricsWindow(window) {
  ipMetricsWindow = window;
  loadIPMetrics();
}

function setIPMetricsSearch(value) {
  ipMetricsSearch = value.trim();
  loadIPMetrics();
}

function setIPMetricsSort(sortBy) {
  if (ipMetricsSortBy === sortBy) {
    ipMetricsOrder = ipMetricsOrder === 'desc' ? 'asc' : 'desc';
  } else {
    ipMetricsSortBy = sortBy;
    ipMetricsOrder = 'desc';
  }
  loadIPMetrics();
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

function formatDuration(ms) {
  if (ms < 1) return '<1ms';
  if (ms < 1000) return Math.round(ms) + 'ms';
  return (ms / 1000).toFixed(2) + 's';
}

function renderIPMetrics() {
  const container = document.getElementById('ip-metrics-container');
  if (!container) return;

  const windows = [
    { key: '5m', label: '5分钟' },
    { key: '1h', label: '1小时' },
    { key: '24h', label: '24小时' },
    { key: '1w', label: '1周' },
    { key: '1m', label: '1月' }
  ];

  const windowButtons = windows.map(w => `
    <button class="metrics-window-btn ${ipMetricsWindow === w.key ? 'active' : ''}" onclick="setIPMetricsWindow('${w.key}')">${w.label}</button>
  `).join('');

  const sortIndicators = {
    requests: ipMetricsSortBy === 'requests' ? (ipMetricsOrder === 'desc' ? '↓' : '↑') : '',
    errors: ipMetricsSortBy === 'errors' ? (ipMetricsOrder === 'desc' ? '↓' : '↑') : '',
    bytes_in: ipMetricsSortBy === 'bytes_in' ? (ipMetricsOrder === 'desc' ? '↓' : '↑') : '',
    bytes_out: ipMetricsSortBy === 'bytes_out' ? (ipMetricsOrder === 'desc' ? '↓' : '↑') : '',
    latency_avg: ipMetricsSortBy === 'latency_avg' ? (ipMetricsOrder === 'desc' ? '↓' : '↑') : ''
  };

  let tableContent = '';
  if (!ipMetricsData || !ipMetricsData.ips || ipMetricsData.ips.length === 0) {
    tableContent = `<tr><td colspan="8" class="empty-cell">暂无 IP 数据</td></tr>`;
  } else {
    tableContent = ipMetricsData.ips.map(ip => `
      <tr>
        <td><code class="ip-address">${esc(ip.ip)}</code></td>
        <td>${formatNumber(ip.requests)}</td>
        <td>${formatNumber(ip.errors)}</td>
        <td>${formatBytes(ip.bytes_in)}</td>
        <td>${formatBytes(ip.bytes_out)}</td>
        <td>${formatDuration(ip.latency_avg_ms)}</td>
        <td>${formatDuration(ip.latency_p99_ms)}</td>
        <td>
          <div class="ip-routes">
            ${(ip.routes || []).slice(0, 3).map(r => `<span class="ip-tag route">${esc(r)}</span>`).join('')}
            ${(ip.routes || []).length > 3 ? `<span class="ip-tag more">+${(ip.routes || []).length - 3}</span>` : ''}
          </div>
        </td>
      </tr>
    `).join('');
  }

  const summary = ipMetricsData ? `
    <div class="ip-metrics-summary">
      <span class="summary-item">IP 数: <strong>${ipMetricsData.total_ips || 0}</strong></span>
      <span class="summary-item">总请求: <strong>${formatNumber(ipMetricsData.total_requests || 0)}</strong></span>
      <span class="summary-item">总错误: <strong>${formatNumber(ipMetricsData.total_errors || 0)}</strong></span>
    </div>
  ` : '';

  container.innerHTML = `
    <div class="ip-metrics-header">
      <div class="metrics-window-selector">
        ${windowButtons}
      </div>
      <div class="ip-metrics-search">
        <input type="text" class="input" placeholder="搜索 IP..." value="${esc(ipMetricsSearch)}" onchange="setIPMetricsSearch(this.value)" />
        <svg class="search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"/>
          <path d="m21 21-4.3-4.3"/>
        </svg>
      </div>
    </div>
    ${summary}
    <div class="metrics-table-wrapper">
      <table class="metrics-table ip-metrics-table">
        <thead>
          <tr>
            <th onclick="setIPMetricsSort('ip')">IP 地址</th>
            <th onclick="setIPMetricsSort('requests')" class="sortable">请求数 ${sortIndicators.requests}</th>
            <th onclick="setIPMetricsSort('errors')" class="sortable">错误数 ${sortIndicators.errors}</th>
            <th onclick="setIPMetricsSort('bytes_in')" class="sortable">入流量 ${sortIndicators.bytes_in}</th>
            <th onclick="setIPMetricsSort('bytes_out')" class="sortable">出流量 ${sortIndicators.bytes_out}</th>
            <th onclick="setIPMetricsSort('latency_avg')" class="sortable">平均延迟 ${sortIndicators.latency_avg}</th>
            <th>P99 延迟</th>
            <th>访问路由</th>
          </tr>
        </thead>
        <tbody>${tableContent}</tbody>
      </table>
    </div>
  `;
}

function renderMetrics() {
  const panel = document.getElementById('tab-metrics');
  if (!panel) return;

  if (!metricsData) {
    panel.innerHTML = `
      <div class="empty-state">
        <svg class="empty-state-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M3 3v18h18"/>
          <path d="M18 17V9"/>
          <path d="M13 17V5"/>
          <path d="M8 17v-3"/>
        </svg>
        <div class="empty-state-title">加载中...</div>
        <div class="empty-state-description">正在获取监控数据</div>
      </div>
    `;
    return;
  }

  const total1h = formatNumber(metricsData.total_requests_1h || 0);
  const total24h = formatNumber(metricsData.total_requests_24h || 0);
  const routeCount = metricsData.routes?.length || 0;
  const tokenCount = metricsData.tokens?.length || 0;

  const routeRows = (metricsData.routes || []).map(r => `
    <tr>
      <td>${esc(r.route_id)}</td>
      <td>${formatNumber(r.requests_1h)}</td>
      <td>${formatNumber(r.requests_24h)}</td>
      <td>${r.inflight_current}</td>
      <td>${r.inflight_peak_1h}</td>
      <td>${r.inflight_peak_24h}</td>
    </tr>
  `).join('');

  const tokenRows = (metricsData.tokens || []).map(t => `
    <tr>
      <td>${esc(t.token)}</td>
      <td>${formatNumber(t.requests_1h)}</td>
      <td>${formatNumber(t.requests_24h)}</td>
    </tr>
  `).join('');

  const generatedAt = metricsData.generated_at_unix_ms
    ? new Date(metricsData.generated_at_unix_ms).toLocaleString()
    : '-';

  // 准备图表数据
  const routeChartData = (metricsData.routes || [])
    .sort((a, b) => b.requests_24h - a.requests_24h)
    .slice(0, 10);

  const tokenChartData = (metricsData.tokens || [])
    .sort((a, b) => b.requests_24h - a.requests_24h)
    .slice(0, 8);

  panel.innerHTML = `
    <div class="metrics-header">
      <div class="metrics-title">
        <h2>实时监控</h2>
        <span class="metrics-time">生成时间: ${generatedAt}</span>
      </div>
      <button class="btn btn-primary btn-sm" onclick="loadMetrics()">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
          <path d="M3 3v5h5"/>
          <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/>
          <path d="M16 21h5v-5"/>
        </svg>
        刷新
      </button>
    </div>

    <div class="metrics-charts">
      <div class="chart-container">
        <h3 class="chart-title">Top 10 路由请求量 (24h)</h3>
        <canvas id="routeChart"></canvas>
      </div>
      <div class="chart-container">
        <h3 class="chart-title">Token 使用量分布 (24h)</h3>
        <canvas id="tokenChart"></canvas>
      </div>
    </div>

    <div class="metrics-cards">
      <div class="metric-card">
        <div class="metric-label">总请求 (1小时)</div>
        <div class="metric-value">${total1h}</div>
      </div>
      <div class="metric-card">
        <div class="metric-label">总请求 (24小时)</div>
        <div class="metric-value">${total24h}</div>
      </div>
      <div class="metric-card">
        <div class="metric-label">路由数</div>
        <div class="metric-value">${routeCount}</div>
      </div>
      <div class="metric-card">
        <div class="metric-label">Token 数</div>
        <div class="metric-value">${tokenCount}</div>
      </div>
    </div>

    <div class="metrics-section">
      <h3>路由维度</h3>
      <div class="metrics-table-wrapper">
        <table class="metrics-table">
          <thead>
            <tr>
              <th>Route</th>
              <th>请求 (1h)</th>
              <th>请求 (24h)</th>
              <th>当前并发</th>
              <th>并发峰值 (1h)</th>
              <th>并发峰值 (24h)</th>
            </tr>
          </thead>
          <tbody>${routeRows || '<tr><td colspan="6" class="empty-cell">暂无数据</td></tr>'}</tbody>
        </table>
      </div>
    </div>

    <div class="metrics-section">
      <h3>Token 维度</h3>
      <div class="metrics-table-wrapper">
        <table class="metrics-table">
          <thead>
            <tr>
              <th>Token</th>
              <th>请求 (1h)</th>
              <th>请求 (24h)</th>
            </tr>
          </thead>
          <tbody>${tokenRows || '<tr><td colspan="3" class="empty-cell">暂无数据</td></tr>'}</tbody>
        </table>
      </div>
    </div>

    <div class="metrics-section">
      <h3>IP 维度</h3>
      <div id="ip-metrics-container">
        <div class="empty-state">
          <div class="empty-state-description">加载中...</div>
        </div>
      </div>
    </div>
  `;

  // 加载并渲染 IP 维度数据
  loadIPMetrics();

  // 初始化图表
  initCharts(routeChartData, tokenChartData);
}

// ===== 图表功能 =====
let routeChart = null;
let tokenChart = null;

function initCharts(routeData, tokenData) {
  // 销毁旧图表
  if (routeChart) {
    routeChart.destroy();
  }
  if (tokenChart) {
    tokenChart.destroy();
  }

  // 路由柱状图
  const routeCtx = document.getElementById('routeChart');
  if (routeCtx && routeData.length > 0) {
    routeChart = new Chart(routeCtx, {
      type: 'bar',
      data: {
        labels: routeData.map(r => r.route_id),
        datasets: [{
          label: '24小时请求数',
          data: routeData.map(r => r.requests_24h),
          backgroundColor: 'rgba(20, 184, 166, 0.8)',
          borderColor: 'rgba(20, 184, 166, 1)',
          borderWidth: 1,
          borderRadius: 4
        }]
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        plugins: {
          legend: {
            display: false
          }
        },
        scales: {
          y: {
            beginAtZero: true,
            grid: {
              color: 'rgba(0, 0, 0, 0.05)'
            }
          },
          x: {
            grid: {
              display: false
            },
            ticks: {
              maxRotation: 45,
              minRotation: 45
            }
          }
        }
      }
    });
  }

  // Token 饼图
  const tokenCtx = document.getElementById('tokenChart');
  if (tokenCtx && tokenData.length > 0) {
    const colors = [
      'rgba(20, 184, 166, 0.8)',
      'rgba(59, 130, 246, 0.8)',
      'rgba(245, 158, 11, 0.8)',
      'rgba(239, 68, 68, 0.8)',
      'rgba(139, 92, 246, 0.8)',
      'rgba(236, 72, 153, 0.8)',
      'rgba(99, 102, 241, 0.8)',
      'rgba(16, 185, 129, 0.8)'
    ];

    tokenChart = new Chart(tokenCtx, {
      type: 'doughnut',
      data: {
        labels: tokenData.map(t => t.token.substring(0, 8) + '...'),
        datasets: [{
          data: tokenData.map(t => t.requests_24h),
          backgroundColor: colors.slice(0, tokenData.length),
          borderWidth: 2,
          borderColor: '#ffffff'
        }]
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        plugins: {
          legend: {
            position: 'right',
            labels: {
              boxWidth: 12,
              padding: 10,
              font: {
                size: 11
              }
            }
          }
        }
      }
    });
  }
}

function formatNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return n.toString();
}

// ===== 初始化 =====
document.addEventListener('DOMContentLoaded', () => {
  // 检查登录状态
  if (!checkAuth()) {
    return;
  }

  initTabs();

  // 自动加载配置和监控数据
  loadConfig();
  loadMetrics();

  // 监控页面自动刷新
  setInterval(() => {
    const metricsPanel = document.getElementById('tab-metrics');
    if (metricsPanel && metricsPanel.classList.contains('active')) {
      loadMetrics();
      loadIPMetrics();
    }
  }, 30000); // 30秒刷新一次
});

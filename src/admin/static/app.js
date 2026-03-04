// AI Gateway Admin UI - JavaScript

const CONFIG = {
  apiUrl: window.CONFIG?.apiUrl || '{{API_CONFIG_URL}}',
  saveUrl: window.CONFIG?.saveUrl || '{{API_SAVE_URL}}',
  metricsUrl: window.CONFIG?.metricsUrl || '{{API_METRICS_URL}}',
  adminPrefix: window.CONFIG?.adminPrefix || '/admin'
};

/**
 * 验证 route ID 是否已存在
 * @param {string} id - 要验证的 ID
 * @param {number} excludeIndex - 排除的索引（编辑时使用）
 * @returns {boolean}
 */
function isRouteIdExists(id, excludeIndex = -1) {
  if (!cfg?.routes) return false;
  const routes = cfg.routes;
  // 处理 routes 是数组的情况（新配置结构）
  if (Array.isArray(routes)) {
    return routes.some((r, idx) => r.id === id && idx !== excludeIndex);
  }
  return false;
}

/**
 * 验证 apikey ID 是否已存在
 * @param {string} id - 要验证的 ID
 * @param {string} excludeId - 排除的 ID（编辑时使用）
 * @returns {boolean}
 */
function isApiKeyIdExists(id, excludeId = null) {
  if (!cfg?.api_keys?.keys) return false;
  return cfg.api_keys.keys.some(k => k.id === id && k.id !== excludeId);
}

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
  { id: 'apikeys', label: 'API Keys' },
  { id: 'tokenstats', label: 'Token统计' },
  { id: 'banrules', label: '封禁规则' },
  { id: 'banlogs', label: '封禁日志' },
  { id: 'metrics', label: '监控' },
  { id: 'gateway', label: '网关配置' },
  { id: 'advanced', label: '高级', badge: '需重启' }
];

// ===== API Key 管理数据 =====
let apiKeysData = [];
let banLogsData = [];
let apiKeyFilter = {
  route: '',
  status: 'all', // all, enabled, disabled, banned
  search: ''
};
let banLogFilter = {
  apiKey: '',
  actionType: 'all', // all, ban, unban
  startDate: '',
  endDate: ''
};
let banLogPagination = {
  page: 1,
  pageSize: 20,
  total: 0
};

// 从配置加载 API Keys（适配后端数据结构 - 架构设计 v2）
function loadApiKeysFromConfig() {
  if (!cfg || !cfg.api_keys || !cfg.api_keys.keys) {
    return [];
  }

  return cfg.api_keys.keys.map(keyConfig => {
    // 处理 route_ids（优先使用，兼容 route_id）
    const routeIds = keyConfig.route_ids || (keyConfig.route_id ? [keyConfig.route_id] : []);
    const routeNames = routeIds.map(rid => cfg.routes?.find(r => r.id === rid)?.name || rid);

    // 封禁状态处理
    const banStatus = keyConfig.ban_status || {};

    // Token配额配置
    const tokenQuota = keyConfig.token_quota || {};

    return {
      id: keyConfig.id,
      key: keyConfig.key,
      route_ids: routeIds,
      route_names: routeNames,
      enabled: keyConfig.enabled,
      remark: keyConfig.remark || '',
      // 限流配置
      per_minute: keyConfig.rate_limit?.per_minute || 120,
      // 并发配置（新结构：max_inflight）
      max_inflight: keyConfig.concurrency?.max_inflight || null,
      // 封禁状态（新结构）
      ban_status: {
        is_banned: banStatus.is_banned || false,
        banned_at: banStatus.banned_at || null,
        banned_until: banStatus.banned_until || null,
        triggered_rule_id: banStatus.triggered_rule_id || null,
        reason: banStatus.reason || null,
        ban_count: banStatus.ban_count || 0
      },
      // 封禁规则（新结构）
      ban_rules: keyConfig.ban_rules || [],
      // Token配额配置
      token_quota: {
        daily_total_limit: tokenQuota.daily_total_limit || null,
        daily_input_limit: tokenQuota.daily_input_limit || null,
        daily_output_limit: tokenQuota.daily_output_limit || null,
        weekly_total_limit: tokenQuota.weekly_total_limit || null,
        weekly_input_limit: tokenQuota.weekly_input_limit || null,
        weekly_output_limit: tokenQuota.weekly_output_limit || null
      },
      created_at: keyConfig.created_at || new Date().toISOString(),
      updated_at: keyConfig.updated_at || new Date().toISOString()
    };
  });
}

// 将前端 API Key 数据转换为后端配置格式（架构设计 v2）
function convertApiKeyToConfig(apiKey) {
  // 支持多路由：route_ids 数组
  const routeIds = apiKey.route_ids?.length > 0 ? apiKey.route_ids : null;

  // 处理Token配额配置
  const tokenQuota = apiKey.token_quota || {};
  const hasTokenQuota = tokenQuota.daily_total_limit || tokenQuota.daily_input_limit ||
                        tokenQuota.daily_output_limit || tokenQuota.weekly_total_limit ||
                        tokenQuota.weekly_input_limit || tokenQuota.weekly_output_limit;

  return {
    id: apiKey.id,
    route_ids: routeIds,
    key: apiKey.key,
    enabled: apiKey.enabled,
    remark: apiKey.remark || '',
    rate_limit: apiKey.per_minute ? { per_minute: apiKey.per_minute } : null,
    concurrency: apiKey.max_inflight ? { max_inflight: apiKey.max_inflight } : null,
    ban_rules: apiKey.ban_rules || [],
    ban_status: apiKey.ban_status || {
      is_banned: false,
      ban_count: 0
    },
    token_quota: hasTokenQuota ? {
      daily_total_limit: tokenQuota.daily_total_limit || null,
      daily_input_limit: tokenQuota.daily_input_limit || null,
      daily_output_limit: tokenQuota.daily_output_limit || null,
      weekly_total_limit: tokenQuota.weekly_total_limit || null,
      weekly_input_limit: tokenQuota.weekly_input_limit || null,
      weekly_output_limit: tokenQuota.weekly_output_limit || null
    } : null
  };
}

// 从服务器加载 API Keys（包含运行时封禁状态）
async function fetchApiKeysFromServer() {
  const token = getToken();
  if (!token || !window.CONFIG.keysUrl) return null;

  try {
    const response = await fetch(window.CONFIG.keysUrl, {
      headers: { 'Authorization': `Bearer ${token}` }
    });

    if (!response.ok) {
      if (response.status === 401) {
        logout();
        return null;
      }
      throw new Error(`HTTP ${response.status}`);
    }

    const data = await response.json();
    console.log('fetchApiKeysFromServer raw response:', data);
    // 检查第一个 key 的所有字段名（用于调试）
    if (data.keys && data.keys.length > 0) {
      console.log('First key field names:', Object.keys(data.keys[0]));
      console.log('First key ban-related fields:', {
        is_banned: data.keys[0].is_banned,
        banned_at: data.keys[0].banned_at,
        ban_expires_at: data.keys[0].ban_expires_at,
        ban_reason: data.keys[0].ban_reason,
        ban_count: data.keys[0].ban_count,
        triggered_rule_id: data.keys[0].triggered_rule_id
      });
    }
    return data.keys || [];
  } catch (err) {
    console.error('Failed to fetch API Keys:', err);
    return null;
  }
}

// 从服务器加载封禁日志
async function fetchBanLogsFromServer(apiKeyId) {
  const token = getToken();
  if (!token) {
    console.warn('No admin token found');
    return null;
  }

  // 使用 adminPrefix 构建封禁日志 URL
  const prefix = window.CONFIG?.adminPrefix || '/admin';
  const url = apiKeyId
    ? `${prefix}/api/keys/${apiKeyId}/ban-logs?limit=100`
    : `${prefix}/api/ban-logs?limit=100`;

  console.log('Fetching ban logs from:', url);

  try {
    const response = await fetch(url, {
      headers: { 'Authorization': `Bearer ${token}` }
    });

    if (!response.ok) {
      if (response.status === 401) {
        logout();
        return null;
      }
      throw new Error(`HTTP ${response.status}`);
    }

    const data = await response.json();
    console.log('Ban logs response:', data);
    return data.logs || [];
  } catch (err) {
    console.error('Failed to fetch ban logs:', err);
    return null;
  }
}

// 合并 API Key 数据（配置数据 + 服务器运行时数据）
async function loadApiKeys() {
  // 先从配置加载基础数据
  const configKeys = loadApiKeysFromConfig();

  // 从服务器获取运行时数据（包含封禁状态）
  const serverKeys = await fetchApiKeysFromServer();

  if (serverKeys && serverKeys.length > 0) {
    // 使用服务器数据，但需要补充配置中的额外字段
    apiKeysData = serverKeys.map(serverKey => {
      // 查找对应的配置数据
      const configKey = configKeys.find(k => k.id === serverKey.id);

      // 调试：输出服务器返回的原始数据
      console.log('Server key raw data:', serverKey.id, {
        is_banned: serverKey.is_banned,
        banned_at: serverKey.banned_at,
        ban_expires_at: serverKey.ban_expires_at,
        ban_reason: serverKey.ban_reason,
        ban_count: serverKey.ban_count
      });

      // 后端返回扁平结构，转换为前端嵌套结构
      // 确保数值类型正确转换
      const banStatus = {
        is_banned: serverKey.is_banned || false,
        banned_at: serverKey.banned_at ? parseInt(serverKey.banned_at) : null,
        banned_until: serverKey.ban_expires_at ? parseInt(serverKey.ban_expires_at) : null,
        triggered_rule_id: serverKey.triggered_rule_id || null,
        reason: serverKey.ban_reason || null,
        ban_count: parseInt(serverKey.ban_count) || 0
      };

      // 调试：输出转换后的数据
      console.log('Converted ban status:', serverKey.id, banStatus);

      return {
        ...serverKey,
        // 使用配置中的路由信息（如果服务器没有返回）
        route_ids: serverKey.route_ids || configKey?.route_ids || [],
        route_names: serverKey.route_names || configKey?.route_names || [],
        // 合并封禁状态
        ban_status: banStatus,
        // 保留配置中的其他字段
        per_minute: serverKey.per_minute || configKey?.per_minute || 120,
        max_inflight: serverKey.max_inflight || configKey?.max_inflight || null,
        // 保留 Token 配额配置
        token_quota: serverKey.token_quota || configKey?.token_quota || null,
        created_at: serverKey.created_at || configKey?.created_at || new Date().toISOString(),
        updated_at: serverKey.updated_at || configKey?.updated_at || new Date().toISOString()
      };
    });
  } else {
    // 服务器不可用，使用配置数据
    apiKeysData = configKeys;
  }

  return apiKeysData;
}

// 加载封禁日志
async function loadBanLogs() {
  console.log('Loading ban logs...');
  const logs = await fetchBanLogsFromServer();
  console.log('Fetched logs:', logs);
  if (logs !== null) {
    banLogsData = logs;
    console.log('Updated banLogsData:', banLogsData);
  } else {
    console.warn('Failed to load ban logs, keeping existing data');
  }
  return banLogsData;
}

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

async function switchTab(id) {
  document.querySelectorAll('.tab').forEach(t => {
    t.classList.toggle('active', t.dataset.tab === id);
  });
  document.querySelectorAll('.tab-panel').forEach(p => {
    p.classList.toggle('active', p.id === 'tab-' + id);
  });

  // 切换到特定标签页时加载数据
  if (id === 'apikeys') {
    await loadApiKeys();
    renderApiKeys();
  } else if (id === 'banlogs') {
    await loadBanLogs();
    renderBanLogs();
  } else if (id === 'tokenstats') {
    await loadTokenStats();
    renderTokenStats();
  }
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

    // 先从服务器加载实时数据（封禁状态等）
    await loadApiKeys();
    await loadBanLogs();

    // 渲染所有标签页（使用包含实时数据的合并结果）
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
  renderApiKeys();
  renderBanRules();
  renderBanLogs();
  renderMetrics();
  renderGateway();
  renderAdvanced();
  renderTokenStats();
}

// -- API Keys 管理页面 --
function renderApiKeys() {
  const panel = document.getElementById('tab-apikeys');
  if (!panel) return;

  // 如果数据为空，尝试从服务器加载或从配置加载（备用）
  if (apiKeysData.length === 0) {
    // 注意：正常情况下 loadConfig() 应该已经调用了 loadApiKeys()
    // 这里只是作为备用
    console.warn('apiKeysData is empty, falling back to config data');
    if (cfg?.api_keys?.keys) {
      apiKeysData = loadApiKeysFromConfig();
    }
  }

  // 获取所有路由选项
  const routes = cfg?.routes || [];
  const routeOptions = routes.map(r => `<option value="${esc(r.id)}" ${apiKeyFilter.route === r.id ? 'selected' : ''}>${esc(r.id)}</option>`).join('');

  // 过滤 API Keys（适配新数据结构 v2）
  let filteredKeys = apiKeysData.filter(key => {
    // 按路由筛选（检查 route_ids 数组）
    if (apiKeyFilter.route && (!key.route_ids || !key.route_ids.includes(apiKeyFilter.route))) return false;
    // 按状态筛选
    if (apiKeyFilter.status !== 'all') {
      const isBanned = key.ban_status?.is_banned || false;
      if (apiKeyFilter.status === 'enabled' && (!key.enabled || isBanned)) return false;
      if (apiKeyFilter.status === 'disabled' && key.enabled) return false;
      if (apiKeyFilter.status === 'banned' && !isBanned) return false;
    }
    // 搜索
    if (apiKeyFilter.search) {
      const search = apiKeyFilter.search.toLowerCase();
      const matchKey = key.key.toLowerCase().includes(search);
      const matchRemark = (key.remark || '').toLowerCase().includes(search);
      if (!matchKey && !matchRemark) return false;
    }
    return true;
  });

  // 生成表格行
  const tableRows = filteredKeys.map(key => {
    // 检查封禁状态（考虑时间过期的情况）
    let isBanned = key.ban_status?.is_banned || false;
    const bannedUntil = key.ban_status?.banned_until;
    if (isBanned && bannedUntil) {
      const now = Math.floor(Date.now() / 1000); // Unix秒
      if (now >= parseInt(bannedUntil)) {
        // 封禁时间已过期，视为未封禁
        isBanned = false;
      }
    }
    const status = isBanned ? 'banned' : (key.enabled ? 'enabled' : 'disabled');
    const statusClass = `status-${status}`;
    const statusText = isBanned ? '封禁中' : (key.enabled ? '启用' : '禁用');
    const shortKey = key.key;

    // 路由标签（支持多路由显示）
    let routeTags;
    if (!key.route_ids || key.route_ids.length === 0) {
      routeTags = '<span class="route-tag all-routes">所有路由</span>';
    } else {
      routeTags = key.route_ids.slice(0, 2).map((rid, idx) => {
        const rname = key.route_names?.[idx] || rid;
        return `<span class="route-tag" title="${esc(rid)}">${esc(rname)}</span>`;
      }).join('');
      if (key.route_ids.length > 2) {
        routeTags += `<span class="route-tag" title="${key.route_ids.slice(2).join(', ')}">+${key.route_ids.length - 2}</span>`;
      }
    }

    // 封禁状态显示
    let banStatusHtml = '-';
    // 重新计算 isBanned（可能已经过期）
    let isCurrentlyBanned = key.ban_status?.is_banned || false;
    const bannedUntilTs = key.ban_status?.banned_until;
    if (isCurrentlyBanned && bannedUntilTs) {
      const now = Math.floor(Date.now() / 1000);
      if (now >= parseInt(bannedUntilTs)) {
        isCurrentlyBanned = false;
      }
    }
    console.log('Render ban status:', key.id, {
      isCurrentlyBanned,
      banned_until: key.ban_status?.banned_until,
      type: typeof key.ban_status?.banned_until
    });
    if (isCurrentlyBanned && bannedUntilTs) {
      const bannedUntil = parseInt(bannedUntilTs);
      console.log('Parsed bannedUntil:', bannedUntil, 'isNaN:', isNaN(bannedUntil));
      if (!isNaN(bannedUntil) && bannedUntil > 0) {
        const expiresAtMs = bannedUntil * 1000; // Unix秒转毫秒
        console.log('Setting data-expires:', expiresAtMs, 'type:', typeof expiresAtMs);
        banStatusHtml = `<span class="ban-timer" data-expires="${expiresAtMs}">计算中...</span>`;
      } else {
        banStatusHtml = '<span class="ban-status-badge banned">已封禁</span>';
      }
    } else if (key.ban_status?.ban_count > 0) {
      banStatusHtml = `<span class="ban-history">历史封禁 ${key.ban_status.ban_count} 次</span>`;
    }

    return `
      <tr data-id="${esc(key.id)}">
        <td><code class="apikey-code" title="${esc(key.key)}">${esc(shortKey)}</code></td>
        <td class="route-cell">${routeTags}</td>
        <td><span class="status-badge ${statusClass}">${statusText}</span></td>
        <td class="remark-cell">
          <span class="remark-text" onclick="editRemark('${esc(key.id)}', this)">${esc(key.remark || '-')}</span>
        </td>
        <td class="ban-status">${banStatusHtml}</td>
        <td class="actions-cell">
          <label class="toggle apikey-toggle">
            <input type="checkbox" class="toggle-input" ${key.enabled ? 'checked' : ''}
                   onchange="toggleApiKeyEnabled('${esc(key.id)}', this.checked)"
                   ${isBanned ? 'disabled' : ''}>
            <span class="toggle-slider" aria-hidden="true"></span>
          </label>
          <button class="btn btn-secondary btn-sm" onclick="openApiKeyModal('${esc(key.id)}')" title="编辑配置">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
            </svg>
          </button>
          ${isBanned ? `<button class="btn btn-primary btn-sm" onclick="unbanApiKey('${esc(key.id)}')" title="解封">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M12 2v20M2 12h20"/>
            </svg>
            解封
          </button>` : ''}
          <button class="btn btn-danger btn-sm" onclick="deleteApiKeyById('${esc(key.id)}')" title="删除">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
            </svg>
          </button>
        </td>
      </tr>
    `;
  }).join('');

  panel.innerHTML = `
    <div class="apikeys-page">
      <div class="apikeys-header">
        <h2 class="apikeys-title">API Key 管理</h2>
        <button class="btn btn-primary" onclick="openApiKeyModal()">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 5v14M5 12h14"/>
          </svg>
          新增 API Key
        </button>
      </div>

      <div class="apikeys-toolbar">
        <div class="filter-group">
          <select class="input select filter-select" onchange="setApiKeyFilter('route', this.value)">
            <option value="" ${apiKeyFilter.route === '' ? 'selected' : ''}>所有路由</option>
            ${routeOptions}
          </select>
          <select class="input select filter-select" onchange="setApiKeyFilter('status', this.value)">
            <option value="all" ${apiKeyFilter.status === 'all' ? 'selected' : ''}>所有状态</option>
            <option value="enabled" ${apiKeyFilter.status === 'enabled' ? 'selected' : ''}>启用</option>
            <option value="disabled" ${apiKeyFilter.status === 'disabled' ? 'selected' : ''}>禁用</option>
            <option value="banned" ${apiKeyFilter.status === 'banned' ? 'selected' : ''}>封禁中</option>
          </select>
        </div>
        <div class="search-box">
          <input type="text" class="input" placeholder="搜索 API Key 或备注..."
                 value="${esc(apiKeyFilter.search)}" onchange="setApiKeyFilter('search', this.value)">
          <svg class="search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8"/>
            <path d="m21 21-4.3-4.3"/>
          </svg>
        </div>
      </div>

      <div class="apikeys-table-wrapper">
        <table class="apikeys-table">
          <thead>
            <tr>
              <th>API Key</th>
              <th>路由</th>
              <th>状态</th>
              <th>备注</th>
              <th>封禁状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            ${tableRows || '<tr><td colspan="6" class="empty-cell">暂无 API Key</td></tr>'}
          </tbody>
        </table>
      </div>
    </div>
  `;

  // 启动封禁倒计时更新
  updateBanTimers();
}

// 更新封禁倒计时
function updateBanTimers() {
  const timers = document.querySelectorAll('.ban-timer');
  console.log('updateBanTimers called, found timers:', timers.length);
  timers.forEach((timer, idx) => {
    console.log(`Timer ${idx}: dataset.expires =`, timer.dataset.expires);
    const expiresAt = new Date(parseInt(timer.dataset.expires));
    const now = new Date();
    const diff = expiresAt - now;
    console.log(`Timer ${idx}: expiresAt =`, expiresAt, 'diff =', diff, 'isNaN:', isNaN(diff));

    if (isNaN(diff)) {
      timer.textContent = '计算错误';
      console.error(`Timer ${idx}: Invalid diff calculation`);
      return;
    }
    if (diff <= 0) {
      timer.textContent = '即将解封';
      timer.classList.add('expiring');
    } else {
      const hours = Math.floor(diff / 3600000);
      const minutes = Math.floor((diff % 3600000) / 60000);
      const seconds = Math.floor((diff % 60000) / 1000);

      console.log(`Timer ${idx}: hours=${hours}, minutes=${minutes}, seconds=${seconds}, textContent will be:`, hours > 0 ? `${hours}小时${minutes}分` : minutes > 0 ? `${minutes}分${seconds}秒` : `${seconds}秒`);

      if (hours > 0) {
        timer.textContent = `${hours}小时${minutes}分`;
      } else if (minutes > 0) {
        timer.textContent = `${minutes}分${seconds}秒`;
      } else {
        timer.textContent = `${seconds}秒`;
        timer.classList.add('expiring');
      }
    }
  });

  // 每秒更新一次
  if (timers.length > 0) {
    setTimeout(updateBanTimers, 1000);
  }
}

// 设置 API Key 过滤器
function setApiKeyFilter(key, value) {
  apiKeyFilter[key] = value;
  renderApiKeys();
}

// 切换 API Key 启用状态
function toggleApiKeyEnabled(id, enabled) {
  const key = apiKeysData.find(k => k.id === id);
  if (key) {
    key.enabled = enabled;
    key.updated_at = new Date().toISOString();

    // 同步到配置
    if (cfg.api_keys && cfg.api_keys.keys) {
      const configKey = cfg.api_keys.keys.find(k => k.id === id);
      if (configKey) {
        configKey.enabled = enabled;
      }
    }

    Toast.show(enabled ? 'API Key 已启用' : 'API Key 已禁用', 'success');
    renderApiKeys();
  }
}

// 编辑备注
function editRemark(id, element) {
  const key = apiKeysData.find(k => k.id === id);
  if (!key) return;

  const currentRemark = key.remark || '';
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'input remark-input';
  input.value = currentRemark;
  input.placeholder = '输入备注...';

  element.replaceWith(input);
  input.focus();

  const saveRemark = () => {
    const newRemark = input.value.trim();
    key.remark = newRemark || '';
    key.updated_at = new Date().toISOString();

    // 同步到配置
    if (cfg.api_keys && cfg.api_keys.keys) {
      const configKey = cfg.api_keys.keys.find(k => k.id === id);
      if (configKey) {
        configKey.remark = newRemark;
      }
    }

    Toast.show('备注已更新', 'success');
    renderApiKeys();
  };

  input.addEventListener('blur', saveRemark);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      input.blur();
    } else if (e.key === 'Escape') {
      renderApiKeys();
    }
  });
}

 // 解封 API Key（适配新数据结构 v2）
async function unbanApiKey(id) {
  confirmDelete(
    '解封 API Key',
    '确定要手动解封此 API Key 吗？',
    async () => {
      const token = getToken();
      if (!token) {
        logout();
        return;
      }

      try {
        const response = await fetch(`${window.CONFIG.keysUrl}/${id}/unban`, {
          method: 'POST',
          headers: {
            'Authorization': `Bearer ${token}`,
            'Content-Type': 'application/json'
          }
        });

        if (!response.ok) {
          if (response.status === 401) {
            logout();
            return;
          }
          const err = await response.json().catch(() => ({}));
          throw new Error(err.error || `HTTP ${response.status}`);
        }

        const key = apiKeysData.find(k => k.id === id);
        if (key) {
          const now = Math.floor(Date.now() / 1000); // Unix秒

          // 更新前端数据
          key.ban_status = {
            is_banned: false,
            banned_at: key.ban_status?.banned_at || null,
            banned_until: null,
            triggered_rule_id: null,
            reason: null,
            ban_count: key.ban_status?.ban_count || 0
          };
          key.updated_at = new Date().toISOString();
        }

        Toast.show('API Key 已解封', 'success');
        await loadBanLogs(); // 重新加载封禁日志
        renderApiKeys();
        renderBanLogs();
      } catch (err) {
        console.error('Failed to unban API Key:', err);
        Toast.show(`解封失败: ${err.message}`, 'error');
      }
    }
  );
}

// 删除 API Key
function deleteApiKeyById(id) {
  const key = apiKeysData.find(k => k.id === id);
  confirmDelete(
    '删除 API Key',
    `确定要删除 API Key "${esc(key?.key?.substring(0, 20))}..." 吗？此操作不可撤销。`,
    () => {
      apiKeysData = apiKeysData.filter(k => k.id !== id);
      Toast.show('API Key 已删除', 'success');
      renderApiKeys();
    }
  );
}

// 打开 API Key 配置弹窗（架构设计 v2）
function openApiKeyModal(id) {
  const isEdit = !!id;
  const key = isEdit ? apiKeysData.find(k => k.id === id) : null;
  const routes = cfg?.routes || [];

  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.id = 'apikey-modal';

  // 准备路由选择数据（支持多选）
  const selectedRoutes = key?.route_ids || [];
  const isAllRoutes = selectedRoutes.length === 0;

  modal.innerHTML = `
    <div class="modal apikey-modal" role="dialog" aria-modal="true">
      <div class="modal-header">
        <h3 class="modal-title">${isEdit ? '编辑 API Key' : '新增 API Key'}</h3>
        <button class="btn btn-ghost btn-sm" onclick="closeApiKeyModal()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="modal-body">
        <form id="apikey-form">
          <div class="form-section">
            <h4 class="section-title">基本信息</h4>
            <div class="form-row">
              <div class="field">
                <label class="field-label">API Key <span class="required">*</span></label>
                <input type="text" class="input" id="apikey-value" value="${esc(key?.key || '')}"
                       placeholder="sk-..." ${isEdit ? 'readonly' : ''} required>
                ${!isEdit ? '<div class="field-help">留空将自动生成</div>' : ''}
              </div>
            </div>
            <div class="field">
              <label class="field-label">允许访问的路由</label>
              <div class="multi-select" id="route-multi-select" data-selected='${JSON.stringify(selectedRoutes)}'>
                <div class="multi-select-trigger" onclick="toggleMultiSelect(this)">
                  <div class="multi-select-values">
                    ${isAllRoutes ? '<span class="multi-select-tag all-routes">所有路由</span>' : selectedRoutes.map(r => {
                      const route = routes.find(rt => rt.id === r);
                      return `<span class="multi-select-tag" data-value="${esc(r)}">${esc(route?.id || r)}<span class="multi-select-tag-remove" onclick="event.stopPropagation(); removeRouteTag(this)"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></span></span>`;
                    }).join('')}
                  </div>
                  <svg class="multi-select-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>
                </div>
                <div class="multi-select-dropdown" style="display: none;">
                  <div class="multi-select-search">
                    <input type="text" placeholder="搜索路由..." oninput="filterRouteOptions(this)">
                  </div>
                  <div class="multi-select-options">
                    <div class="multi-select-option special ${isAllRoutes ? 'selected' : ''}" data-value="" onclick="selectRouteOption(this)">
                      <div class="multi-select-checkbox"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><polyline points="20 6 9 17 4 12"/></svg></div>
                      <span>所有路由</span>
                    </div>
                    ${routes.map(r => {
                      const isSel = selectedRoutes.includes(r.id);
                      return `<div class="multi-select-option ${isSel ? 'selected' : ''}" data-value="${esc(r.id)}" onclick="selectRouteOption(this)">
                        <div class="multi-select-checkbox"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><polyline points="20 6 9 17 4 12"/></svg></div>
                        <span>${esc(r.id)}${r.prefix ? ` (${esc(r.prefix)})` : ''}</span>
                      </div>`;
                    }).join('')}
                  </div>
                </div>
              </div>
              <input type="hidden" id="apikey-routes" class="apikey-routes-input" value='${JSON.stringify(selectedRoutes)}'>
              <div class="field-help">选择"所有路由"可访问全部路由；选择具体路由则只能访问指定路由</div>
            </div>
            <div class="field">
              <label class="field-label">备注</label>
              <input type="text" class="input" id="apikey-remark" value="${esc(key?.remark || '')}"
                     placeholder="描述此 API Key 的用途...">
            </div>
          </div>

          <div class="form-section">
            <h4 class="section-title">限流配置</h4>
            <div class="form-row">
              <div class="field">
                <label class="field-label">每分钟请求数 (per_minute)</label>
                <input type="number" class="input" id="apikey-per-minute" value="${key?.per_minute || 120}"
                       min="1" placeholder="120">
              </div>
            </div>
          </div>

          <div class="form-section">
            <h4 class="section-title">并发配置</h4>
            <div class="form-row">
              <div class="field">
                <label class="field-label">最大并发数 (max_inflight)</label>
                <input type="number" class="input" id="apikey-max-inflight" value="${key?.max_inflight || ''}"
                       min="1" placeholder="无限制">
              </div>
            </div>
          </div>

          <div class="form-section">
            <h4 class="section-title">Token 配额配置</h4>
            <div class="form-row">
              <div class="field">
                <label class="field-label">每日 Token 上限（总数）</label>
                <input type="number" class="input" id="apikey-daily-total-limit" value="${key?.token_quota?.daily_total_limit || ''}"
                       min="0" placeholder="无限制">
                <div class="field-help">每日允许使用的最大 Token 总数（input + output）</div>
              </div>
              <div class="field">
                <label class="field-label">每日 Input Token 上限</label>
                <input type="number" class="input" id="apikey-daily-input-limit" value="${key?.token_quota?.daily_input_limit || ''}"
                       min="0" placeholder="无限制">
                <div class="field-help">每日允许使用的最大 Input Token 数</div>
              </div>
            </div>
            <div class="form-row">
              <div class="field">
                <label class="field-label">每日 Output Token 上限</label>
                <input type="number" class="input" id="apikey-daily-output-limit" value="${key?.token_quota?.daily_output_limit || ''}"
                       min="0" placeholder="无限制">
                <div class="field-help">每日允许使用的最大 Output Token 数</div>
              </div>
              <div class="field">
                <label class="field-label">每周 Token 上限（总数）</label>
                <input type="number" class="input" id="apikey-weekly-total-limit" value="${key?.token_quota?.weekly_total_limit || ''}"
                       min="0" placeholder="无限制">
                <div class="field-help">每周允许使用的最大 Token 总数</div>
              </div>
            </div>
            <div class="form-row">
              <div class="field">
                <label class="field-label">每周 Input Token 上限</label>
                <input type="number" class="input" id="apikey-weekly-input-limit" value="${key?.token_quota?.weekly_input_limit || ''}"
                       min="0" placeholder="无限制">
              </div>
              <div class="field">
                <label class="field-label">每周 Output Token 上限</label>
                <input type="number" class="input" id="apikey-weekly-output-limit" value="${key?.token_quota?.weekly_output_limit || ''}"
                       min="0" placeholder="无限制">
              </div>
            </div>
          </div>

          <div class="form-section info-section">
            <div class="info-box">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="10"/>
                <path d="M12 16v-4"/>
                <path d="M12 8h.01"/>
              </svg>
              <p>封禁规则已移至<strong>"封禁规则"</strong>标签页统一管理，对所有 API Key 生效。</p>
            </div>
          </div>
        </form>
      </div>
      <div class="modal-footer">
        <button type="button" class="btn btn-secondary" onclick="closeApiKeyModal()">取消</button>
        <button type="button" class="btn btn-primary" onclick="saveApiKeyV2('${esc(id || '')}')">保存</button>
      </div>
    </div>
  `;

  document.body.appendChild(modal);

  // 点击遮罩关闭
  modal.addEventListener('click', (e) => {
    if (e.target === modal) closeApiKeyModal();
  });
}

// 关闭 API Key 弹窗
function closeApiKeyModal() {
  const modal = document.getElementById('apikey-modal');
  if (modal) modal.remove();
}

// 添加封禁规则（架构设计 v2）
function addBanRuleV2() {
  const list = document.getElementById('ban-rules-list');
  const emptyMsg = list.querySelector('.ban-rules-empty');
  if (emptyMsg) emptyMsg.remove();

  const idx = Date.now();
  const ruleHtml = `
    <div class="ban-rule-item" data-idx="${idx}">
      <div class="ban-rule-header">
        <input type="text" class="input rule-name" placeholder="规则名称" value="">
        <select class="input select rule-type" onchange="updateBanRuleFields(this)">
          <option value="error_rate">错误率</option>
          <option value="request_count">请求数</option>
          <option value="consecutive_errors">连续错误</option>
        </select>
        <input type="number" class="input" placeholder="封禁(秒)" value="3600" min="1" data-field="ban_duration">
        <label class="toggle rule-toggle">
          <input type="checkbox" checked data-field="enabled">
          <span class="toggle-slider"></span>
        </label>
        <button type="button" class="btn btn-danger btn-sm" onclick="this.closest('.ban-rule-item').remove(); checkBanRulesEmpty();">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="ban-rule-conditions">
        <input type="number" class="input" placeholder="窗口(秒)" value="300" min="1" data-field="window_secs">
        <input type="number" class="input" placeholder="错误率(0-1)" value="0.5" min="0" max="1" step="0.1" data-field="threshold">
        <input type="number" class="input" placeholder="最小请求" value="10" min="1" data-field="min_requests">
      </div>
    </div>
  `;
  list.insertAdjacentHTML('beforeend', ruleHtml);
}

// 更新封禁规则条件字段（根据类型）
function updateBanRuleFields(selectEl) {
  const type = selectEl.value;
  const conditionsDiv = selectEl.closest('.ban-rule-item').querySelector('.ban-rule-conditions');

  let fieldsHtml = '';
  if (type === 'error_rate') {
    fieldsHtml = `
      <input type="number" class="input" placeholder="窗口(秒)" value="300" min="1" data-field="window_secs">
      <input type="number" class="input" placeholder="错误率(0-1)" value="0.5" min="0" max="1" step="0.1" data-field="threshold">
      <input type="number" class="input" placeholder="最小请求" value="10" min="1" data-field="min_requests">
    `;
  } else if (type === 'request_count') {
    fieldsHtml = `
      <input type="number" class="input" placeholder="窗口(秒)" value="60" min="1" data-field="window_secs">
      <input type="number" class="input" placeholder="最大请求" value="1000" min="1" data-field="max_requests">
      <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
    `;
  } else if (type === 'consecutive_errors') {
    fieldsHtml = `
      <input type="number" class="input" placeholder="连续错误数" value="5" min="1" data-field="count">
      <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
      <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
    `;
  }
  conditionsDiv.innerHTML = fieldsHtml;
}

// 保存 API Key（架构设计 v2）
function saveApiKeyV2(id) {
  const value = document.getElementById('apikey-value').value.trim();
  const remark = document.getElementById('apikey-remark').value.trim();
  const perMinute = parseInt(document.getElementById('apikey-per-minute').value) || 120;
  const maxInflight = parseInt(document.getElementById('apikey-max-inflight').value) || null;

  // Token配额配置
  const dailyTotalLimit = parseInt(document.getElementById('apikey-daily-total-limit')?.value) || null;
  const dailyInputLimit = parseInt(document.getElementById('apikey-daily-input-limit')?.value) || null;
  const dailyOutputLimit = parseInt(document.getElementById('apikey-daily-output-limit')?.value) || null;
  const weeklyTotalLimit = parseInt(document.getElementById('apikey-weekly-total-limit')?.value) || null;
  const weeklyInputLimit = parseInt(document.getElementById('apikey-weekly-input-limit')?.value) || null;
  const weeklyOutputLimit = parseInt(document.getElementById('apikey-weekly-output-limit')?.value) || null;

  // 收集选中的路由（支持多路由）
  const routeInput = document.getElementById('apikey-routes');
  const selectedRoutes = routeInput ? JSON.parse(routeInput.value || '[]') : [];

  // 生成新的 ID
  const newId = id || 'key_' + Date.now();

  // 验证 apikey ID 唯一性
  if (!id || newId !== id) {
    if (isApiKeyIdExists(newId, id)) {
      Toast.show(`API Key ID '${newId}' 已存在，请使用其他名称`, 'error');
      return;
    }
  }

  // 查找路由名称
  const routeNames = selectedRoutes.map(r => {
    const rt = cfg?.routes?.find(rt => rt.id === r);
    return rt ? (rt.name || rt.id) : r;
  });

  // 构建 Token 配额对象
  const tokenQuota = {};
  if (dailyTotalLimit) tokenQuota.daily_total_limit = dailyTotalLimit;
  if (dailyInputLimit) tokenQuota.daily_input_limit = dailyInputLimit;
  if (dailyOutputLimit) tokenQuota.daily_output_limit = dailyOutputLimit;
  if (weeklyTotalLimit) tokenQuota.weekly_total_limit = weeklyTotalLimit;
  if (weeklyInputLimit) tokenQuota.weekly_input_limit = weeklyInputLimit;
  if (weeklyOutputLimit) tokenQuota.weekly_output_limit = weeklyOutputLimit;

  // 构建 API Key 数据对象
  const keyData = {
    id: newId,
    key: value || generateApiKey(selectedRoutes[0] || 'global'),
    route_ids: selectedRoutes,
    route_names: routeNames,
    enabled: true,
    remark: remark || '',
    per_minute: perMinute,
    max_inflight: maxInflight,
    ban_status: {
      is_banned: false,
      banned_at: null,
      banned_until: null,
      triggered_rule_id: null,
      reason: null,
      ban_count: 0
    },
    token_quota: Object.keys(tokenQuota).length > 0 ? tokenQuota : null,
    updated_at: new Date().toISOString()
  };

  if (!id) {
    keyData.created_at = new Date().toISOString();
  } else {
    // 保留原有的封禁状态
    const existingKey = apiKeysData.find(k => k.id === id);
    if (existingKey) {
      keyData.ban_status = existingKey.ban_status || keyData.ban_status;
      keyData.created_at = existingKey.created_at;
    }
  }

  // 更新前端数据
  if (id) {
    const index = apiKeysData.findIndex(k => k.id === id);
    if (index !== -1) {
      apiKeysData[index] = keyData;
    }
    Toast.show('API Key 已更新', 'success');
  } else {
    apiKeysData.push(keyData);
    Toast.show('API Key 已创建', 'success');
  }

  // 同步到 cfg.api_keys（后端配置格式）
  if (!cfg.api_keys) {
    cfg.api_keys = { keys: [] };
  }

  // 转换为后端配置格式
  const configKey = convertApiKeyToConfig(keyData);

  const existingIndex = cfg.api_keys.keys.findIndex(k => k.id === keyData.id);
  if (existingIndex !== -1) {
    cfg.api_keys.keys[existingIndex] = configKey;
  } else {
    cfg.api_keys.keys.push(configKey);
  }

  closeApiKeyModal();
  renderApiKeys();
}

// 检查封禁规则是否为空
function checkBanRulesEmpty() {
  const list = document.getElementById('ban-rules-list');
  if (list.children.length === 0) {
    list.innerHTML = '<div class="ban-rules-empty">暂无规则，点击"添加规则"创建</div>';
  }
}

// 保存 API Key 到配置
function saveApiKey(id) {
  const value = document.getElementById('apikey-value').value.trim();
  const routeId = document.getElementById('apikey-route').value;
  const remark = document.getElementById('apikey-remark').value.trim();
  const perMinute = parseInt(document.getElementById('apikey-per-minute').value) || 120;
  const downstream = parseInt(document.getElementById('apikey-downstream').value) || null;
  const upstream = parseInt(document.getElementById('apikey-upstream').value) || null;

  // 收集封禁规则
  const banRules = [];
  document.querySelectorAll('.ban-rule-row').forEach(row => {
    const inputs = row.querySelectorAll('input');
    banRules.push({
      window_seconds: parseInt(inputs[0].value) || 60,
      violation_threshold: parseInt(inputs[1].value) || 10,
      ban_duration_seconds: parseInt(inputs[2].value) || 300
    });
  });

  if (!routeId) {
    Toast.show('请选择路由', 'error');
    return;
  }

  // 查找路由名称
  const route = cfg?.routes?.find(r => r.id === routeId);
  const routeName = route ? (route.name || routeId) : routeId;

  // 构建 API Key 数据对象（使用 route_ids 数组格式）
  const keyData = {
    id: id || Date.now().toString(),
    key: value || generateApiKey(routeId),
    route_ids: routeId ? [routeId] : [],
    route_names: routeName ? [routeName] : [],
    enabled: true,
    banned: false,
    remark: remark || '',
    per_minute: perMinute,
    downstream_max_inflight: downstream,
    upstream_per_key_max_inflight: upstream,
    ban_rules: banRules,
    updated_at: new Date().toISOString()
  };

  if (!id) {
    keyData.created_at = new Date().toISOString();
  }

  // 更新前端数据
  if (id) {
    const index = apiKeysData.findIndex(k => k.id === id);
    if (index !== -1) {
      // 保留原有的 enabled 和 banned 状态
      keyData.enabled = apiKeysData[index].enabled;
      keyData.banned = apiKeysData[index].banned;
      keyData.created_at = apiKeysData[index].created_at;
      apiKeysData[index] = keyData;
    }
    Toast.show('API Key 已更新', 'success');
  } else {
    apiKeysData.push(keyData);
    Toast.show('API Key 已创建', 'success');
  }

  // 同步到 cfg.api_keys（后端配置格式）
  if (!cfg.api_keys) {
    cfg.api_keys = { keys: [] };
  }

  // 转换为后端配置格式
  const configKey = convertApiKeyToConfig(keyData);

  const existingIndex = cfg.api_keys.keys.findIndex(k => k.id === keyData.id);
  if (existingIndex !== -1) {
    cfg.api_keys.keys[existingIndex] = configKey;
  } else {
    cfg.api_keys.keys.push(configKey);
  }

  closeApiKeyModal();
  renderApiKeys();
}

// 删除 API Key
function deleteApiKeyById(id) {
  const key = apiKeysData.find(k => k.id === id);
  confirmDelete(
    '删除 API Key',
    `确定要删除 API Key "${esc(key?.key?.substring(0, 20))}..." 吗？此操作不可撤销。`,
    () => {
      // 从前端数据中删除
      apiKeysData = apiKeysData.filter(k => k.id !== id);

      // 从配置中删除
      if (cfg.api_keys && cfg.api_keys.keys) {
        cfg.api_keys.keys = cfg.api_keys.keys.filter(k => k.id !== id);
      }

      Toast.show('API Key 已删除', 'success');
      renderApiKeys();
    }
  );
}

// -- 全局封禁规则管理页面 --
let banRulesData = []; // 全局封禁规则数据

function renderBanRules() {
  const panel = document.getElementById('tab-banrules');
  if (!panel) return;

  // 从配置加载全局封禁规则
  if (cfg?.api_keys?.ban_rules) {
    banRulesData = cfg.api_keys.ban_rules;
  } else {
    banRulesData = [];
  }

  const rulesHtml = banRulesData.map((rule, idx) => {
    const cond = rule.condition || {};
    let conditionText = '';
    if (cond.type === 'error_rate') {
      conditionText = `错误率 > ${(cond.threshold * 100).toFixed(0)}% (${cond.window_secs}秒窗口, 最少${cond.min_requests}请求)`;
    } else if (cond.type === 'request_count') {
      conditionText = `请求数 > ${cond.max_requests} (${cond.window_secs}秒窗口)`;
    } else if (cond.type === 'consecutive_errors') {
      conditionText = `连续错误 > ${cond.count} 次`;
    }

    const durationText = formatDuration(rule.ban_duration_secs);
    const triggerThreshold = rule.trigger_count_threshold || 1;
    const triggerWindow = formatDuration(rule.trigger_window_secs || 3600);
    const triggerText = triggerThreshold > 1
      ? `<span class="trigger-badge" title="${triggerThreshold} 次触发 / ${triggerWindow}">${triggerThreshold} 次</span>`
      : '<span class="trigger-badge immediate">立即</span>';

    return `
      <tr data-idx="${idx}">
        <td><span class="rule-name">${esc(rule.name || '未命名规则')}</span></td>
        <td><span class="condition-badge ${cond.type}">${conditionText}</span></td>
        <td>${durationText}</td>
        <td>${triggerText}</td>
        <td>
          <label class="toggle">
            <input type="checkbox" class="toggle-input" ${rule.enabled !== false ? 'checked' : ''} onchange="toggleBanRule(${idx}, this.checked)">
            <span class="toggle-slider"></span>
          </label>
        </td>
        <td class="actions-cell">
          <button class="btn btn-secondary btn-sm" onclick="editBanRule(${idx})" title="编辑">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
            </svg>
          </button>
          <button class="btn btn-danger btn-sm" onclick="deleteBanRule(${idx})" title="删除">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
            </svg>
          </button>
        </td>
      </tr>
    `;
  }).join('');

  panel.innerHTML = `
    <div class="banrules-page">
      <div class="banrules-header">
        <div class="header-info">
          <h2 class="banrules-title">全局封禁规则</h2>
          <p class="banrules-desc">配置对所有 API Key 生效的自动封禁规则。当某个 API Key 的请求满足规则条件时，将自动被封禁。</p>
        </div>
        <button class="btn btn-primary" onclick="openBanRuleModal()">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 5v14M5 12h14"/>
          </svg>
          添加规则
        </button>
      </div>

      <div class="banrules-table-wrapper">
        <table class="banrules-table">
          <thead>
            <tr>
              <th>规则名称</th>
              <th>触发条件</th>
              <th>封禁时长</th>
              <th>触发阈值</th>
              <th>启用状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            ${rulesHtml || '<tr><td colspan="6" class="empty-cell">暂无封禁规则</td></tr>'}
          </tbody>
        </table>
      </div>

      <div class="banrules-help">
        <h4><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
          <path d="M12 16v-4"/>
          <path d="M12 8h.01"/>
        </svg> 规则说明</h4>
        <ul>
          <li><strong>错误率规则</strong>：在指定时间窗口内，错误率超过阈值且请求数达到最小值时触发</li>
          <li><strong>请求数规则</strong>：在指定时间窗口内，请求数超过最大值时触发</li>
          <li><strong>连续错误规则</strong>：连续出现指定次数的错误时触发</li>
          <li><strong>触发次数阈值</strong>：可以设置规则在多长时间内触发多少次后才真正执行封禁（防止偶发波动）</li>
          <li>规则对所有 API Key 生效，每个 API Key 的统计是独立的</li>
        </ul>
      </div>
    </div>
  `;
}

// 格式化时长显示
function formatDuration(seconds) {
  if (seconds < 60) return `${seconds}秒`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}分钟`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}小时`;
  return `${Math.floor(seconds / 86400)}天`;
}

// 切换规则启用状态
function toggleBanRule(idx, enabled) {
  if (!cfg.api_keys) cfg.api_keys = {};
  if (!cfg.api_keys.ban_rules) cfg.api_keys.ban_rules = [];

  if (cfg.api_keys.ban_rules[idx]) {
    cfg.api_keys.ban_rules[idx].enabled = enabled;
    Toast.show(enabled ? '规则已启用' : '规则已禁用', 'success');
    renderBanRules();
  }
}

// 删除封禁规则
function deleteBanRule(idx) {
  confirmDelete('删除封禁规则', '确定要删除这条封禁规则吗？此操作不可撤销。', () => {
    if (cfg?.api_keys?.ban_rules) {
      cfg.api_keys.ban_rules.splice(idx, 1);
      Toast.show('封禁规则已删除', 'success');
      renderBanRules();
    }
  });
}

// 打开封禁规则编辑弹窗
function openBanRuleModal(idx) {
  const isEdit = idx !== undefined;
  const rule = isEdit ? cfg?.api_keys?.ban_rules?.[idx] : null;

  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.id = 'banrule-modal';

  const cond = rule?.condition || {};
  const condType = cond.type || 'error_rate';

  modal.innerHTML = `
    <div class="modal banrule-modal" role="dialog" aria-modal="true">
      <div class="modal-header">
        <h3 class="modal-title">${isEdit ? '编辑封禁规则' : '添加封禁规则'}</h3>
        <button class="btn btn-ghost btn-sm" onclick="closeBanRuleModal()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="modal-body">
        <form id="banrule-form">
          <div class="field">
            <label class="field-label">规则名称</label>
            <input type="text" class="input" id="banrule-name" value="${esc(rule?.name || '')}"
                   placeholder="例如：高频错误封禁">
          </div>

          <div class="field">
            <label class="field-label">触发条件类型</label>
            <select class="input select" id="banrule-type" onchange="updateBanRuleConditionFields()">
              <option value="error_rate" ${condType === 'error_rate' ? 'selected' : ''}>错误率阈值</option>
              <option value="request_count" ${condType === 'request_count' ? 'selected' : ''}>请求数阈值</option>
              <option value="consecutive_errors" ${condType === 'consecutive_errors' ? 'selected' : ''}>连续错误数</option>
            </select>
          </div>

          <div id="banrule-condition-fields" class="condition-fields">
            ${getBanRuleConditionFieldsHtml(condType, cond)}
          </div>

          <div class="field">
            <label class="field-label">封禁时长</label>
            <div class="duration-input-group">
              <input type="number" class="input" id="banrule-duration" value="${rule?.ban_duration_secs || 3600}"
                     min="60" step="60">
              <span class="input-suffix">秒</span>
            </div>
            <div class="field-help">建议：1小时=3600秒，1天=86400秒</div>
          </div>

          <div class="form-section">
            <h5 class="section-subtitle">多次触发设置</h5>
            <div class="form-row">
              <div class="field">
                <label class="field-label">触发次数阈值</label>
                <input type="number" class="input" id="banrule-trigger-count"
                       value="${rule?.trigger_count_threshold || 1}"
                       min="1" max="100" step="1">
                <div class="field-help">在计数窗口期内触发多少次后才执行封禁（1=立即封禁）</div>
              </div>
              <div class="field">
                <label class="field-label">触发计数窗口 (秒)</label>
                <input type="number" class="input" id="banrule-trigger-window"
                       value="${rule?.trigger_window_secs || 3600}"
                       min="60" step="60">
                <div class="field-help">统计触发次数的时间窗口（例如：3600=1小时）</div>
              </div>
            </div>
          </div>

          <div class="field">
            <label class="toggle-label">
              <input type="checkbox" id="banrule-enabled" ${rule?.enabled !== false ? 'checked' : ''}>
              <span>启用此规则</span>
            </label>
          </div>
        </form>
      </div>
      <div class="modal-footer">
        <button type="button" class="btn btn-secondary" onclick="closeBanRuleModal()">取消</button>
        <button type="button" class="btn btn-primary" onclick="saveBanRule(${isEdit ? idx : 'null'})">保存</button>
      </div>
    </div>
  `;

  document.body.appendChild(modal);

  modal.addEventListener('click', (e) => {
    if (e.target === modal) closeBanRuleModal();
  });
}

// 获取封禁规则条件字段 HTML
function getBanRuleConditionFieldsHtml(type, cond) {
  if (type === 'error_rate') {
    return `
      <div class="form-row">
        <div class="field">
          <label class="field-label">错误率阈值 (0-1)</label>
          <input type="number" class="input" id="cond-threshold" value="${cond.threshold || 0.5}"
                 min="0" max="1" step="0.1" placeholder="0.5">
          <div class="field-help">例如：0.5 表示 50% 错误率</div>
        </div>
        <div class="field">
          <label class="field-label">时间窗口 (秒)</label>
          <input type="number" class="input" id="cond-window" value="${cond.window_secs || 300}"
                 min="10" step="10" placeholder="300">
          <div class="field-help">例如：300 = 5分钟</div>
        </div>
        <div class="field">
          <label class="field-label">最小请求数</label>
          <input type="number" class="input" id="cond-min-requests" value="${cond.min_requests || 10}"
                 min="1" placeholder="10">
          <div class="field-help">避免样本过少误触发</div>
        </div>
      </div>
    `;
  } else if (type === 'request_count') {
    return `
      <div class="form-row">
        <div class="field">
          <label class="field-label">最大请求数</label>
          <input type="number" class="input" id="cond-max-requests" value="${cond.max_requests || 1000}"
                 min="1" placeholder="1000">
        </div>
        <div class="field">
          <label class="field-label">时间窗口 (秒)</label>
          <input type="number" class="input" id="cond-window" value="${cond.window_secs || 60}"
                 min="10" step="10" placeholder="60">
          <div class="field-help">例如：60 = 1分钟</div>
        </div>
      </div>
    `;
  } else if (type === 'consecutive_errors') {
    return `
      <div class="field">
        <label class="field-label">连续错误数阈值</label>
        <input type="number" class="input" id="cond-count" value="${cond.count || 5}"
               min="1" placeholder="5">
        <div class="field-help">连续出现指定次数的错误时触发封禁</div>
      </div>
    `;
  }
  return '';
}

// 更新封禁规则条件字段
function updateBanRuleConditionFields() {
  const type = document.getElementById('banrule-type').value;
  const container = document.getElementById('banrule-condition-fields');
  container.innerHTML = getBanRuleConditionFieldsHtml(type, {});
}

// 关闭封禁规则弹窗
function closeBanRuleModal() {
  const modal = document.getElementById('banrule-modal');
  if (modal) modal.remove();
}

// 保存封禁规则
function saveBanRule(idx) {
  const name = document.getElementById('banrule-name').value.trim();
  const type = document.getElementById('banrule-type').value;
  const duration = parseInt(document.getElementById('banrule-duration').value) || 3600;
  const enabled = document.getElementById('banrule-enabled').checked;

  if (!name) {
    Toast.show('请输入规则名称', 'error');
    return;
  }

  // 收集条件字段
  let condition = { type };
  if (type === 'error_rate') {
    condition.threshold = parseFloat(document.getElementById('cond-threshold').value) || 0.5;
    condition.window_secs = parseInt(document.getElementById('cond-window').value) || 300;
    condition.min_requests = parseInt(document.getElementById('cond-min-requests').value) || 10;
  } else if (type === 'request_count') {
    condition.max_requests = parseInt(document.getElementById('cond-max-requests').value) || 1000;
    condition.window_secs = parseInt(document.getElementById('cond-window').value) || 60;
  } else if (type === 'consecutive_errors') {
    condition.count = parseInt(document.getElementById('cond-count').value) || 5;
  }

  const rule = {
    id: idx !== null ? cfg.api_keys.ban_rules[idx]?.id : 'rule_' + Date.now(),
    name,
    condition,
    ban_duration_secs: duration,
    enabled,
    trigger_count_threshold: parseInt(document.getElementById('banrule-trigger-count').value) || 1,
    trigger_window_secs: parseInt(document.getElementById('banrule-trigger-window').value) || 3600
  };

  if (!cfg.api_keys) cfg.api_keys = {};
  if (!cfg.api_keys.ban_rules) cfg.api_keys.ban_rules = [];

  if (idx !== null) {
    cfg.api_keys.ban_rules[idx] = rule;
    Toast.show('封禁规则已更新', 'success');
  } else {
    cfg.api_keys.ban_rules.push(rule);
    Toast.show('封禁规则已添加', 'success');
  }

  closeBanRuleModal();
  renderBanRules();
}

// 编辑封禁规则
function editBanRule(idx) {
  openBanRuleModal(idx);
}

// -- Ban Logs 封禁日志页面（架构设计 v2）--
function renderBanLogs() {
  console.log('Rendering ban logs, banLogsData length:', banLogsData.length);
  const panel = document.getElementById('tab-banlogs');
  if (!panel) {
    console.warn('Ban logs panel not found');
    return;
  }

  // 过滤日志（新结构：使用 api_key_id 和 Unix 时间戳）
  let filteredLogs = banLogsData.filter(log => {
    // 查找 API Key 值用于搜索
    const apiKey = apiKeysData.find(k => k.id === log.api_key_id);
    const apiKeyValue = apiKey?.key || '';

    if (banLogFilter.apiKey && !apiKeyValue.toLowerCase().includes(banLogFilter.apiKey.toLowerCase()) &&
        !log.api_key_id.toLowerCase().includes(banLogFilter.apiKey.toLowerCase())) return false;

    // 判断操作类型（根据 unbanned_at 字段）
    const actionType = log.unbanned_at ? 'unban' : 'ban';
    if (banLogFilter.actionType !== 'all' && actionType !== banLogFilter.actionType) return false;

    if (banLogFilter.startDate) {
      const logDate = new Date((log.banned_at || 0) * 1000); // Unix秒转毫秒
      const startDate = new Date(banLogFilter.startDate);
      if (logDate < startDate) return false;
    }
    if (banLogFilter.endDate) {
      const logDate = new Date((log.banned_at || 0) * 1000);
      const endDate = new Date(banLogFilter.endDate);
      endDate.setHours(23, 59, 59, 999);
      if (logDate > endDate) return false;
    }
    return true;
  });

  // 分页
  banLogPagination.total = filteredLogs.length;
  const start = (banLogPagination.page - 1) * banLogPagination.pageSize;
  const end = start + banLogPagination.pageSize;
  const paginatedLogs = filteredLogs.slice(start, end);

  // 生成表格行
  const tableRows = paginatedLogs.map(log => {
    const isUnban = !!log.unbanned_at;
    const actionClass = isUnban ? 'action-unban' : 'action-ban';
    const actionText = isUnban ? '解封' : '封禁';
    const actionIcon = isUnban
      ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2v20M2 12h20"/></svg>'
      : '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2v20M2 12h20"/></svg>';

    // 查找 API Key 信息
    const apiKey = apiKeysData.find(k => k.id === log.api_key_id);
    const apiKeyDisplay = apiKey ? apiKey.key : log.api_key_id;

    // 计算封禁时长
    const durationSecs = log.banned_until - log.banned_at;
    const durationText = formatDurationSeconds(durationSecs);

    // 格式化时间
    const bannedTime = formatUnixTime(log.banned_at);

    return `
      <tr>
        <td><code class="apikey-code" title="${esc(apiKey?.key || log.api_key_id)}">${esc(apiKeyDisplay)}</code></td>
        <td><span class="action-badge ${actionClass}">${actionIcon} ${actionText}</span></td>
        <td class="reason-cell" title="${esc(log.reason)}">${esc(log.reason)}</td>
        <td>${bannedTime}</td>
        <td>${durationText}</td>
        <td><span class="operator-badge system">system</span></td>
      </tr>
    `;
  }).join('');

  // 统计（新结构）
  const totalBans = banLogsData.filter(l => !l.unbanned_at).length;
  const totalUnbans = banLogsData.filter(l => !!l.unbanned_at).length;

  // 生成分页
  const totalPages = Math.ceil(banLogPagination.total / banLogPagination.pageSize);
  const paginationHtml = generatePagination(banLogPagination.page, totalPages, 'banLog');

  panel.innerHTML = `
    <div class="banlogs-page">
      <div class="banlogs-header">
        <h2 class="banlogs-title">封禁日志</h2>
        <div class="banlogs-stats">
          <span class="stat-item">总记录: <strong>${banLogsData.length}</strong></span>
          <span class="stat-item">封禁: <strong class="text-danger">${totalBans}</strong></span>
          <span class="stat-item">解封: <strong class="text-success">${totalUnbans}</strong></span>
        </div>
      </div>

      <div class="banlogs-toolbar">
        <div class="filter-group">
          <select class="input select filter-select" onchange="setBanLogFilter('actionType', this.value)">
            <option value="all" ${banLogFilter.actionType === 'all' ? 'selected' : ''}>所有操作</option>
            <option value="ban" ${banLogFilter.actionType === 'ban' ? 'selected' : ''}>封禁</option>
            <option value="unban" ${banLogFilter.actionType === 'unban' ? 'selected' : ''}>解封</option>
          </select>
          <input type="date" class="input filter-date" placeholder="开始日期"
                 value="${banLogFilter.startDate}" onchange="setBanLogFilter('startDate', this.value)">
          <span class="date-separator">至</span>
          <input type="date" class="input filter-date" placeholder="结束日期"
                 value="${banLogFilter.endDate}" onchange="setBanLogFilter('endDate', this.value)">
        </div>
        <div class="search-box">
          <input type="text" class="input" placeholder="搜索 API Key..."
                 value="${esc(banLogFilter.apiKey)}" onchange="setBanLogFilter('apiKey', this.value)">
          <svg class="search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8"/>
            <path d="m21 21-4.3-4.3"/>
          </svg>
        </div>
      </div>

      <div class="banlogs-table-wrapper">
        <table class="banlogs-table">
          <thead>
            <tr>
              <th>API Key</th>
              <th>操作类型</th>
              <th>原因</th>
              <th>时间</th>
              <th>封禁时长</th>
              <th>操作者</th>
            </tr>
          </thead>
          <tbody>
            ${tableRows || '<tr><td colspan="6" class="empty-cell">暂无日志记录</td></tr>'}
          </tbody>
        </table>
      </div>

      ${paginationHtml}
    </div>
  `;
}

// 设置封禁日志过滤器
function setBanLogFilter(key, value) {
  banLogFilter[key] = value;
  banLogPagination.page = 1; // 重置到第一页
  renderBanLogs();
}

// 切换封禁日志分页
function changeBanLogPage(page) {
  banLogPagination.page = page;
  renderBanLogs();
}

// 生成分页 HTML
function generatePagination(currentPage, totalPages, prefix) {
  if (totalPages <= 1) return '';

  let pages = [];
  const maxVisible = 5;

  if (totalPages <= maxVisible) {
    for (let i = 1; i <= totalPages; i++) pages.push(i);
  } else {
    if (currentPage <= 3) {
      pages = [1, 2, 3, 4, '...', totalPages];
    } else if (currentPage >= totalPages - 2) {
      pages = [1, '...', totalPages - 3, totalPages - 2, totalPages - 1, totalPages];
    } else {
      pages = [1, '...', currentPage - 1, currentPage, currentPage + 1, '...', totalPages];
    }
  }

  const pageButtons = pages.map(p => {
    if (p === '...') return '<span class="pagination-ellipsis">...</span>';
    return `<button class="pagination-btn ${p === currentPage ? 'active' : ''}" onclick="changeBanLogPage(${p})">${p}</button>`;
  }).join('');

  return `
    <div class="pagination">
      <button class="pagination-btn" onclick="changeBanLogPage(${currentPage - 1})" ${currentPage === 1 ? 'disabled' : ''}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="m15 18-6-6 6-6"/>
        </svg>
      </button>
      ${pageButtons}
      <button class="pagination-btn" onclick="changeBanLogPage(${currentPage + 1})" ${currentPage === totalPages ? 'disabled' : ''}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="m9 18 6-6-6-6"/>
        </svg>
      </button>
      <span class="pagination-info">共 ${totalPages} 页</span>
    </div>
  `;
}

// 格式化秒数为可读格式
function formatDurationSeconds(seconds) {
  if (seconds < 60) return `${seconds}秒`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}分钟`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}小时`;
  return `${Math.floor(seconds / 86400)}天`;
}

// 格式化日期时间
function formatDateTime(isoString) {
  const date = new Date(isoString);
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  });
}

// 格式化 Unix 时间戳
function formatUnixTime(unixSeconds) {
  if (!unixSeconds) return '-';
  const date = new Date(unixSeconds * 1000);
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  });
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
  // 验证 route ID 唯一性
  if (value && isRouteIdExists(value, index)) {
    Toast.show(`路由 ID '${value}' 已存在，请使用其他名称`, 'error');
    // 恢复原来的值
    const input = document.querySelector(`.route-detail-body input[data-validate*="required"]`);
    if (input) {
      input.value = cfg.routes[index]?.id || '';
    }
    return;
  }

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

// 生成随机 API Key，格式: sk-<route>-<16-32字节随机字符串[0-9a-z]>
function generateApiKey(routeId) {
  const normalizedRoute = routeId.replace(/[^a-zA-Z0-9]/g, '-').toLowerCase() || 'route';
  const length = Math.floor(Math.random() * 17) + 16; // 16-32 字节
  const chars = '0123456789abcdefghijklmnopqrstuvwxyz';
  let randomStr = '';
  for (let i = 0; i < length; i++) {
    randomStr += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return `sk-${normalizedRoute}-${randomStr}`;
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

function parseTokenSource(i, value) {
  if (value.startsWith('Header:') || value.startsWith('header:')) {
    cfg.gateway_auth.token_sources[i] = { type: 'header', name: value.substring(7).trim() };
  }
}

// -- Gateway Config (整合认证、CORS、限流、并发控制) --
function renderGateway() {
  const panel = document.getElementById('tab-gateway');
  if (!panel) return;

  // 渲染各个子模块
  const authHtml = renderGatewayAuthSection();
  const corsHtml = renderGatewayCorsSection();
  const rateLimitHtml = renderGatewayRateLimitSection();
  const concurrencyHtml = renderGatewayConcurrencySection();
  const tokenStatsHtml = renderGatewayTokenStatsSection();

  panel.innerHTML = `
    <div class="gateway-config-layout">
      <!-- 左侧快速导航 -->
      <aside class="gateway-nav">
        <div class="gateway-nav-title">快速导航</div>
        <nav class="gateway-nav-list">
          <a href="#section-auth" class="gateway-nav-item active" data-section="auth">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
            </svg>
            认证配置
          </a>
          <a href="#section-cors" class="gateway-nav-item" data-section="cors">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"/>
              <path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
            </svg>
            CORS 配置
          </a>
          <a href="#section-ratelimit" class="gateway-nav-item" data-section="ratelimit">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"/>
              <polyline points="12 6 12 12 16 14"/>
            </svg>
            限流配置
          </a>
          <a href="#section-concurrency" class="gateway-nav-item" data-section="concurrency">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
            </svg>
            并发控制
          </a>
          <a href="#section-tokenstats" class="gateway-nav-item" data-section="tokenstats">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10"/>
              <path d="M12 6v6l4 2"/>
            </svg>
            Token统计
          </a>
        </nav>
      </aside>

      <!-- 右侧配置内容 -->
      <div class="gateway-config-content">
        <div id="section-auth" class="config-section">
          <h3 class="section-title">认证配置</h3>
          ${authHtml}
        </div>
        <div id="section-cors" class="config-section">
          <h3 class="section-title">CORS 配置</h3>
          ${corsHtml}
        </div>
        <div id="section-ratelimit" class="config-section">
          <h3 class="section-title">限流配置</h3>
          ${rateLimitHtml}
        </div>
        <div id="section-concurrency" class="config-section">
          <h3 class="section-title">并发控制配置</h3>
          ${concurrencyHtml}
        </div>
        <div id="section-tokenstats" class="config-section">
          <h3 class="section-title">Token统计配置</h3>
          ${tokenStatsHtml}
        </div>
      </div>
    </div>
  `;

  // 初始化导航交互
  initGatewayNav();
}

// 初始化网关配置导航
function initGatewayNav() {
  const navItems = document.querySelectorAll('.gateway-nav-item');
  const sections = document.querySelectorAll('.config-section');

  // 点击导航平滑滚动到对应区块
  navItems.forEach(item => {
    item.addEventListener('click', (e) => {
      e.preventDefault();
      const targetId = item.getAttribute('href').substring(1);
      const targetSection = document.getElementById(targetId);
      if (targetSection) {
        targetSection.scrollIntoView({ behavior: 'smooth', block: 'start' });
        // 更新活跃状态
        navItems.forEach(n => n.classList.remove('active'));
        item.classList.add('active');
      }
    });
  });

  // 滚动时自动高亮对应导航项
  const observerOptions = {
    root: null,
    rootMargin: '-20% 0px -60% 0px',
    threshold: 0
  };

  const observer = new IntersectionObserver((entries) => {
    entries.forEach(entry => {
      if (entry.isIntersecting) {
        const id = entry.target.id;
        navItems.forEach(item => {
          item.classList.toggle('active', item.getAttribute('href') === `#${id}`);
        });
      }
    });
  }, observerOptions);

  sections.forEach(section => observer.observe(section));
}

function renderGatewayAuthSection() {
  const auth = cfg.gateway_auth || { token_sources: [] };

  let srcHtml = (auth.token_sources || []).map((s, i) => {
    if (s.type === 'authorization_bearer') {
      return `<div class="token-row">
        <input class="input" value="Authorization Bearer" readonly />
        <button class="btn btn-danger btn-sm" onclick="cfg.gateway_auth.token_sources.splice(${i},1);renderGateway()">删除</button>
      </div>`;
    }
    return `<div class="token-row">
      <input class="input" value="Header: ${esc(s.name || '')}" onchange="parseTokenSource(${i}, this.value)" />
      <button class="btn btn-danger btn-sm" onclick="cfg.gateway_auth.token_sources.splice(${i},1);renderGateway()">删除</button>
    </div>`;
  }).join('');

  return `
    <div class="field">
      <label class="field-label">Token Sources</label>
      <div class="token-list">${srcHtml}</div>
      <div style="display:flex;gap:var(--space-2);margin-top:var(--space-2)">
        <button class="btn btn-secondary btn-sm" onclick="cfg.gateway_auth.token_sources.push({type:'authorization_bearer'});renderGateway()">+ Authorization Bearer</button>
        <button class="btn btn-secondary btn-sm" onclick="cfg.gateway_auth.token_sources.push({type:'header',name:'x-gw-token'});renderGateway()">+ 自定义 Header</button>
      </div>
    </div>
  `;
}

function renderGatewayCorsSection() {
  const cors = cfg.cors || { enabled: false, allow_origins: [], allow_headers: [], allow_methods: [], expose_headers: [] };
  if (!cfg.cors) cfg.cors = cors;

  return `
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

function renderGatewayRateLimitSection() {
  const rl = cfg.rate_limit;
  const enabled = !!rl;
  const perMinute = rl ? rl.per_minute : 120;

  return `
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

function renderGatewayConcurrencySection() {
  const cc = cfg.concurrency;
  const enabled = !!cc;
  const ds = cc ? cc.downstream_max_inflight : null;
  const us = cc ? cc.upstream_per_key_max_inflight : null;

  return `
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

function renderGatewayTokenStatsSection() {
  const ts = cfg.token_stats;
  const enabled = !!(ts && ts.enabled);
  const sqlite = ts ? ts.sqlite : null;

  return `
    <div class="field">
      <label class="toggle">
        <input type="checkbox" class="toggle-input" ${enabled ? 'checked' : ''} onchange="toggleTokenStats(this.checked)" />
        <span class="toggle-slider" aria-hidden="true"></span>
        <span class="toggle-label">启用Token统计</span>
      </label>
      <div class="field-help">启用后会统计每个API Key和路由的Token使用量，支持配额限制</div>
    </div>
    <div class="field">
      <label class="field-label">SQLite数据库路径</label>
      <input class="input" type="text" value="${sqlite ? sqlite.path : './data/token_stats.db'}" placeholder="./data/token_stats.db" ${enabled ? '' : 'disabled'} onchange="updateTokenStatsSqlitePath(this.value)" />
      <div class="field-help">Token统计数据存储的SQLite数据库文件路径</div>
    </div>
    <div class="field">
      <label class="field-label">批量写入间隔（秒）</label>
      <input class="input" type="number" value="${sqlite ? sqlite.flush_interval_secs : 60}" placeholder="60" min="1" ${enabled ? '' : 'disabled'} onchange="updateTokenStatsFlushInterval(+this.value)" />
      <div class="field-help">数据写入数据库的间隔时间（秒）</div>
    </div>
    <div class="field">
      <label class="field-label">批量大小</label>
      <input class="input" type="number" value="${sqlite ? sqlite.batch_size : 1000}" placeholder="1000" min="1" ${enabled ? '' : 'disabled'} onchange="updateTokenStatsBatchSize(+this.value)" />
      <div class="field-help">每次批量写入的记录数</div>
    </div>
  `;
}

function toggleTokenStats(enabled) {
  if (enabled) {
    cfg.token_stats = {
      enabled: true,
      sqlite: {
        path: './data/token_stats.db',
        flush_interval_secs: 60,
        batch_size: 1000
      }
    };
  } else {
    cfg.token_stats = null;
  }
  renderGateway();
}

function updateTokenStatsSqlitePath(path) {
  if (cfg.token_stats && cfg.token_stats.sqlite) {
    cfg.token_stats.sqlite.path = path;
  }
}

function updateTokenStatsFlushInterval(value) {
  if (cfg.token_stats && cfg.token_stats.sqlite) {
    cfg.token_stats.sqlite.flush_interval_secs = value;
  }
}

function updateTokenStatsBatchSize(value) {
  if (cfg.token_stats && cfg.token_stats.sqlite) {
    cfg.token_stats.sqlite.batch_size = value;
  }
}

function toggleRateLimit(enabled) {
  if (enabled) {
    cfg.rate_limit = { per_minute: 120 };
  } else {
    cfg.rate_limit = null;
  }
  renderGateway();
}

function toggleConcurrency(enabled) {
  if (enabled) {
    cfg.concurrency = { downstream_max_inflight: null, upstream_per_key_max_inflight: null };
  } else {
    cfg.concurrency = null;
  }
  renderGateway();
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
// 准备配置用于保存：转换 api_keys 格式
function prepareConfigForSave(config) {
  const prepared = JSON.parse(JSON.stringify(config));

  // Route-level api_keys have been removed - api_keys are now only managed globally
  // via cfg.api_keys (see API Keys management tab)

  return prepared;
}

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
    const preparedCfg = prepareConfigForSave(cfg);
    const res = await fetch(CONFIG.apiUrl, {
      method: 'PUT',
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: JSON.stringify(preparedCfg)
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
  if (!cfg) {
    Toast.show('请先加载配置', 'error');
    return;
  }

  const btn = document.querySelector('button[onclick="saveConfig()"]');
  const loading = new LoadingState(btn);
  loading.start();

  try {
    const preparedCfg = prepareConfigForSave(cfg);

    // 先应用当前编辑的配置到内存
    const applyRes = await fetch(CONFIG.apiUrl, {
      method: 'PUT',
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: JSON.stringify(preparedCfg)
    });
    const applyData = await applyRes.json();

    if (!applyRes.ok) {
      throw new Error('应用失败: ' + (applyData.error || applyRes.status));
    }

    // 然后保存到文件
    const saveRes = await fetch(CONFIG.saveUrl, {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + token }
    });
    const saveData = await saveRes.json();

    if (!saveRes.ok) {
      throw new Error('保存失败: ' + (saveData.error || saveRes.status));
    }

    // 更新保存状态显示
    const saveStatus = document.getElementById('saveStatus');
    if (saveStatus) {
      const now = new Date();
      saveStatus.textContent = `已保存 ${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}`;
    }
    Toast.show('配置已保存到文件', 'success');
  } catch (e) {
    Toast.show(e.message, 'error');
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
let apiKeySortBy = 'requests_24h';
let apiKeyOrder = 'desc';

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

function formatDurationMs(ms) {
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
        <td>${formatDurationMs(ip.latency_avg_ms)}</td>
        <td>${formatDurationMs(ip.latency_p99_ms)}</td>
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

// ===== APIKEY 维度排序 =====
function sortApiKeyTable(sortBy) {
  if (apiKeySortBy === sortBy) {
    apiKeyOrder = apiKeyOrder === 'desc' ? 'asc' : 'desc';
  } else {
    apiKeySortBy = sortBy;
    apiKeyOrder = 'desc';
  }
  renderMetrics();
}

function getSortIcon(column, currentSortBy, currentOrder) {
  if (currentSortBy !== column) {
    return '<span class="sort-icon sort-inactive">↕</span>';
  }
  return `<span class="sort-icon">${currentOrder === 'desc' ? '↓' : '↑'}</span>`;
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

  // 对 token 数据进行排序
  let sortedTokens = [...(metricsData.tokens || [])];
  sortedTokens.sort((a, b) => {
    let cmp = 0;
    switch (apiKeySortBy) {
      case 'route':
        cmp = (a.route_id || 'unknown').localeCompare(b.route_id || 'unknown');
        break;
      case 'apikey':
        cmp = a.token.localeCompare(b.token);
        break;
      case 'requests_1h':
        cmp = a.requests_1h - b.requests_1h;
        break;
      case 'requests_24h':
        cmp = a.requests_24h - b.requests_24h;
        break;
    }
    return apiKeyOrder === 'desc' ? -cmp : cmp;
  });

  const tokenRows = sortedTokens.map(t => `
    <tr>
      <td>${esc(t.route_id || 'unknown')}</td>
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
        <h3 class="chart-title">APIKEY 使用量分布 (24h)</h3>
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
      <h3>APIKEY 维度</h3>
      <div class="metrics-table-wrapper">
        <table class="metrics-table" id="apiKeyTable">
          <thead>
            <tr>
              <th onclick="sortApiKeyTable('route')" class="sortable" data-sort="route">Route ${getSortIcon('route', apiKeySortBy, apiKeyOrder)}</th>
              <th onclick="sortApiKeyTable('apikey')" class="sortable" data-sort="apikey">APIKEY ${getSortIcon('apikey', apiKeySortBy, apiKeyOrder)}</th>
              <th onclick="sortApiKeyTable('requests_1h')" class="sortable" data-sort="requests_1h">请求 (1h) ${getSortIcon('requests_1h', apiKeySortBy, apiKeyOrder)}</th>
              <th onclick="sortApiKeyTable('requests_24h')" class="sortable" data-sort="requests_24h">请求 (24h) ${getSortIcon('requests_24h', apiKeySortBy, apiKeyOrder)}</th>
            </tr>
          </thead>
          <tbody>${tokenRows || '<tr><td colspan="4" class="empty-cell">暂无数据</td></tr>'}</tbody>
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

  // APIKEY 饼图
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

// ===== 初始化 =====
document.addEventListener('DOMContentLoaded', async () => {
  // 检查登录状态
  if (!checkAuth()) {
    return;
  }

  initTabs();

  // 自动加载配置和监控数据
  await loadConfig();
  loadMetrics();

  // 根据当前活动的标签页加载数据
  const activeTab = document.querySelector('.tab.active');
  if (activeTab) {
    const tabId = activeTab.dataset.tab;
    if (tabId === 'apikeys') {
      await loadApiKeys();
      renderApiKeys();
    } else if (tabId === 'banlogs') {
      await loadBanLogs();
      renderBanLogs();
    }
  }

  // 监控页面自动刷新
  setInterval(() => {
    const metricsPanel = document.getElementById('tab-metrics');
    if (metricsPanel && metricsPanel.classList.contains('active')) {
      loadMetrics();
      loadIPMetrics();
    }
  }, 30000); // 30秒刷新一次
});

// ===== 多选下拉框组件函数 =====

// 切换下拉框展开/收起
function toggleMultiSelect(trigger) {
  const multiSelect = trigger.closest('.multi-select');
  const dropdown = multiSelect.querySelector('.multi-select-dropdown');
  const isOpen = dropdown.style.display !== 'none';

  // 关闭所有其他下拉框
  document.querySelectorAll('.multi-select-dropdown').forEach(d => {
    d.style.display = 'none';
    d.closest('.multi-select').querySelector('.multi-select-trigger').classList.remove('active');
  });

  if (!isOpen) {
    dropdown.style.display = 'flex';
    trigger.classList.add('active');
    // 聚焦搜索框
    setTimeout(() => {
      const searchInput = dropdown.querySelector('.multi-select-search input');
      if (searchInput) searchInput.focus();
    }, 10);
  } else {
    dropdown.style.display = 'none';
    trigger.classList.remove('active');
  }
}

// 搜索过滤路由选项
function filterRouteOptions(input) {
  const filter = input.value.toLowerCase();
  const options = input.closest('.multi-select-dropdown').querySelectorAll('.multi-select-option:not(.special)');

  options.forEach(option => {
    const text = option.textContent.toLowerCase();
    option.style.display = text.includes(filter) ? 'flex' : 'none';
  });
}

// 选择/取消选择路由选项
function selectRouteOption(option) {
  if (!option) return;
  const value = option.dataset.value;
  const multiSelect = option.closest('.multi-select');
  if (!multiSelect) return;
  const isSpecial = option.classList.contains('special');
  const hiddenInput = multiSelect.parentElement.querySelector('.apikey-routes-input');
  if (!hiddenInput) return;
  let selectedRoutes = JSON.parse(hiddenInput.value || '[]');
  const allRoutesOption = multiSelect.querySelector('.multi-select-option.special');

  if (isSpecial) {
    // 点击"所有路由"
    if (option.classList.contains('selected')) {
      // 已选中，取消选择（变成空选，表示所有路由）
      option.classList.remove('selected');
      selectedRoutes = [];
    } else {
      // 未选中，选择"所有路由"，清空所有具体路由
      option.classList.add('selected');
      multiSelect.querySelectorAll('.multi-select-option:not(.special)').forEach(opt => {
        opt.classList.remove('selected');
      });
      selectedRoutes = [];
    }
  } else {
    // 点击具体路由，自动取消"所有路由"
    if (allRoutesOption) {
      allRoutesOption.classList.remove('selected');
    }

    if (option.classList.contains('selected')) {
      // 取消选择
      option.classList.remove('selected');
      selectedRoutes = selectedRoutes.filter(r => r !== value);
    } else {
      // 选择
      option.classList.add('selected');
      if (!selectedRoutes.includes(value)) {
        selectedRoutes.push(value);
      }
    }
  }

  // 更新隐藏输入框
  hiddenInput.value = JSON.stringify(selectedRoutes);

  // 更新触发器显示
  updateMultiSelectDisplay(multiSelect, selectedRoutes);
}

// 移除已选标签
function removeRouteTag(removeBtn) {
  if (!removeBtn) return;
  const tag = removeBtn.closest('.multi-select-tag');
  if (!tag) return;
  const value = tag.dataset.value;
  const multiSelect = tag.closest('.multi-select');
  if (!multiSelect) return;
  const hiddenInput = multiSelect.parentElement.querySelector('.apikey-routes-input');
  if (!hiddenInput) return;
  let selectedRoutes = JSON.parse(hiddenInput.value || '[]');

  selectedRoutes = selectedRoutes.filter(r => r !== value);
  hiddenInput.value = JSON.stringify(selectedRoutes);

  // 更新选项状态
  const option = multiSelect.querySelector(`.multi-select-option[data-value="${value}"]`);
  if (option) {
    option.classList.remove('selected');
  }

  updateMultiSelectDisplay(multiSelect, selectedRoutes);
}

// 更新多选框显示
function updateMultiSelectDisplay(multiSelect, selectedRoutes) {
  const valuesContainer = multiSelect.querySelector('.multi-select-values');
  const routes = cfg?.routes || [];

  // 根据实际选择渲染标签
  if (selectedRoutes.length === 0) {
    valuesContainer.innerHTML = '<span class="multi-select-tag all-routes">所有路由</span>';
  } else {
    valuesContainer.innerHTML = selectedRoutes.map(r => {
      const route = routes.find(rt => rt.id === r);
      return `<span class="multi-select-tag" data-value="${esc(r)}">${esc(route?.id || r)}<span class="multi-select-tag-remove" onclick="event.stopPropagation(); removeRouteTag(this)"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></span></span>`;
    }).join('');
  }

  // 同步选项的选中状态
  const allRoutesOption = multiSelect.querySelector('.multi-select-option.special');
  const otherOptions = multiSelect.querySelectorAll('.multi-select-option:not(.special)');

  if (allRoutesOption) {
    // 没有选择具体路由时，"所有路由"视为选中
    allRoutesOption.classList.toggle('selected', selectedRoutes.length === 0);
  }

  // 同步具体路由选项的选中状态
  otherOptions.forEach(opt => {
    const routeValue = opt.dataset.value;
    opt.classList.toggle('selected', selectedRoutes.includes(routeValue));
  });
}

// 点击外部关闭下拉框
document.addEventListener('click', (e) => {
  if (!e.target.closest('.multi-select')) {
    document.querySelectorAll('.multi-select-dropdown').forEach(d => {
      d.style.display = 'none';
      d.closest('.multi-select').querySelector('.multi-select-trigger').classList.remove('active');
    });
  }
});

// ===== 主题管理 =====
const THEME_KEY = 'ai_gateway_theme';

// 初始化主题
function initTheme() {
  const savedTheme = localStorage.getItem(THEME_KEY) || 'light';
  applyTheme(savedTheme);
  updateThemeToggleUI(savedTheme);
}

// 设置主题
function setTheme(theme) {
  if (theme !== 'light' && theme !== 'dark' && theme !== 'auto') {
    theme = 'light';
  }

  localStorage.setItem(THEME_KEY, theme);
  applyTheme(theme);
  updateThemeToggleUI(theme);

  // 显示提示
  const themeNames = {
    light: '亮色模式',
    dark: '暗色模式',
    auto: '跟随系统'
  };
  Toast.show(`已切换到${themeNames[theme]}`, 'success', 2000);
}

// 应用主题
function applyTheme(theme) {
  document.documentElement.setAttribute('data-theme', theme);

  // 如果是自动模式，需要检测系统主题并设置 Chart.js 主题
  if (theme === 'auto') {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    updateChartsTheme(prefersDark ? 'dark' : 'light');
  } else {
    updateChartsTheme(theme);
  }
}

// 循环切换主题
function cycleTheme() {
  const currentTheme = localStorage.getItem(THEME_KEY) || 'light';
  const themeOrder = ['light', 'dark', 'auto'];
  const currentIndex = themeOrder.indexOf(currentTheme);
  const nextTheme = themeOrder[(currentIndex + 1) % themeOrder.length];
  setTheme(nextTheme);
}

// 更新主题切换按钮UI（单按钮模式，通过CSS显示对应图标）
function updateThemeToggleUI(theme) {
  // 单按钮模式下，通过data-theme属性控制图标显示
  // 这里可以添加额外的tooltip更新逻辑
  const themeNames = {
    light: '亮色模式（点击切换）',
    dark: '暗色模式（点击切换）',
    auto: '跟随系统（点击切换）'
  };
  const btn = document.getElementById('themeToggleBtn');
  if (btn) {
    btn.title = themeNames[theme];
  }
}

// 更新图表主题（Chart.js）
function updateChartsTheme(theme) {
  // 设置 Chart.js 全局默认配置
  const isDark = theme === 'dark';
  const textColor = isDark ? '#8b949e' : '#475569';
  const gridColor = isDark ? '#30363d' : '#e2e8f0';

  // 如果 Chart.js 已加载
  if (typeof Chart !== 'undefined') {
    Chart.defaults.color = textColor;
    Chart.defaults.borderColor = gridColor;
    Chart.defaults.backgroundColor = isDark ? '#1e293b' : '#ffffff';

    // 更新所有已存在的图表（Chart.js v4.x: instances 是对象）
    for (const chart of Object.values(Chart.instances)) {
      if (chart.options.scales) {
        Object.values(chart.options.scales).forEach(scale => {
          if (scale.ticks) scale.ticks.color = textColor;
          if (scale.grid) scale.grid.color = gridColor;
        });
      }
      if (chart.options.plugins && chart.options.plugins.legend) {
        chart.options.plugins.legend.labels = chart.options.plugins.legend.labels || {};
        chart.options.plugins.legend.labels.color = textColor;
      }
      chart.update('none');
    }
  }
}

// 监听系统主题变化
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
  const currentTheme = localStorage.getItem(THEME_KEY) || 'light';
  if (currentTheme === 'auto') {
    updateChartsTheme(e.matches ? 'dark' : 'light');
    // 重新渲染图表以适应新主题
    if (typeof renderMetricsCharts === 'function') {
      renderMetricsCharts();
    }
  }
});

// 页面加载时初始化主题
document.addEventListener('DOMContentLoaded', initTheme);

// ===== Token 统计管理 =====
let tokenStatsData = {
  summary: null,
  apiKeys: [],
  routes: []
};
let tokenStatsFilter = {
  window: 'day', // day, week, month
  apiKey: '',
  route: ''
};

// 从服务器加载 Token 统计数据
async function loadTokenStats() {
  const token = getToken();
  if (!token) return null;

  const prefix = window.CONFIG?.adminPrefix || '/admin';

  try {
    // 加载汇总数据
    const summaryUrl = `${prefix}/api/token-stats/summary?window=${tokenStatsFilter.window}`;
    const summaryResponse = await fetch(summaryUrl, {
      headers: { 'Authorization': `Bearer ${token}` }
    });

    if (!summaryResponse.ok) {
      if (summaryResponse.status === 401) {
        logout();
        return null;
      }
      throw new Error(`HTTP ${summaryResponse.status}`);
    }

    const summaryData = await summaryResponse.json();
    tokenStatsData.summary = summaryData;
    tokenStatsData.apiKeys = summaryData.api_keys || [];
    tokenStatsData.routes = summaryData.routes || [];

    return tokenStatsData;
  } catch (err) {
    console.error('Failed to load token stats:', err);
    return null;
  }
}

// 加载单个 API Key 的 Token 统计详情
async function loadApiKeyTokenStats(apiKeyId) {
  const token = getToken();
  if (!token) return null;

  const prefix = window.CONFIG?.adminPrefix || '/admin';
  const url = `${prefix}/api/token-stats/keys/${apiKeyId}?window=${tokenStatsFilter.window}`;

  try {
    const response = await fetch(url, {
      headers: { 'Authorization': `Bearer ${token}` }
    });

    if (!response.ok) {
      if (response.status === 401) {
        logout();
        return null;
      }
      throw new Error(`HTTP ${response.status}`);
    }

    return await response.json();
  } catch (err) {
    console.error('Failed to load API Key token stats:', err);
    return null;
  }
}

// 加载单个 Route 的 Token 统计详情
async function loadRouteTokenStats(routeId) {
  const token = getToken();
  if (!token) return null;

  const prefix = window.CONFIG?.adminPrefix || '/admin';
  const url = `${prefix}/api/token-stats/routes/${routeId}?window=${tokenStatsFilter.window}`;

  try {
    const response = await fetch(url, {
      headers: { 'Authorization': `Bearer ${token}` }
    });

    if (!response.ok) {
      if (response.status === 401) {
        logout();
        return null;
      }
      throw new Error(`HTTP ${response.status}`);
    }

    return await response.json();
  } catch (err) {
    console.error('Failed to load Route token stats:', err);
    return null;
  }
}

// Token 统计图表实例
let tokenTrendChart = null;
let tokenDistributionChart = null;
let tokenRatioChart = null;

// 表格排序状态
let apiKeySortState = { column: 'totalTokens', order: 'desc' };
let routeSortState = { column: 'totalTokens', order: 'desc' };

// 渲染 Token 统计页面
function renderTokenStats() {
  const panel = document.getElementById('tab-tokenstats');
  if (!panel) return;

  const hasData = tokenStatsData.summary && (tokenStatsData.apiKeys.length > 0 || tokenStatsData.routes.length > 0);

  const windowLabels = {
    day: '今日',
    week: '本周',
    month: '本月'
  };

  panel.innerHTML = `
    ${!hasData ? `
      <div class="metrics-header">
        <div class="metrics-title">
          <h2>Token 统计</h2>
        </div>
        <div style="display: flex; gap: var(--space-3);">
          <select class="input select" id="token-stats-window" onchange="changeTokenStatsWindow(this.value)">
            <option value="day" ${tokenStatsFilter.window === 'day' ? 'selected' : ''}>今日</option>
            <option value="week" ${tokenStatsFilter.window === 'week' ? 'selected' : ''}>本周</option>
            <option value="month" ${tokenStatsFilter.window === 'month' ? 'selected' : ''}>本月</option>
          </select>
          <button class="btn btn-primary btn-sm" onclick="refreshTokenStats()">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
              <path d="M3 3v5h5"/>
              <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/>
              <path d="M16 21h5v-5"/>
            </svg>
            刷新
          </button>
        </div>
      </div>
      <div class="card">
        <div class="card-body">
          <div class="empty-state">
            <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <circle cx="12" cy="12" r="10"/>
              <path d="M12 6v6l4 2"/>
            </svg>
            <p>暂无 Token 统计数据</p>
            <p class="text-secondary">请确保已启用 Token 统计功能并已有请求记录</p>
          </div>
        </div>
      </div>
    ` : `
      <!-- 页面头部 -->
      <div class="metrics-header">
        <div class="metrics-title">
          <h2>Token 统计</h2>
          <span class="metrics-time">${windowLabels[tokenStatsFilter.window]}数据</span>
        </div>
        <div style="display: flex; gap: var(--space-3);">
          <select class="input select" id="token-stats-window" onchange="changeTokenStatsWindow(this.value)">
            <option value="day" ${tokenStatsFilter.window === 'day' ? 'selected' : ''}>今日</option>
            <option value="week" ${tokenStatsFilter.window === 'week' ? 'selected' : ''}>本周</option>
            <option value="month" ${tokenStatsFilter.window === 'month' ? 'selected' : ''}>本月</option>
          </select>
          <button class="btn btn-primary btn-sm" onclick="refreshTokenStats()">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
              <path d="M3 3v5h5"/>
              <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/>
              <path d="M16 21h5v-5"/>
            </svg>
            刷新
          </button>
        </div>
      </div>

      <!-- 图表区域 -->
      <div class="token-stats-charts">
        <div class="chart-container">
          <h3 class="chart-title">Token 使用趋势</h3>
          <canvas id="tokenTrendChart"></canvas>
        </div>
        <div class="chart-container">
          <h3 class="chart-title">API Key 分布</h3>
          <canvas id="tokenDistributionChart"></canvas>
        </div>
      </div>

      <!-- 总览卡片 -->
      ${renderTokenStatsOverview()}

      <!-- API Key 统计 -->
      <div class="metrics-section">
        <div class="token-stats-section-header">
          <h3>API Key 统计</h3>
          <span class="token-stats-count">共 ${tokenStatsData.apiKeys.length} 个</span>
        </div>
        <div class="metrics-table-wrapper">
          ${renderApiKeyTokenStatsTable()}
        </div>
      </div>

      <!-- Route 统计 -->
      <div class="metrics-section">
        <div class="token-stats-section-header">
          <h3>路由统计</h3>
          <span class="token-stats-count">共 ${tokenStatsData.routes.length} 个</span>
        </div>
        <div class="metrics-table-wrapper">
          ${renderRouteTokenStatsTable()}
        </div>
      </div>
    `}
  `;

  // 如果有数据，渲染图表
  if (hasData) {
    initTokenStatsCharts();
  }
}

// 渲染 Token 统计概览 - 使用监控页面 metrics-cards 风格
function renderTokenStatsOverview() {
  const apiKeys = tokenStatsData.apiKeys;
  const routes = tokenStatsData.routes;

  // 计算总计
  let totalInput = 0, totalOutput = 0, totalRequests = 0;

  if (tokenStatsFilter.window === 'day') {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.today_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.today_output_tokens || 0), 0);
    totalRequests = apiKeys.reduce((sum, k) => sum + (k.request_count_today || 0), 0);
  } else if (tokenStatsFilter.window === 'week') {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.week_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.week_output_tokens || 0), 0);
    totalRequests = apiKeys.reduce((sum, k) => sum + (k.request_count_week || 0), 0);
  } else {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.month_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.month_output_tokens || 0), 0);
    totalRequests = apiKeys.reduce((sum, k) => sum + (k.request_count_month || 0), 0);
  }

  const totalTokens = totalInput + totalOutput;
  const inputPercent = totalTokens > 0 ? (totalInput / totalTokens * 100).toFixed(1) : 0;
  const outputPercent = totalTokens > 0 ? (totalOutput / totalTokens * 100).toFixed(1) : 0;

  return `
    <div class="metrics-cards">
      <div class="metric-card">
        <div class="metric-label">总 Token 数</div>
        <div class="metric-value">${formatCompactNumber(totalTokens)}</div>
        <div class="token-ratio-bar">
          <div class="token-ratio-input" style="width: ${inputPercent}%"></div>
          <div class="token-ratio-output" style="width: ${outputPercent}%"></div>
        </div>
        <div class="token-ratio-legend">
          <span class="legend-item"><span class="legend-dot input"></span>Input ${inputPercent}%</span>
          <span class="legend-item"><span class="legend-dot output"></span>Output ${outputPercent}%</span>
        </div>
      </div>
      <div class="metric-card">
        <div class="metric-label">Input Tokens</div>
        <div class="metric-value" style="color: var(--success-600);">${formatCompactNumber(totalInput)}</div>
      </div>
      <div class="metric-card">
        <div class="metric-label">Output Tokens</div>
        <div class="metric-value" style="color: var(--warning-600);">${formatCompactNumber(totalOutput)}</div>
      </div>
      <div class="metric-card">
        <div class="metric-label">请求数</div>
        <div class="metric-value" style="color: var(--primary-600);">${formatCompactNumber(totalRequests)}</div>
      </div>
    </div>
  `;
}

// 渲染 API Key Token 统计表格 - 增强版，支持排序、排名、进度条
function renderApiKeyTokenStatsTable() {
  const apiKeys = [...tokenStatsData.apiKeys]; // 复制数组避免修改原数据

  if (apiKeys.length === 0) {
    return '<div class="empty-state"><p>暂无 API Key 统计数据</p></div>';
  }

  // 计算所有数据并排序
  const enrichedKeys = apiKeys.map(key => {
    let inputTokens, outputTokens, totalTokens, requestCount;

    if (tokenStatsFilter.window === 'day') {
      inputTokens = key.today_input_tokens || 0;
      outputTokens = key.today_output_tokens || 0;
      totalTokens = key.today_total_tokens || 0;
      requestCount = key.request_count_today || 0;
    } else if (tokenStatsFilter.window === 'week') {
      inputTokens = key.week_input_tokens || 0;
      outputTokens = key.week_output_tokens || 0;
      totalTokens = key.week_total_tokens || 0;
      requestCount = key.request_count_week || 0;
    } else {
      inputTokens = key.month_input_tokens || 0;
      outputTokens = key.month_output_tokens || 0;
      totalTokens = key.month_total_tokens || 0;
      requestCount = key.request_count_month || 0;
    }

    return {
      ...key,
      inputTokens,
      outputTokens,
      totalTokens,
      requestCount
    };
  });

  // 根据当前排序列排序
  enrichedKeys.sort((a, b) => {
    const col = apiKeySortState.column;
    const order = apiKeySortState.order === 'asc' ? 1 : -1;

    switch (col) {
      case 'api_key_id':
        return order * a.api_key_id.localeCompare(b.api_key_id);
      case 'inputTokens':
        return order * (a.inputTokens - b.inputTokens);
      case 'outputTokens':
        return order * (a.outputTokens - b.outputTokens);
      case 'totalTokens':
        return order * (a.totalTokens - b.totalTokens);
      case 'requestCount':
        return order * (a.requestCount - b.requestCount);
      default:
        return 0;
    }
  });

  // 计算最大总 Token 数用于进度条
  const maxTotalTokens = Math.max(...enrichedKeys.map(k => k.totalTokens), 1);

  const rows = enrichedKeys.map((key, index) => {
    const rank = index + 1;
    const rankClass = rank === 1 ? 'top1' : rank === 2 ? 'top2' : rank === 3 ? 'top3' : '';
    const progressPercent = (key.totalTokens / maxTotalTokens * 100).toFixed(1);

    // 计算 Input/Output 比例
    const inputPercent = key.totalTokens > 0 ? (key.inputTokens / key.totalTokens * 100).toFixed(0) : 50;
    const outputPercent = key.totalTokens > 0 ? (key.outputTokens / key.totalTokens * 100).toFixed(0) : 50;

    return `
      <tr class="${rank <= 3 ? 'top-row' : ''}">
        <td class="rank-cell">
          ${rankClass ? `<span class="rank-badge ${rankClass}">${rank}</span>` : `<span class="rank-badge">${rank}</span>`}
        </td>
        <td><code class="apikey-code" title="${esc(key.api_key_id)}">${esc(key.api_key || key.api_key_id)}</code></td>
        <td>
          <div class="token-cell">
            <span class="token-value">${formatNumber(key.inputTokens)}</span>
            <div class="token-usage-bar">
              <div class="token-usage-bar-input" style="width: ${inputPercent}%"></div>
              <div class="token-usage-bar-output" style="width: ${outputPercent}%"></div>
            </div>
          </div>
        </td>
        <td>
          <div class="token-cell">
            <span class="token-value">${formatNumber(key.outputTokens)}</span>
          </div>
        </td>
        <td>
          <div class="token-total-cell">
            <strong>${formatNumber(key.totalTokens)}</strong>
            <div class="token-progress-bg">
              <div class="token-progress-bar" style="width: ${progressPercent}%"></div>
            </div>
          </div>
        </td>
        <td>${formatNumber(key.requestCount)}</td>
        <td>
          <button class="btn btn-sm btn-ghost" onclick="showApiKeyTokenDetail('${esc(key.api_key_id)}')">
            详情
          </button>
        </td>
      </tr>
    `;
  }).join('');

  return `
    <table class="metrics-table token-stats-table">
      <thead>
        <tr>
          <th class="rank-header">#</th>
          <th onclick="sortApiKeyTokenStats('api_key_id')" class="sortable ${apiKeySortState.column === 'api_key_id' ? 'sorted' : ''}">
            API Key ${getTokenStatsSortIcon('api_key_id', apiKeySortState.column, apiKeySortState.order)}
          </th>
          <th onclick="sortApiKeyTokenStats('inputTokens')" class="sortable ${apiKeySortState.column === 'inputTokens' ? 'sorted' : ''}">
            Input Tokens ${getTokenStatsSortIcon('inputTokens', apiKeySortState.column, apiKeySortState.order)}
          </th>
          <th onclick="sortApiKeyTokenStats('outputTokens')" class="sortable ${apiKeySortState.column === 'outputTokens' ? 'sorted' : ''}">
            Output Tokens ${getTokenStatsSortIcon('outputTokens', apiKeySortState.column, apiKeySortState.order)}
          </th>
          <th onclick="sortApiKeyTokenStats('totalTokens')" class="sortable ${apiKeySortState.column === 'totalTokens' ? 'sorted' : ''}">
            总 Token 数 ${getTokenStatsSortIcon('totalTokens', apiKeySortState.column, apiKeySortState.order)}
          </th>
          <th onclick="sortApiKeyTokenStats('requestCount')" class="sortable ${apiKeySortState.column === 'requestCount' ? 'sorted' : ''}">
            请求数 ${getTokenStatsSortIcon('requestCount', apiKeySortState.column, apiKeySortState.order)}
          </th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        ${rows}
      </tbody>
    </table>
  `;
}

// 渲染 Route Token 统计表格 - 增强版，支持排序、排名、进度条
function renderRouteTokenStatsTable() {
  const routes = [...tokenStatsData.routes]; // 复制数组避免修改原数据

  if (routes.length === 0) {
    return '<div class="empty-state"><p>暂无路由统计数据</p></div>';
  }

  // 计算所有数据并排序
  const enrichedRoutes = routes.map(route => {
    let inputTokens, outputTokens, totalTokens, requestCount;

    if (tokenStatsFilter.window === 'day') {
      inputTokens = route.today_input_tokens || 0;
      outputTokens = route.today_output_tokens || 0;
      totalTokens = route.today_total_tokens || 0;
      requestCount = route.request_count_today || 0;
    } else if (tokenStatsFilter.window === 'week') {
      inputTokens = route.week_input_tokens || 0;
      outputTokens = route.week_output_tokens || 0;
      totalTokens = route.week_total_tokens || 0;
      requestCount = route.request_count_week || 0;
    } else {
      inputTokens = route.month_input_tokens || 0;
      outputTokens = route.month_output_tokens || 0;
      totalTokens = route.month_total_tokens || 0;
      requestCount = route.request_count_month || 0;
    }

    return {
      ...route,
      inputTokens,
      outputTokens,
      totalTokens,
      requestCount
    };
  });

  // 根据当前排序列排序
  enrichedRoutes.sort((a, b) => {
    const col = routeSortState.column;
    const order = routeSortState.order === 'asc' ? 1 : -1;

    switch (col) {
      case 'route_id':
        return order * a.route_id.localeCompare(b.route_id);
      case 'inputTokens':
        return order * (a.inputTokens - b.inputTokens);
      case 'outputTokens':
        return order * (a.outputTokens - b.outputTokens);
      case 'totalTokens':
        return order * (a.totalTokens - b.totalTokens);
      case 'requestCount':
        return order * (a.requestCount - b.requestCount);
      default:
        return 0;
    }
  });

  // 计算最大总 Token 数用于进度条
  const maxTotalTokens = Math.max(...enrichedRoutes.map(r => r.totalTokens), 1);

  const rows = enrichedRoutes.map((route, index) => {
    const rank = index + 1;
    const rankClass = rank === 1 ? 'top1' : rank === 2 ? 'top2' : rank === 3 ? 'top3' : '';
    const progressPercent = (route.totalTokens / maxTotalTokens * 100).toFixed(1);

    // 计算 Input/Output 比例
    const inputPercent = route.totalTokens > 0 ? (route.inputTokens / route.totalTokens * 100).toFixed(0) : 50;
    const outputPercent = route.totalTokens > 0 ? (route.outputTokens / route.totalTokens * 100).toFixed(0) : 50;

    return `
      <tr class="${rank <= 3 ? 'top-row' : ''}">
        <td class="rank-cell">
          ${rankClass ? `<span class="rank-badge ${rankClass}">${rank}</span>` : `<span class="rank-badge">${rank}</span>`}
        </td>
        <td><code>${esc(route.route_id)}</code></td>
        <td>
          <div class="token-cell">
            <span class="token-value">${formatNumber(route.inputTokens)}</span>
            <div class="token-usage-bar">
              <div class="token-usage-bar-input" style="width: ${inputPercent}%"></div>
              <div class="token-usage-bar-output" style="width: ${outputPercent}%"></div>
            </div>
          </div>
        </td>
        <td>
          <div class="token-cell">
            <span class="token-value">${formatNumber(route.outputTokens)}</span>
          </div>
        </td>
        <td>
          <div class="token-total-cell">
            <strong>${formatNumber(route.totalTokens)}</strong>
            <div class="token-progress-bg">
              <div class="token-progress-bar" style="width: ${progressPercent}%"></div>
            </div>
          </div>
        </td>
        <td>${formatNumber(route.requestCount)}</td>
        <td>
          <button class="btn btn-sm btn-ghost" onclick="showRouteTokenDetail('${esc(route.route_id)}')">
            详情
          </button>
        </td>
      </tr>
    `;
  }).join('');

  return `
    <table class="metrics-table token-stats-table">
      <thead>
        <tr>
          <th class="rank-header">#</th>
          <th onclick="sortRouteTokenStats('route_id')" class="sortable ${routeSortState.column === 'route_id' ? 'sorted' : ''}">
            路由 ID ${getTokenStatsSortIcon('route_id', routeSortState.column, routeSortState.order)}
          </th>
          <th onclick="sortRouteTokenStats('inputTokens')" class="sortable ${routeSortState.column === 'inputTokens' ? 'sorted' : ''}">
            Input Tokens ${getTokenStatsSortIcon('inputTokens', routeSortState.column, routeSortState.order)}
          </th>
          <th onclick="sortRouteTokenStats('outputTokens')" class="sortable ${routeSortState.column === 'outputTokens' ? 'sorted' : ''}">
            Output Tokens ${getTokenStatsSortIcon('outputTokens', routeSortState.column, routeSortState.order)}
          </th>
          <th onclick="sortRouteTokenStats('totalTokens')" class="sortable ${routeSortState.column === 'totalTokens' ? 'sorted' : ''}">
            总 Token 数 ${getTokenStatsSortIcon('totalTokens', routeSortState.column, routeSortState.order)}
          </th>
          <th onclick="sortRouteTokenStats('requestCount')" class="sortable ${routeSortState.column === 'requestCount' ? 'sorted' : ''}">
            请求数 ${getTokenStatsSortIcon('requestCount', routeSortState.column, routeSortState.order)}
          </th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        ${rows}
      </tbody>
    </table>
  `;
}

// 渲染 Token 统计图表 - 使用 Chart.js
function initTokenStatsCharts() {
  const apiKeys = tokenStatsData.apiKeys;
  if (!apiKeys || apiKeys.length === 0) return;

  // 销毁旧图表
  if (tokenTrendChart) {
    tokenTrendChart.destroy();
  }
  if (tokenDistributionChart) {
    tokenDistributionChart.destroy();
  }

  // 使用后端返回的真实时间序列数据
  const timeSeries = tokenStatsData.summary?.time_series || [];

  // 准备趋势图数据
  let trendLabels, inputData, outputData;

  if (timeSeries.length > 0) {
    // 使用真实数据，根据用户本地时区格式化时间标签
    trendLabels = timeSeries.map(p => formatTimeSeriesLabel(p.timestamp, tokenStatsFilter.window));
    inputData = timeSeries.map(p => p.input_tokens);
    outputData = timeSeries.map(p => p.output_tokens);
  } else {
    // 无数据时显示空的时间框架
    trendLabels = generateTrendLabels();
    inputData = new Array(trendLabels.length).fill(0);
    outputData = new Array(trendLabels.length).fill(0);
  }

  // Token 趋势图 (柱状图)
  const trendCtx = document.getElementById('tokenTrendChart');
  if (trendCtx) {
    tokenTrendChart = new Chart(trendCtx, {
      type: 'bar',
      data: {
        labels: trendLabels,
        datasets: [
          {
            label: 'Input Tokens',
            data: inputData,
            backgroundColor: 'rgba(16, 185, 129, 0.8)',
            borderColor: 'rgba(16, 185, 129, 1)',
            borderWidth: 1,
            borderRadius: 4
          },
          {
            label: 'Output Tokens',
            data: outputData,
            backgroundColor: 'rgba(245, 158, 11, 0.8)',
            borderColor: 'rgba(245, 158, 11, 1)',
            borderWidth: 1,
            borderRadius: 4
          }
        ]
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        plugins: {
          legend: {
            position: 'top',
            labels: {
              usePointStyle: true,
              padding: 15
            }
          }
        },
        scales: {
          y: {
            beginAtZero: true,
            grid: {
              color: 'rgba(0, 0, 0, 0.05)'
            },
            ticks: {
              callback: function(value) {
                return formatCompactNumber(value);
              }
            }
          },
          x: {
            grid: {
              display: false
            }
          }
        }
      }
    });
  }

  // API Key 分布图 (环形图) - Top 5 + Others
  const distributionCtx = document.getElementById('tokenDistributionChart');
  if (distributionCtx) {
    const topKeys = [...apiKeys]
      .map(key => ({
        id: key.api_key_id,
        total: tokenStatsFilter.window === 'day'
          ? (key.today_total_tokens || 0)
          : tokenStatsFilter.window === 'week'
            ? (key.week_total_tokens || 0)
            : (key.month_total_tokens || 0)
      }))
      .sort((a, b) => b.total - a.total)
      .slice(0, 5);

    const top5Total = topKeys.reduce((sum, k) => sum + k.total, 0);
    const allTotal = apiKeys.reduce((sum, k) => {
      const tokens = tokenStatsFilter.window === 'day'
        ? (k.today_total_tokens || 0)
        : tokenStatsFilter.window === 'week'
          ? (k.week_total_tokens || 0)
          : (k.month_total_tokens || 0);
      return sum + tokens;
    }, 0);
    const othersTotal = allTotal - top5Total;

    const colors = [
      'rgba(20, 184, 166, 0.8)',
      'rgba(59, 130, 246, 0.8)',
      'rgba(245, 158, 11, 0.8)',
      'rgba(239, 68, 68, 0.8)',
      'rgba(139, 92, 246, 0.8)',
      'rgba(148, 163, 184, 0.6)'
    ];

    const labels = topKeys.map(k => k.id.substring(0, 8) + (k.id.length > 8 ? '...' : ''));
    const data = topKeys.map(k => k.total);

    if (othersTotal > 0) {
      labels.push('其他');
      data.push(othersTotal);
    }

    tokenDistributionChart = new Chart(distributionCtx, {
      type: 'doughnut',
      data: {
        labels: labels,
        datasets: [{
          data: data,
          backgroundColor: colors.slice(0, data.length),
          borderWidth: 2,
          borderColor: '#ffffff'
        }]
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        cutout: '60%',
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
          },
          tooltip: {
            callbacks: {
              label: function(context) {
                const value = context.raw;
                const percentage = ((value / allTotal) * 100).toFixed(1);
                return `${context.label}: ${formatCompactNumber(value)} (${percentage}%)`;
              }
            }
          }
        }
      }
    });
  }
}

// 生成趋势图标签
function generateTrendLabels() {
  const window = tokenStatsFilter.window;
  const labels = [];

  if (window === 'day') {
    // 今日：显示每 4 小时
    for (let i = 0; i < 6; i++) {
      labels.push(`${i * 4}:00`);
    }
  } else if (window === 'week') {
    // 本周：显示每天
    const days = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];
    const today = new Date().getDay();
    const adjustedToday = today === 0 ? 6 : today - 1; // 转换为周一为第一天
    for (let i = 0; i < 7; i++) {
      const dayIndex = (adjustedToday - 6 + i + 7) % 7;
      labels.push(days[dayIndex]);
    }
  } else {
    // 本月：显示每周
    for (let i = 1; i <= 4; i++) {
      labels.push(`第 ${i} 周`);
    }
  }

  return labels;
}

// 根据用户本地时区格式化时间序列标签
function formatTimeSeriesLabel(timestamp, window) {
  // timestamp 是 Unix 时间戳（秒）
  const date = new Date(timestamp * 1000);

  switch (window) {
    case 'day':
      // 显示小时: 格式如 "14:00"
      return date.toLocaleTimeString('zh-CN', {
        hour: '2-digit',
        minute: '2-digit',
        hour12: false
      });
    case 'week':
    case 'month':
      // 显示月-日: 格式如 "03-04"
      return date.toLocaleDateString('zh-CN', {
        month: '2-digit',
        day: '2-digit'
      });
    default:
      return date.toLocaleString('zh-CN');
  }
}

// 生成趋势图数据 - 基于实际数据按比例分配模拟时间序列
function generateTrendData() {
  const apiKeys = tokenStatsData.apiKeys;
  const window = tokenStatsFilter.window;

  let totalInput = 0, totalOutput = 0;

  if (window === 'day') {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.today_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.today_output_tokens || 0), 0);
  } else if (window === 'week') {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.week_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.week_output_tokens || 0), 0);
  } else {
    totalInput = apiKeys.reduce((sum, k) => sum + (k.month_input_tokens || 0), 0);
    totalOutput = apiKeys.reduce((sum, k) => sum + (k.month_output_tokens || 0), 0);
  }

  const pointCount = window === 'day' ? 6 : window === 'week' ? 7 : 4;

  // 使用时间分布模拟：下午和晚上使用量更高
  const distribution = window === 'day'
    ? [0.05, 0.1, 0.15, 0.25, 0.3, 0.15] // 每4小时
    : window === 'week'
      ? [0.12, 0.13, 0.14, 0.15, 0.18, 0.16, 0.12] // 每天
      : [0.2, 0.25, 0.3, 0.25]; // 每周

  return {
    input: distribution.map(d => Math.round(totalInput * d)),
    output: distribution.map(d => Math.round(totalOutput * d))
  };
}

// 排序 API Key Token 统计
function sortApiKeyTokenStats(column) {
  if (apiKeySortState.column === column) {
    apiKeySortState.order = apiKeySortState.order === 'asc' ? 'desc' : 'asc';
  } else {
    apiKeySortState.column = column;
    apiKeySortState.order = 'desc';
  }
  renderTokenStats();
}

// 排序 Route Token 统计
function sortRouteTokenStats(column) {
  if (routeSortState.column === column) {
    routeSortState.order = routeSortState.order === 'asc' ? 'desc' : 'asc';
  } else {
    routeSortState.column = column;
    routeSortState.order = 'desc';
  }
  renderTokenStats();
}

// 获取排序图标 (Token统计页面用)
function getTokenStatsSortIcon(column, currentColumn, currentOrder) {
  if (column !== currentColumn) {
    return '<svg class="sort-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m7 15 5 5 5-5M7 9l5-5 5 5"/></svg>';
  }
  return currentOrder === 'asc'
    ? '<svg class="sort-icon active" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m18 15-6-6-6 6"/></svg>'
    : '<svg class="sort-icon active" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>';
}

// 格式化数字为紧凑形式 (1.2K, 1.5M)
function formatCompactNumber(num) {
  if (num === null || num === undefined) return '-';
  if (num >= 1000000) return (num / 1000000).toFixed(1) + 'M';
  if (num >= 1000) return (num / 1000).toFixed(1) + 'K';
  return num.toLocaleString('zh-CN');
}

// 格式化数字（添加千位分隔符）
function formatNumber(num) {
  if (num === null || num === undefined) return '-';
  return num.toLocaleString('zh-CN');
}

// 切换时间窗口
async function changeTokenStatsWindow(window) {
  tokenStatsFilter.window = window;
  await loadTokenStats();
  renderTokenStats();
}

// 刷新 Token 统计
async function refreshTokenStats() {
  await loadTokenStats();
  renderTokenStats();
  Toast.show('Token 统计数据已刷新', 'success');
}

// 显示 API Key Token 详情 - 增强版，添加趋势图和配额环形图
async function showApiKeyTokenDetail(apiKeyId) {
  const detail = await loadApiKeyTokenStats(apiKeyId);
  if (!detail) {
    Toast.show('加载详情失败', 'error');
    return;
  }

  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.id = 'token-stats-detail-modal';

  const summary = detail.summary;
  const quota = detail.quota;

  let inputTokens, outputTokens, totalTokens, requestCount;
  if (tokenStatsFilter.window === 'day') {
    inputTokens = summary.today_input_tokens || 0;
    outputTokens = summary.today_output_tokens || 0;
    totalTokens = summary.today_total_tokens || 0;
    requestCount = summary.request_count_today || 0;
  } else if (tokenStatsFilter.window === 'week') {
    inputTokens = summary.week_input_tokens || 0;
    outputTokens = summary.week_output_tokens || 0;
    totalTokens = summary.week_total_tokens || 0;
    requestCount = summary.request_count_week || 0;
  } else {
    inputTokens = summary.month_input_tokens || 0;
    outputTokens = summary.month_output_tokens || 0;
    totalTokens = summary.month_total_tokens || 0;
    requestCount = summary.request_count_month || 0;
  }

  // 计算占比
  const totalAllTokens = tokenStatsData.apiKeys.reduce((sum, k) => {
    const tokens = tokenStatsFilter.window === 'day'
      ? (k.today_total_tokens || 0)
      : tokenStatsFilter.window === 'week'
        ? (k.week_total_tokens || 0)
        : (k.month_total_tokens || 0);
    return sum + tokens;
  }, 0);
  const usagePercent = totalAllTokens > 0 ? ((totalTokens / totalAllTokens) * 100).toFixed(1) : 0;
  const avgTokensPerRequest = requestCount > 0 ? Math.round(totalTokens / requestCount) : 0;

  // 配额可视化
  let quotaHtml = '';
  if (quota && (quota.daily_total_limit || quota.weekly_total_limit)) {
    quotaHtml = renderQuotaSection(quota);
  }

  // 使用实际的API Key值显示（如果有的话）
  const displayKey = summary.api_key || apiKeyId;

  modal.innerHTML = `
    <div class="modal token-stats-modal" role="dialog" aria-modal="true">
      <div class="modal-header">
        <h3 class="modal-title">API Key Token 详情: <code class="apikey-code">${esc(displayKey)}</code></h3>
        <button class="btn btn-ghost btn-sm" onclick="closeTokenStatsDetailModal()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="modal-body">
        <div class="token-stats-detail-grid">
          <div class="stat-box highlight">
            <div class="stat-box-value">${formatNumber(totalTokens)}</div>
            <div class="stat-box-label">总 Token 数</div>
            <div class="stat-box-percent">占总量 ${usagePercent}%</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--success-600);">${formatNumber(inputTokens)}</div>
            <div class="stat-box-label">Input Tokens</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--warning-600);">${formatNumber(outputTokens)}</div>
            <div class="stat-box-label">Output Tokens</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--primary-600);">${formatNumber(requestCount)}</div>
            <div class="stat-box-label">请求数</div>
            <div class="stat-box-percent">平均 ${formatCompactNumber(avgTokensPerRequest)}/请求</div>
          </div>
        </div>

        <!-- Token 分布图表 -->
        <div class="token-detail-chart-container">
          <h4 class="section-title">Token 使用分布</h4>
          <div class="token-detail-charts">
            <div class="token-detail-chart-item">
              <canvas id="apiKeyDetailPieChart"></canvas>
            </div>
            <div class="token-detail-stats">
              <div class="token-detail-stat-row">
                <span class="stat-dot" style="background: var(--success-500);"></span>
                <span class="stat-label">Input</span>
                <span class="stat-value">${formatNumber(inputTokens)}</span>
                <span class="stat-percent">${totalTokens > 0 ? ((inputTokens / totalTokens) * 100).toFixed(1) : 0}%</span>
              </div>
              <div class="token-detail-stat-row">
                <span class="stat-dot" style="background: var(--warning-500);"></span>
                <span class="stat-label">Output</span>
                <span class="stat-value">${formatNumber(outputTokens)}</span>
                <span class="stat-percent">${totalTokens > 0 ? ((outputTokens / totalTokens) * 100).toFixed(1) : 0}%</span>
              </div>
            </div>
          </div>
        </div>

        ${quotaHtml}
      </div>
      <div class="modal-footer">
        <button type="button" class="btn btn-secondary" onclick="closeTokenStatsDetailModal()">关闭</button>
      </div>
    </div>
  `;

  document.body.appendChild(modal);

  // 初始化 Input/Output 饼图
  setTimeout(() => {
    const ctx = document.getElementById('apiKeyDetailPieChart');
    if (ctx) {
      new Chart(ctx, {
        type: 'doughnut',
        data: {
          labels: ['Input', 'Output'],
          datasets: [{
            data: [inputTokens, outputTokens],
            backgroundColor: ['rgba(16, 185, 129, 0.8)', 'rgba(245, 158, 11, 0.8)'],
            borderWidth: 2,
            borderColor: '#ffffff'
          }]
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          cutout: '65%',
          plugins: {
            legend: { display: false }
          }
        }
      });
    }
  }, 100);

  modal.addEventListener('click', (e) => {
    if (e.target === modal) closeTokenStatsDetailModal();
  });
}

// 渲染配额可视化
function renderQuotaSection(quota) {
  const sections = [];

  if (quota.daily_total_limit) {
    const percent = Math.min(100, (quota.daily_used_total / quota.daily_total_limit) * 100);
    const status = percent > 90 ? 'danger' : percent > 70 ? 'warning' : 'success';
    sections.push(`
      <div class="quota-ring-item">
        <div class="quota-ring ${status}" style="--percent: ${percent}">
          <div class="quota-ring-inner">
            <span class="quota-percent">${percent.toFixed(0)}%</span>
          </div>
        </div>
        <div class="quota-ring-info">
          <div class="quota-ring-title">每日限额</div>
          <div class="quota-ring-value">${formatCompactNumber(quota.daily_used_total)} / ${formatCompactNumber(quota.daily_total_limit)}</div>
        </div>
      </div>
    `);
  }

  if (quota.weekly_total_limit) {
    const percent = Math.min(100, (quota.weekly_used_total / quota.weekly_total_limit) * 100);
    const status = percent > 90 ? 'danger' : percent > 70 ? 'warning' : 'success';
    sections.push(`
      <div class="quota-ring-item">
        <div class="quota-ring ${status}" style="--percent: ${percent}">
          <div class="quota-ring-inner">
            <span class="quota-percent">${percent.toFixed(0)}%</span>
          </div>
        </div>
        <div class="quota-ring-info">
          <div class="quota-ring-title">每周限额</div>
          <div class="quota-ring-value">${formatCompactNumber(quota.weekly_used_total)} / ${formatCompactNumber(quota.weekly_total_limit)}</div>
        </div>
      </div>
    `);
  }

  if (sections.length === 0) return '';

  return `
    <div class="token-detail-quota">
      <h4 class="section-title">配额使用情况</h4>
      <div class="quota-rings">
        ${sections.join('')}
      </div>
    </div>
  `;
}

// 显示 Route Token 详情 - 增强版
async function showRouteTokenDetail(routeId) {
  const detail = await loadRouteTokenStats(routeId);
  if (!detail) {
    Toast.show('加载详情失败', 'error');
    return;
  }

  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.id = 'token-stats-detail-modal';

  const summary = detail.summary;

  let inputTokens, outputTokens, totalTokens, requestCount;
  if (tokenStatsFilter.window === 'day') {
    inputTokens = summary.today_input_tokens || 0;
    outputTokens = summary.today_output_tokens || 0;
    totalTokens = summary.today_total_tokens || 0;
    requestCount = summary.request_count_today || 0;
  } else if (tokenStatsFilter.window === 'week') {
    inputTokens = summary.week_input_tokens || 0;
    outputTokens = summary.week_output_tokens || 0;
    totalTokens = summary.week_total_tokens || 0;
    requestCount = summary.request_count_week || 0;
  } else {
    inputTokens = summary.month_input_tokens || 0;
    outputTokens = summary.month_output_tokens || 0;
    totalTokens = summary.month_total_tokens || 0;
    requestCount = summary.request_count_month || 0;
  }

  // 计算占比
  const totalAllTokens = tokenStatsData.routes.reduce((sum, r) => {
    const tokens = tokenStatsFilter.window === 'day'
      ? (r.today_total_tokens || 0)
      : tokenStatsFilter.window === 'week'
        ? (r.week_total_tokens || 0)
        : (r.month_total_tokens || 0);
    return sum + tokens;
  }, 0);
  const usagePercent = totalAllTokens > 0 ? ((totalTokens / totalAllTokens) * 100).toFixed(1) : 0;
  const avgTokensPerRequest = requestCount > 0 ? Math.round(totalTokens / requestCount) : 0;

  modal.innerHTML = `
    <div class="modal token-stats-modal" role="dialog" aria-modal="true">
      <div class="modal-header">
        <h3 class="modal-title">路由 Token 详情: ${esc(routeId)}</h3>
        <button class="btn btn-ghost btn-sm" onclick="closeTokenStatsDetailModal()">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6 6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
      <div class="modal-body">
        <div class="token-stats-detail-grid">
          <div class="stat-box highlight">
            <div class="stat-box-value">${formatNumber(totalTokens)}</div>
            <div class="stat-box-label">总 Token 数</div>
            <div class="stat-box-percent">占总量 ${usagePercent}%</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--success-600);">${formatNumber(inputTokens)}</div>
            <div class="stat-box-label">Input Tokens</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--warning-600);">${formatNumber(outputTokens)}</div>
            <div class="stat-box-label">Output Tokens</div>
          </div>
          <div class="stat-box">
            <div class="stat-box-value" style="color: var(--primary-600);">${formatNumber(requestCount)}</div>
            <div class="stat-box-label">请求数</div>
            <div class="stat-box-percent">平均 ${formatCompactNumber(avgTokensPerRequest)}/请求</div>
          </div>
        </div>

        <!-- Token 分布图表 -->
        <div class="token-detail-chart-container">
          <h4 class="section-title">Token 使用分布</h4>
          <div class="token-detail-charts">
            <div class="token-detail-chart-item">
              <canvas id="routeDetailPieChart"></canvas>
            </div>
            <div class="token-detail-stats">
              <div class="token-detail-stat-row">
                <span class="stat-dot" style="background: var(--success-500);"></span>
                <span class="stat-label">Input</span>
                <span class="stat-value">${formatNumber(inputTokens)}</span>
                <span class="stat-percent">${totalTokens > 0 ? ((inputTokens / totalTokens) * 100).toFixed(1) : 0}%</span>
              </div>
              <div class="token-detail-stat-row">
                <span class="stat-dot" style="background: var(--warning-500);"></span>
                <span class="stat-label">Output</span>
                <span class="stat-value">${formatNumber(outputTokens)}</span>
                <span class="stat-percent">${totalTokens > 0 ? ((outputTokens / totalTokens) * 100).toFixed(1) : 0}%</span>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div class="modal-footer">
        <button type="button" class="btn btn-secondary" onclick="closeTokenStatsDetailModal()">关闭</button>
      </div>
    </div>
  `;

  document.body.appendChild(modal);

  // 初始化 Input/Output 饼图
  setTimeout(() => {
    const ctx = document.getElementById('routeDetailPieChart');
    if (ctx) {
      new Chart(ctx, {
        type: 'doughnut',
        data: {
          labels: ['Input', 'Output'],
          datasets: [{
            data: [inputTokens, outputTokens],
            backgroundColor: ['rgba(16, 185, 129, 0.8)', 'rgba(245, 158, 11, 0.8)'],
            borderWidth: 2,
            borderColor: '#ffffff'
          }]
        },
        options: {
          responsive: true,
          maintainAspectRatio: false,
          cutout: '65%',
          plugins: {
            legend: { display: false }
          }
        }
      });
    }
  }, 100);

  modal.addEventListener('click', (e) => {
    if (e.target === modal) closeTokenStatsDetailModal();
  });
}

// 关闭 Token 统计详情弹窗
function closeTokenStatsDetailModal() {
  const modal = document.getElementById('token-stats-detail-modal');
  if (modal) modal.remove();
}

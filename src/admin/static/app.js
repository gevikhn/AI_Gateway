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
  { id: 'apikeys', label: 'API Keys' },
  { id: 'banlogs', label: '封禁日志' },
  { id: 'metrics', label: '监控' },
  { id: 'auth', label: '认证' },
  { id: 'cors', label: 'CORS' },
  { id: 'ratelimit', label: '限流' },
  { id: 'concurrency', label: '并发控制' },
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
    // 处理 route_id（后端是 Option<String>，前端用数组便于处理）
    const routeId = keyConfig.route_id || null;
    const routeIds = routeId ? [routeId] : [];
    const routeName = routeId ? (cfg.routes?.find(r => r.id === routeId)?.name || routeId) : null;

    // 封禁状态处理
    const banStatus = keyConfig.ban_status || {};

    return {
      id: keyConfig.id,
      key: keyConfig.key,
      route_id: routeId,
      route_ids: routeIds,
      route_name: routeName,
      route_names: routeName ? [routeName] : [],
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
      created_at: keyConfig.created_at || new Date().toISOString(),
      updated_at: keyConfig.updated_at || new Date().toISOString()
    };
  });
}

// 将前端 API Key 数据转换为后端配置格式（架构设计 v2）
function convertApiKeyToConfig(apiKey) {
  // 如果 route_ids 有多个，取第一个（后端当前只支持单路由）
  const routeId = apiKey.route_ids?.length > 0 ? apiKey.route_ids[0] : null;

  return {
    id: apiKey.id,
    route_id: routeId,
    key: apiKey.key,
    enabled: apiKey.enabled,
    remark: apiKey.remark || '',
    rate_limit: apiKey.per_minute ? { per_minute: apiKey.per_minute } : null,
    concurrency: apiKey.max_inflight ? { max_inflight: apiKey.max_inflight } : null,
    ban_rules: apiKey.ban_rules || [],
    ban_status: apiKey.ban_status || {
      is_banned: false,
      ban_count: 0
    }
  };
}

// 模拟数据 - 用于测试 UI（匹配架构设计 v2）
function initMockData() {
  // 模拟 API Keys - 匹配后端 ApiKeyConfig 结构 v2
  apiKeysData = [
    {
      id: 'key_001',
      key: 'sk_prod_abc123def456ghi789',
      route_id: 'openai',
      route_ids: ['openai'],
      route_name: 'OpenAI API',
      route_names: ['OpenAI API'],
      enabled: true,
      remark: '生产环境主密钥',
      per_minute: 600,
      max_inflight: 10,
      ban_status: {
        is_banned: false,
        banned_at: null,
        banned_until: null,
        triggered_rule_id: null,
        reason: null,
        ban_count: 0
      },
      ban_rules: [
        {
          id: 'rule_001',
          name: '高错误率封禁',
          condition: {
            type: 'error_rate',
            window_secs: 300,
            threshold: 0.5,
            min_requests: 10
          },
          ban_duration_secs: 3600,
          enabled: true
        },
        {
          id: 'rule_002',
          name: '请求过多封禁',
          condition: {
            type: 'request_count',
            window_secs: 60,
            max_requests: 10000
          },
          ban_duration_secs: 1800,
          enabled: true
        }
      ],
      created_at: '2024-01-15T08:00:00Z',
      updated_at: '2024-01-20T10:30:00Z'
    },
    {
      id: 'key_002',
      key: 'sk_test_xyz789test123456',
      route_id: 'openai',
      route_ids: ['openai'],
      route_name: 'OpenAI API',
      route_names: ['OpenAI API'],
      enabled: true,
      remark: '测试环境密钥',
      per_minute: 60,
      max_inflight: 5,
      ban_status: {
        is_banned: true,
        banned_at: 1709452800,
        banned_until: 1709456400,
        triggered_rule_id: 'rule_003',
        reason: '错误率超过阈值',
        ban_count: 3
      },
      ban_rules: [
        {
          id: 'rule_003',
          name: '连续错误封禁',
          condition: {
            type: 'consecutive_errors',
            count: 5
          },
          ban_duration_secs: 300,
          enabled: true
        }
      ],
      created_at: '2024-01-16T09:00:00Z',
      updated_at: '2024-03-03T10:00:00Z'
    },
    {
      id: 'key_003',
      key: 'sk-anthropic-claude987654321',
      route_id: 'anthropic',
      route_ids: ['anthropic'],
      route_name: 'Anthropic Claude',
      route_names: ['Anthropic Claude'],
      enabled: false,
      remark: '已停用 - 迁移到新版',
      per_minute: 100,
      max_inflight: 8,
      ban_status: {
        is_banned: false,
        banned_at: null,
        banned_until: null,
        triggered_rule_id: null,
        reason: null,
        ban_count: 0
      },
      ban_rules: [],
      created_at: '2024-01-18T14:00:00Z',
      updated_at: '2024-02-01T16:00:00Z'
    },
    {
      id: 'key_004',
      key: 'sk-azure-openai-123456789abc',
      route_id: 'azure-openai',
      route_ids: ['azure-openai'],
      route_name: 'Azure OpenAI',
      route_names: ['Azure OpenAI'],
      enabled: true,
      remark: 'Azure 中国区',
      per_minute: 200,
      max_inflight: 20,
      ban_status: {
        is_banned: false,
        banned_at: null,
        banned_until: null,
        triggered_rule_id: null,
        reason: null,
        ban_count: 2
      },
      ban_rules: [
        {
          id: 'rule_004',
          name: '请求数封禁',
          condition: {
            type: 'request_count',
            window_secs: 60,
            max_requests: 200
          },
          ban_duration_secs: 600,
          enabled: true
        }
      ],
      created_at: '2024-02-01T10:00:00Z',
      updated_at: '2024-02-10T12:00:00Z'
    },
    {
      id: 'key_005',
      key: 'sk-gemini-pro-abcdef123456789',
      route_id: null,
      route_ids: [],
      route_name: null,
      route_names: [],
      enabled: true,
      remark: 'Gemini Pro 密钥 - 所有路由',
      per_minute: 80,
      max_inflight: 8,
      ban_status: {
        is_banned: false,
        banned_at: null,
        banned_until: null,
        triggered_rule_id: null,
        reason: null,
        ban_count: 1
      },
      ban_rules: [
        {
          id: 'rule_005',
          name: '高错误率封禁',
          condition: {
            type: 'error_rate',
            window_secs: 300,
            threshold: 0.5,
            min_requests: 10
          },
          ban_duration_secs: 3600,
          enabled: true
        }
      ],
      created_at: '2024-02-15T08:30:00Z',
      updated_at: '2024-02-20T09:00:00Z'
    }
  ];

  // 模拟封禁日志（架构设计 v2 - 使用 Unix 时间戳和 api_key_id）
  banLogsData = [
    {
      id: 'log_001',
      api_key_id: 'key_002',
      rule_id: 'rule_003',
      reason: '连续错误数超过阈值: 5次',
      banned_at: 1709452800,
      banned_until: 1709456400,
      unbanned_at: null,
      metrics_snapshot: { requests: 20, errors: 5, error_rate: 0.25 }
    },
    {
      id: 'log_002',
      api_key_id: 'key_001',
      rule_id: 'rule_001',
      reason: '错误率超过阈值: 55%',
      banned_at: 1709366400,
      banned_until: 1709370000,
      unbanned_at: 1709370000,
      metrics_snapshot: { requests: 100, errors: 55, error_rate: 0.55 }
    },
    {
      id: 'log_003',
      api_key_id: 'key_002',
      rule_id: 'rule_003',
      reason: '连续错误数超过阈值: 5次',
      banned_at: 1709280000,
      banned_until: 1709283600,
      unbanned_at: 1709283600,
      metrics_snapshot: { requests: 15, errors: 5, error_rate: 0.33 }
    },
    {
      id: 'log_004',
      api_key_id: 'key_004',
      rule_id: 'manual',
      reason: '手动封禁 - 密钥泄露',
      banned_at: 1709107200,
      banned_until: 1709193600,
      unbanned_at: 1709193600,
      metrics_snapshot: { requests: 0, errors: 0, error_rate: 0.0 }
    },
    {
      id: 'log_005',
      api_key_id: 'key_002',
      rule_id: 'rule_003',
      reason: '连续错误数超过阈值: 5次',
      banned_at: 1709020800,
      banned_until: 1709024400,
      unbanned_at: 1709024400,
      metrics_snapshot: { requests: 30, errors: 5, error_rate: 0.17 }
    },
    {
      id: 'log_006',
      api_key_id: 'key_002',
      rule_id: 'rule_003',
      reason: '连续错误数超过阈值: 5次',
      banned_at: 1708934400,
      banned_until: 1708938000,
      unbanned_at: 1708938000,
      metrics_snapshot: { requests: 25, errors: 5, error_rate: 0.20 }
    },
    {
      id: 'log_007',
      api_key_id: 'key_002',
      rule_id: 'rule_003',
      reason: '连续错误数超过阈值: 5次',
      reason: '超过限流阈值: 60秒内请求8次，阈值5',
      ban_duration_seconds: 300,
      created_at: '2024-02-25T11:10:00Z',
      operator: 'system'
    },
    {
      id: '8',
      api_key: 'sk-openai-test987654321xyz',
      route_id: 'openai',
      action_type: 'unban',
      reason: '自动解封',
      created_at: '2024-02-25T11:15:00Z',
      operator: 'system'
    },
    {
      id: '9',
      api_key: 'sk-anthropic-claude987654321',
      banned_at: 1709020800,
      banned_until: 1709107200,
      unbanned_at: 1709107200,
      metrics_snapshot: { requests: 0, errors: 0, error_rate: 0.0 }
    },
    {
      id: 'log_008',
      api_key_id: 'key_001',
      rule_id: 'rule_002',
      reason: '请求数超过阈值: 10000次/60秒',
      banned_at: 1708934400,
      banned_until: 1708936200,
      unbanned_at: 1708936200,
      metrics_snapshot: { requests: 10000, errors: 0, error_rate: 0.0 }
    },
    {
      id: 'log_009',
      api_key_id: 'key_001',
      rule_id: 'rule_001',
      reason: '错误率超过阈值: 52%',
      banned_at: 1708848000,
      banned_until: 1708851600,
      unbanned_at: 1708851600,
      metrics_snapshot: { requests: 50, errors: 26, error_rate: 0.52 }
    },
    {
      id: 'log_010',
      api_key_id: 'key_004',
      rule_id: 'rule_004',
      reason: '请求数超过阈值: 200次/60秒',
      banned_at: 1708761600,
      banned_until: 1708765200,
      unbanned_at: 1708765200,
      metrics_snapshot: { requests: 200, errors: 0, error_rate: 0.0 }
    },
    {
      id: 'log_011',
      api_key_id: 'key_005',
      rule_id: 'rule_005',
      reason: '错误率超过阈值: 60%',
      banned_at: 1708675200,
      banned_until: 1708678800,
      unbanned_at: 1708678800,
      metrics_snapshot: { requests: 30, errors: 18, error_rate: 0.60 }
    },
    {
      id: 'log_012',
      api_key_id: 'key_004',
      rule_id: 'rule_004',
      reason: '请求数超过阈值: 220次/60秒',
      banned_at: 1708588800,
      banned_until: 1708592400,
      unbanned_at: 1708592400,
      metrics_snapshot: { requests: 220, errors: 0, error_rate: 0.0 }
    }
  ];
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
  renderApiKeys();
  renderBanLogs();
  renderMetrics();
  renderAuth();
  renderCors();
  renderRateLimit();
  renderConcurrency();
  renderAdvanced();
}

// -- API Keys 管理页面 --
function renderApiKeys() {
  const panel = document.getElementById('tab-apikeys');
  if (!panel) return;

  // 从配置加载 API Keys（如果配置中有数据且前端数据为空）
  if (cfg?.api_keys?.keys && apiKeysData.length === 0) {
    apiKeysData = loadApiKeysFromConfig();
  }

  // 获取所有路由选项
  const routes = cfg?.routes || [];
  const routeOptions = routes.map(r => `<option value="${esc(r.id)}">${esc(r.id)}</option>`).join('');

  // 过滤 API Keys（适配新数据结构 v2）
  let filteredKeys = apiKeysData.filter(key => {
    // 按路由筛选（检查 route_id 或 route_ids）
    if (apiKeyFilter.route && key.route_id !== apiKeyFilter.route) return false;
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
    const isBanned = key.ban_status?.is_banned || false;
    const status = isBanned ? 'banned' : (key.enabled ? 'enabled' : 'disabled');
    const statusClass = `status-${status}`;
    const statusText = isBanned ? '封禁中' : (key.enabled ? '启用' : '禁用');
    const shortKey = key.key.substring(0, 20) + '...';

    // 路由标签（单路由，后端 route_id: Option<String>）
    const routeTags = key.route_id
      ? `<span class="route-tag" title="${esc(key.route_id)}">${esc(key.route_name || key.route_id)}</span>`
      : '<span class="route-tag all-routes">所有路由</span>';

    // 封禁状态显示
    let banStatusHtml = '-';
    if (isBanned && key.ban_status?.banned_until) {
      const expiresAt = key.ban_status.banned_until * 1000; // Unix秒转毫秒
      banStatusHtml = `<span class="ban-timer" data-expires="${expiresAt}">计算中...</span>`;
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
            <option value="">所有路由</option>
            ${routeOptions}
          </select>
          <select class="input select filter-select" onchange="setApiKeyFilter('status', this.value)">
            <option value="all">所有状态</option>
            <option value="enabled">启用</option>
            <option value="disabled">禁用</option>
            <option value="banned">封禁中</option>
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
  timers.forEach(timer => {
    const expiresAt = new Date(timer.dataset.expires);
    const now = new Date();
    const diff = expiresAt - now;

    if (diff <= 0) {
      timer.textContent = '即将解封';
      timer.classList.add('expiring');
    } else {
      const hours = Math.floor(diff / 3600000);
      const minutes = Math.floor((diff % 3600000) / 60000);
      const seconds = Math.floor((diff % 60000) / 1000);

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
function unbanApiKey(id) {
  confirmDelete(
    '解封 API Key',
    '确定要手动解封此 API Key 吗？',
    () => {
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

        // 同步到配置
        if (cfg.api_keys && cfg.api_keys.keys) {
          const configKey = cfg.api_keys.keys.find(k => k.id === id);
          if (configKey) {
            configKey.ban_status = { ...key.ban_status };
          }
        }

        // 添加到日志（新结构）
        banLogsData.unshift({
          id: 'log_' + Date.now(),
          api_key_id: key.id,
          rule_id: key.ban_status?.triggered_rule_id || 'manual',
          reason: '手动解封',
          banned_at: key.ban_status?.banned_at || now,
          banned_until: now,
          unbanned_at: now
        });

        Toast.show('API Key 已解封', 'success');
        renderApiKeys();
        renderBanLogs();
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

  // 路由单选（后端当前是 route_id: Option<String>）
  const routeOptions = routes.map(r => {
    const isSelected = key?.route_id === r.id;
    return `<option value="${esc(r.id)}" ${isSelected ? 'selected' : ''}>${esc(r.id)}</option>`;
  }).join('');

  // 生成封禁规则 HTML（新结构）
  const banRulesHtml = (key?.ban_rules || []).map((rule, idx) => {
    const cond = rule.condition || {};
    const condType = cond.type || 'error_rate';

    // 根据条件类型生成不同的输入字段
    let conditionFields = '';
    if (condType === 'error_rate') {
      conditionFields = `
        <input type="number" class="input" placeholder="窗口(秒)" value="${cond.window_secs || 300}" min="1" data-field="window_secs">
        <input type="number" class="input" placeholder="错误率(0-1)" value="${cond.threshold || 0.5}" min="0" max="1" step="0.1" data-field="threshold">
        <input type="number" class="input" placeholder="最小请求" value="${cond.min_requests || 10}" min="1" data-field="min_requests">
      `;
    } else if (condType === 'request_count') {
      conditionFields = `
        <input type="number" class="input" placeholder="窗口(秒)" value="${cond.window_secs || 60}" min="1" data-field="window_secs">
        <input type="number" class="input" placeholder="最大请求" value="${cond.max_requests || 1000}" min="1" data-field="max_requests">
        <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
      `;
    } else if (condType === 'consecutive_errors') {
      conditionFields = `
        <input type="number" class="input" placeholder="连续错误数" value="${cond.count || 5}" min="1" data-field="count">
        <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
        <input type="text" class="input" placeholder="-" disabled style="opacity:0.3">
      `;
    }

    return `
      <div class="ban-rule-item" data-idx="${idx}">
        <div class="ban-rule-header">
          <input type="text" class="input rule-name" placeholder="规则名称" value="${esc(rule.name || '')}">
          <select class="input select rule-type" onchange="updateBanRuleFields(this)">
            <option value="error_rate" ${condType === 'error_rate' ? 'selected' : ''}>错误率</option>
            <option value="request_count" ${condType === 'request_count' ? 'selected' : ''}>请求数</option>
            <option value="consecutive_errors" ${condType === 'consecutive_errors' ? 'selected' : ''}>连续错误</option>
          </select>
          <input type="number" class="input" placeholder="封禁(秒)" value="${rule.ban_duration_secs || 3600}" min="1" data-field="ban_duration">
          <label class="toggle rule-toggle">
            <input type="checkbox" ${rule.enabled !== false ? 'checked' : ''} data-field="enabled">
            <span class="toggle-slider"></span>
          </label>
          <button type="button" class="btn btn-danger btn-sm" onclick="this.closest('.ban-rule-item').remove(); checkBanRulesEmpty();">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M18 6 6 18M6 6l12 12"/>
            </svg>
          </button>
        </div>
        <div class="ban-rule-conditions">
          ${conditionFields}
        </div>
      </div>
    `;
  }).join('');

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
              <select class="input select" id="apikey-route">
                <option value="">所有路由</option>
                ${routeOptions}
              </select>
              <div class="field-help">不选择 = 可以访问所有路由</div>
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
            <h4 class="section-title">
              封禁规则配置
              <button type="button" class="btn btn-secondary btn-sm" onclick="addBanRuleV2()">+ 添加规则</button>
            </h4>
            <div id="ban-rules-list">
              ${banRulesHtml || '<div class="ban-rules-empty">暂无规则，点击"添加规则"创建</div>'}
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

  // 收集选中的路由（单选）
  const routeId = document.getElementById('apikey-route').value || null;

  // 收集封禁规则
  const banRules = [];
  document.querySelectorAll('.ban-rule-item').forEach(item => {
    const name = item.querySelector('.rule-name').value || '未命名规则';
    const type = item.querySelector('.rule-type').value;
    const banDuration = parseInt(item.querySelector('[data-field="ban_duration"]').value) || 3600;
    const enabled = item.querySelector('[data-field="enabled"]').checked;

    // 根据类型收集条件字段
    let condition = { type };
    if (type === 'error_rate') {
      condition.window_secs = parseInt(item.querySelector('[data-field="window_secs"]').value) || 300;
      condition.threshold = parseFloat(item.querySelector('[data-field="threshold"]').value) || 0.5;
      condition.min_requests = parseInt(item.querySelector('[data-field="min_requests"]').value) || 10;
    } else if (type === 'request_count') {
      condition.window_secs = parseInt(item.querySelector('[data-field="window_secs"]').value) || 60;
      condition.max_requests = parseInt(item.querySelector('[data-field="max_requests"]').value) || 1000;
    } else if (type === 'consecutive_errors') {
      condition.count = parseInt(item.querySelector('[data-field="count"]').value) || 5;
    }

    banRules.push({
      id: 'rule_' + Date.now() + '_' + Math.random().toString(36).substr(2, 9),
      name,
      condition,
      ban_duration_secs: banDuration,
      enabled
    });
  });

  // 查找路由名称
  const routeName = routeId ? (cfg?.routes?.find(r => r.id === routeId)?.name || routeId) : null;

  // 构建 API Key 数据对象
  const keyData = {
    id: id || 'key_' + Date.now(),
    key: value || generateApiKey(routeId || 'global'),
    route_id: routeId,
    route_ids: routeId ? [routeId] : [],
    route_name: routeName,
    route_names: routeName ? [routeName] : [],
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
    ban_rules: banRules,
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

  // 构建 API Key 数据对象
  const keyData = {
    id: id || Date.now().toString(),
    key: value || generateApiKey(routeId),
    route_id: routeId,
    route_name: routeName,
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

// -- Ban Logs 封禁日志页面（架构设计 v2）--
function renderBanLogs() {
  const panel = document.getElementById('tab-banlogs');
  if (!panel) return;

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
    const apiKeyDisplay = apiKey ? (apiKey.key.substring(0, 20) + '...') : log.api_key_id;

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
            <option value="all">所有操作</option>
            <option value="ban">封禁</option>
            <option value="unban">解封</option>
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

        <!-- API Keys 配置 (多租户隔离) -->
        <div class="field full-width">
          <div class="apikey-block">
            <div class="apikey-block-header">
              <label class="field-label">
                路由专属 API Keys
                <span class="field-hint" style="font-weight: normal; color: var(--text-secondary);">(可选，用于多租户隔离)</span>
              </label>
              <button class="btn btn-secondary btn-sm" onclick="addApiKey(${i})">+ 新增 API Key</button>
            </div>
            <div class="apikey-list" id="apikey-list-${i}">
              ${renderApiKeyList(i, r.api_keys)}
            </div>
            <div class="field-help">
              配置后，只有启用的 Key 能访问此路由。禁用后 Key 保留但无法使用。
              <strong>留空则回退到全局 gateway_auth.tokens</strong>
            </div>
          </div>
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

// 渲染 API Key 列表
function renderApiKeyList(routeIndex, apiKeys) {
  if (!apiKeys || apiKeys.length === 0) {
    return '<div class="apikey-empty">暂无 API Key，点击"新增 API Key"添加</div>';
  }

  return apiKeys.map((key, idx) => {
    // 支持旧格式字符串和新格式对象
    const keyValue = typeof key === 'string' ? key : key.key;
    const enabled = typeof key === 'string' ? true : (key.enabled !== false);

    return `
      <div class="apikey-item ${enabled ? '' : 'apikey-disabled'}" data-idx="${idx}">
        <label class="toggle apikey-toggle">
          <input type="checkbox" class="toggle-input" ${enabled ? 'checked' : ''} onchange="toggleApiKey(${routeIndex}, ${idx})" />
          <span class="toggle-slider" aria-hidden="true"></span>
        </label>
        <code class="apikey-value" title="${esc(keyValue)}">${esc(keyValue)}</code>
        <button class="btn btn-danger btn-sm" onclick="deleteApiKey(${routeIndex}, ${idx})" title="删除">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M3 6h18"/>
            <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/>
            <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>
          </svg>
        </button>
      </div>
    `;
  }).join('');
}

// 添加新的 API Key
function addApiKey(routeIndex) {
  const route = cfg.routes[routeIndex];
  if (!route.api_keys) {
    route.api_keys = [];
  }

  // 生成新的 API Key
  const newKey = generateApiKey(route.id || 'route');

  // 添加到列表（新格式：对象包含 key 和 enabled）
  route.api_keys.push({
    key: newKey,
    enabled: true
  });

  // 重新渲染
  const listEl = document.getElementById(`apikey-list-${routeIndex}`);
  if (listEl) {
    listEl.innerHTML = renderApiKeyList(routeIndex, route.api_keys);
  }

  Toast.show('API Key 已生成', 'success');
}

// 删除 API Key
function deleteApiKey(routeIndex, keyIndex) {
  const route = cfg.routes[routeIndex];
  if (!route.api_keys || keyIndex >= route.api_keys.length) return;

  route.api_keys.splice(keyIndex, 1);
  if (route.api_keys.length === 0) {
    route.api_keys = null;
  }

  // 重新渲染
  const listEl = document.getElementById(`apikey-list-${routeIndex}`);
  if (listEl) {
    listEl.innerHTML = renderApiKeyList(routeIndex, route.api_keys);
  }

  Toast.show('API Key 已删除', 'success');
}

// 切换 API Key 启用状态
function toggleApiKey(routeIndex, keyIndex) {
  const route = cfg.routes[routeIndex];
  if (!route.api_keys || keyIndex >= route.api_keys.length) return;

  const key = route.api_keys[keyIndex];
  if (typeof key === 'string') {
    // 旧格式，转换为新格式
    route.api_keys[keyIndex] = {
      key: key,
      enabled: false // 当前是禁用操作
    };
  } else {
    // 新格式，切换状态
    key.enabled = !key.enabled;
  }

  // 重新渲染
  const listEl = document.getElementById(`apikey-list-${routeIndex}`);
  if (listEl) {
    listEl.innerHTML = renderApiKeyList(routeIndex, route.api_keys);
  }
}

// 兼容旧格式：转换 api_keys 为后端需要的格式（只返回启用的 key 字符串数组）
function normalizeApiKeysForSave(apiKeys) {
  if (!apiKeys || apiKeys.length === 0) return null;

  return apiKeys
    .filter(k => {
      if (typeof k === 'string') return true; // 旧格式默认为启用
      return k.enabled !== false;
    })
    .map(k => typeof k === 'string' ? k : k.key);
}

function parseApiKeys(i, text) {
  const keys = text.split('\n').map(l => l.trim()).filter(l => l);
  cfg.routes[i].api_keys = keys.length > 0 ? keys : null;
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
    api_keys: null,
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
// 准备配置用于保存：转换 api_keys 格式
function prepareConfigForSave(config) {
  const prepared = JSON.parse(JSON.stringify(config));

  if (prepared.routes) {
    for (const route of prepared.routes) {
      if (route.api_keys) {
        // 只保存启用的 key，并转换为字符串数组
        route.api_keys = route.api_keys
          .filter(k => typeof k === 'string' || k.enabled !== false)
          .map(k => typeof k === 'string' ? k : k.key);

        if (route.api_keys.length === 0) {
          route.api_keys = null;
        }
      }
    }
  }

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

  // 初始化模拟数据
  initMockData();

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

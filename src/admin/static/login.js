// ===== 登录逻辑 =====
const TOKEN_KEY = 'ai_gateway_admin_token';
const THEME_KEY = 'ai_gateway_theme';

// ===== 主题管理 =====
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
}

// 应用主题
function applyTheme(theme) {
  document.documentElement.setAttribute('data-theme', theme);
}

// 更新主题切换按钮UI
function updateThemeToggleUI(theme) {
  document.querySelectorAll('.theme-toggle-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.themeValue === theme);
  });
}

// 监听系统主题变化
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  const currentTheme = localStorage.getItem(THEME_KEY) || 'light';
  if (currentTheme === 'auto') {
    applyTheme('auto');
  }
});

// 检查是否已登录
function checkAuth() {
  const token = localStorage.getItem(TOKEN_KEY);
  if (token) {
    // 验证 token 是否有效
    validateToken(token).then(valid => {
      if (valid) {
        // 跳转到管理界面
        window.location.href = window.CONFIG.adminPrefix + '/ui';
      } else {
        // Token 无效，清除并留在登录页
        localStorage.removeItem(TOKEN_KEY);
        showError('登录已过期，请重新输入 Token');
      }
    });
  }
}

// 验证 token
async function validateToken(token) {
  try {
    const res = await fetch(window.CONFIG.apiUrl, {
      headers: { Authorization: 'Bearer ' + token },
      cache: 'no-store'
    });
    return res.ok;
  } catch (e) {
    return false;
  }
}

// 处理登录
async function handleLogin(event) {
  event.preventDefault();

  const tokenInput = document.getElementById('token');
  const loginBtn = document.getElementById('loginBtn');
  const errorDiv = document.getElementById('loginError');

  const token = tokenInput.value.trim();
  if (!token) {
    showError('请输入 Admin Token');
    return;
  }

  // 显示加载状态
  loginBtn.disabled = true;
  loginBtn.classList.add('loading');
  errorDiv.style.display = 'none';

  // 验证 token
  const valid = await validateToken(token);

  if (valid) {
    // 保存 token
    localStorage.setItem(TOKEN_KEY, token);
    // 跳转到管理界面
    window.location.href = window.CONFIG.adminPrefix + '/ui';
  } else {
    // 显示错误
    showError('Token 无效，请检查您的凭据');
    loginBtn.disabled = false;
    loginBtn.classList.remove('loading');
    tokenInput.focus();
  }
}

// 显示错误信息
function showError(message) {
  const errorDiv = document.getElementById('loginError');
  errorDiv.textContent = message;
  errorDiv.style.display = 'block';
}

// 页面加载时检查
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => {
    initTheme();
    checkAuth();
  });
} else {
  initTheme();
  checkAuth();
}

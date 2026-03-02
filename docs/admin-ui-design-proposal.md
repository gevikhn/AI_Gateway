# AI Gateway Admin UI 设计优化方案

## 1. 当前UI分析

### 1.1 功能结构
当前Admin UI包含以下核心功能模块：
- **Routes**: 路由配置管理（增删改查）
- **Auth**: 认证令牌和来源配置
- **CORS**: 跨域资源共享配置
- **Rate Limit**: 限流配置
- **Concurrency**: 并发控制配置
- **Advanced**: 高级配置（只读显示）

### 1.2 当前设计优点
1. **简洁的布局**: 采用卡片式布局，信息层次清晰
2. **标签页导航**: 6个功能模块分组合理，切换方便
3. **即时反馈**: 操作后有状态提示（成功/错误）
4. **数据持久化**: 支持Token本地存储，减少重复输入
5. **响应式基础**: 使用flex/grid布局，有一定适配能力

### 1.3 当前设计缺点

#### 视觉设计
1. **配色单一**: 只有基础品牌色，缺乏层次感和视觉引导
2. **间距不一致**: 部分区域间距过小，视觉拥挤
3. **字体层次不足**: 标题、标签、内容区分度不够
4. **缺乏图标**: 纯文字界面，识别度较低

#### 用户体验
1. **无加载状态**: 网络请求时没有loading指示
2. **表单验证弱**: 缺少实时验证和视觉反馈
3. **删除无确认**: 删除操作没有二次确认，易误操作
4. **状态提示位置固定**: 容易被忽略
5. **无自动保存**: 长时间编辑可能丢失数据

#### 表单设计
1. **输入框样式单一**: 不同类型输入缺乏区分
2. **缺少帮助文本**: 复杂配置项没有说明
3. **布尔值使用select**: Strip Prefix等布尔值用下拉框不够直观
4. **文本域高度固定**: Inject Headers等字段编辑体验差

#### 响应式适配
1. **移动端体验差**: 小屏幕下布局混乱
2. **路由卡片网格**: 两列布局在小屏幕下挤压严重
3. **工具栏换行**: Token输入和按钮在小屏幕下堆叠

#### 可访问性
1. **对比度不足**: 部分文字颜色对比度低于WCAG 2.1 AA标准
2. **缺少焦点样式**: 键盘导航不可见
3. **无ARIA标签**: 屏幕阅读器支持不足
4. **表单关联**: label和input关联正确，但缺少描述文本

---

## 2. 设计规范系统

### 2.1 颜色系统

```css
:root {
  /* 主色调 */
  --primary-50: #f0fdfa;
  --primary-100: #ccfbf1;
  --primary-200: #99f6e4;
  --primary-300: #5eead4;
  --primary-400: #2dd4bf;
  --primary-500: #14b8a6;  /* 主品牌色 */
  --primary-600: #0d9488;
  --primary-700: #0f766e;  /* 原accent色 */
  --primary-800: #115e59;
  --primary-900: #134e4a;

  /* 功能色 */
  --success-50: #f0fdf4;
  --success-500: #22c55e;
  --success-600: #16a34a;
  --success-700: #15803d;

  --warning-50: #fffbeb;
  --warning-500: #f59e0b;
  --warning-600: #d97706;
  --warning-700: #b45309;

  --danger-50: #fef2f2;
  --danger-500: #ef4444;
  --danger-600: #dc2626;
  --danger-700: #b91c1c;

  --info-50: #eff6ff;
  --info-500: #3b82f6;
  --info-600: #2563eb;
  --info-700: #1d4ed8;

  /* 中性色 */
  --gray-50: #f8fafc;
  --gray-100: #f1f5f9;
  --gray-200: #e2e8f0;  /* 原line色 */
  --gray-300: #cbd5e1;
  --gray-400: #94a3b8;
  --gray-500: #64748b;  /* 原muted色 */
  --gray-600: #475569;
  --gray-700: #334155;
  --gray-800: #1e293b;
  --gray-900: #0f172a;  /* 原text色 */

  /* 背景色 */
  --bg-primary: #ffffff;
  --bg-secondary: #f8fafc;  /* 原bg色优化 */
  --bg-tertiary: #f1f5f9;
  --bg-gradient-start: #e0f2fe;
  --bg-gradient-end: #f0f9ff;

  /* 文字色 */
  --text-primary: #0f172a;
  --text-secondary: #475569;
  --text-tertiary: #64748b;
  --text-disabled: #94a3b8;
  --text-inverse: #ffffff;

  /* 边框色 */
  --border-light: #e2e8f0;
  --border-medium: #cbd5e1;
  --border-focus: #14b8a6;

  /* 阴影 */
  --shadow-sm: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  --shadow-md: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
  --shadow-lg: 0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1);
  --shadow-focus: 0 0 0 3px rgb(20 184 166 / 0.2);
}

/* 暗色模式 */
[data-theme="dark"] {
  --bg-primary: #0f172a;
  --bg-secondary: #1e293b;
  --bg-tertiary: #334155;
  --bg-gradient-start: #1e293b;
  --bg-gradient-end: #0f172a;

  --text-primary: #f8fafc;
  --text-secondary: #cbd5e1;
  --text-tertiary: #94a3b8;
  --text-disabled: #64748b;

  --border-light: #334155;
  --border-medium: #475569;
  --border-focus: #2dd4bf;

  --shadow-sm: 0 1px 2px 0 rgb(0 0 0 / 0.3);
  --shadow-md: 0 4px 6px -1px rgb(0 0 0 / 0.4), 0 2px 4px -2px rgb(0 0 0 / 0.4);
  --shadow-lg: 0 10px 15px -3px rgb(0 0 0 / 0.5), 0 4px 6px -4px rgb(0 0 0 / 0.5);
  --shadow-focus: 0 0 0 3px rgb(45 212 191 / 0.3);
}
```

### 2.2 字体规范

```css
:root {
  /* 字体族 */
  --font-sans: "Inter", "PingFang SC", "Microsoft YaHei", -apple-system, BlinkMacSystemFont, sans-serif;
  --font-mono: "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;

  /* 字号 */
  --text-xs: 0.75rem;    /* 12px */
  --text-sm: 0.875rem;   /* 14px */
  --text-base: 1rem;     /* 16px */
  --text-lg: 1.125rem;   /* 18px */
  --text-xl: 1.25rem;    /* 20px */
  --text-2xl: 1.5rem;    /* 24px */
  --text-3xl: 1.875rem;  /* 30px */

  /* 字重 */
  --font-normal: 400;
  --font-medium: 500;
  --font-semibold: 600;
  --font-bold: 700;

  /* 行高 */
  --leading-tight: 1.25;
  --leading-normal: 1.5;
  --leading-relaxed: 1.625;
}
```

### 2.3 间距系统

```css
:root {
  /* 间距 */
  --space-1: 0.25rem;   /* 4px */
  --space-2: 0.5rem;    /* 8px */
  --space-3: 0.75rem;   /* 12px */
  --space-4: 1rem;      /* 16px */
  --space-5: 1.25rem;   /* 20px */
  --space-6: 1.5rem;    /* 24px */
  --space-8: 2rem;      /* 32px */
  --space-10: 2.5rem;   /* 40px */
  --space-12: 3rem;     /* 48px */

  /* 圆角 */
  --radius-sm: 0.25rem;   /* 4px */
  --radius-md: 0.375rem;  /* 6px */
  --radius-lg: 0.5rem;    /* 8px */
  --radius-xl: 0.75rem;   /* 12px */
  --radius-2xl: 1rem;     /* 16px */
  --radius-full: 9999px;
}
```

---

## 3. 组件设计规范

### 3.1 按钮组件

```css
/* 基础按钮 */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  font-size: var(--text-sm);
  font-weight: var(--font-semibold);
  line-height: var(--leading-tight);
  border-radius: var(--radius-lg);
  border: 1px solid transparent;
  cursor: pointer;
  transition: all 0.15s ease;
  white-space: nowrap;
}

.btn:focus-visible {
  outline: none;
  box-shadow: var(--shadow-focus);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 主要按钮 */
.btn-primary {
  background: linear-gradient(135deg, var(--primary-600), var(--primary-700));
  color: var(--text-inverse);
  box-shadow: var(--shadow-sm);
}

.btn-primary:hover:not(:disabled) {
  background: linear-gradient(135deg, var(--primary-500), var(--primary-600));
  box-shadow: var(--shadow-md);
  transform: translateY(-1px);
}

.btn-primary:active:not(:disabled) {
  transform: translateY(0);
  box-shadow: var(--shadow-sm);
}

/* 次要按钮 */
.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border-color: var(--border-light);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--gray-200);
  color: var(--text-primary);
  border-color: var(--border-medium);
}

/* 危险按钮 */
.btn-danger {
  background: var(--danger-600);
  color: var(--text-inverse);
}

.btn-danger:hover:not(:disabled) {
  background: var(--danger-500);
}

/* 幽灵按钮 */
.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
}

.btn-ghost:hover:not(:disabled) {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

/* 按钮尺寸 */
.btn-sm {
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  border-radius: var(--radius-md);
}

.btn-lg {
  padding: var(--space-3) var(--space-6);
  font-size: var(--text-base);
}

/* 加载状态 */
.btn-loading {
  position: relative;
  color: transparent !important;
}

.btn-loading::after {
  content: "";
  position: absolute;
  width: 1rem;
  height: 1rem;
  border: 2px solid transparent;
  border-top-color: currentColor;
  border-radius: var(--radius-full);
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}
```

### 3.2 输入框组件

```css
/* 基础输入框 */
.input {
  width: 100%;
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-sm);
  line-height: var(--leading-normal);
  color: var(--text-primary);
  background: var(--bg-primary);
  border: 1px solid var(--border-medium);
  border-radius: var(--radius-lg);
  transition: all 0.15s ease;
}

.input::placeholder {
  color: var(--text-tertiary);
}

.input:hover {
  border-color: var(--gray-400);
}

.input:focus {
  outline: none;
  border-color: var(--primary-500);
  box-shadow: var(--shadow-focus);
}

.input:disabled,
.input[readonly] {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  cursor: not-allowed;
}

/* 错误状态 */
.input-error {
  border-color: var(--danger-500);
  background: var(--danger-50);
}

.input-error:focus {
  border-color: var(--danger-500);
  box-shadow: 0 0 0 3px rgb(239 68 68 / 0.2);
}

/* 成功状态 */
.input-success {
  border-color: var(--success-500);
  background: var(--success-50);
}

/* 文本域 */
.textarea {
  min-height: 100px;
  resize: vertical;
  font-family: var(--font-mono);
  line-height: var(--leading-relaxed);
}

/* 选择框 */
.select {
  appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%2364748b' stroke-width='2'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right var(--space-3) center;
  padding-right: var(--space-10);
}

/* 输入框组 */
.input-group {
  display: flex;
  align-items: stretch;
}

.input-group .input {
  border-radius: 0;
  border-right-width: 0;
}

.input-group .input:first-child {
  border-radius: var(--radius-lg) 0 0 var(--radius-lg);
}

.input-group .input:last-child {
  border-radius: 0 var(--radius-lg) var(--radius-lg) 0;
  border-right-width: 1px;
}

.input-group .btn {
  border-radius: 0 var(--radius-lg) var(--radius-lg) 0;
}
```

### 3.3 卡片组件

```css
.card {
  background: var(--bg-primary);
  border: 1px solid var(--border-light);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
  transition: box-shadow 0.15s ease;
}

.card:hover {
  box-shadow: var(--shadow-md);
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border-light);
}

.card-title {
  font-size: var(--text-base);
  font-weight: var(--font-semibold);
  color: var(--text-primary);
}

.card-body {
  padding: var(--space-5);
}

.card-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-5);
  border-top: 1px solid var(--border-light);
  background: var(--bg-secondary);
  border-radius: 0 0 var(--radius-xl) var(--radius-xl);
}
```

### 3.4 标签页组件

```css
.tabs {
  display: flex;
  gap: var(--space-1);
  border-bottom: 2px solid var(--border-light);
  padding: 0 var(--space-2);
}

.tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-sm);
  font-weight: var(--font-medium);
  color: var(--text-secondary);
  background: transparent;
  border: none;
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  cursor: pointer;
  transition: all 0.15s ease;
}

.tab:hover {
  color: var(--text-primary);
  background: var(--bg-tertiary);
}

.tab:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px var(--primary-500);
}

.tab.active {
  color: var(--primary-700);
  background: var(--bg-primary);
}

.tab.active::after {
  content: "";
  position: absolute;
  bottom: -2px;
  left: 0;
  right: 0;
  height: 2px;
  background: var(--primary-500);
}

/* 徽章 */
.badge {
  display: inline-flex;
  align-items: center;
  padding: var(--space-1) var(--space-2);
  font-size: var(--text-xs);
  font-weight: var(--font-semibold);
  border-radius: var(--radius-full);
}

.badge-warning {
  background: var(--warning-500);
  color: white;
}

.badge-info {
  background: var(--info-500);
  color: white;
}
```

### 3.5 表单字段组件

```css
.field {
  margin-bottom: var(--space-5);
}

.field-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
  font-size: var(--text-sm);
  font-weight: var(--font-semibold);
  color: var(--text-primary);
}

.field-label .required {
  color: var(--danger-500);
}

.field-help {
  margin-top: var(--space-1);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  line-height: var(--leading-normal);
}

.field-error {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  margin-top: var(--space-1);
  font-size: var(--text-xs);
  color: var(--danger-600);
}

/* 开关组件（替代select用于布尔值） */
.toggle {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  cursor: pointer;
}

.toggle-input {
  position: absolute;
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: relative;
  width: 44px;
  height: 24px;
  background: var(--gray-300);
  border-radius: var(--radius-full);
  transition: background 0.15s ease;
}

.toggle-slider::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 20px;
  height: 20px;
  background: white;
  border-radius: var(--radius-full);
  transition: transform 0.15s ease;
  box-shadow: var(--shadow-sm);
}

.toggle-input:checked + .toggle-slider {
  background: var(--primary-500);
}

.toggle-input:checked + .toggle-slider::after {
  transform: translateX(20px);
}

.toggle-input:focus-visible + .toggle-slider {
  box-shadow: var(--shadow-focus);
}

.toggle-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

/* 复选框 */
.checkbox {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
}

.checkbox-input {
  width: 18px;
  height: 18px;
  border: 2px solid var(--border-medium);
  border-radius: var(--radius-md);
  appearance: none;
  cursor: pointer;
  transition: all 0.15s ease;
}

.checkbox-input:checked {
  background: var(--primary-500);
  border-color: var(--primary-500);
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='white' stroke-width='3'%3E%3Cpath d='M20 6 9 17l-5-5'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: center;
}

.checkbox-input:focus-visible {
  outline: none;
  box-shadow: var(--shadow-focus);
}
```

### 3.6 状态提示组件

```css
/* Toast 通知 */
.toast-container {
  position: fixed;
  top: var(--space-4);
  right: var(--space-4);
  z-index: 1000;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  max-width: 400px;
}

.toast {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-4);
  background: var(--bg-primary);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  border-left: 4px solid;
  animation: slideIn 0.3s ease;
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateX(100%);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}

.toast-success {
  border-left-color: var(--success-500);
}

.toast-error {
  border-left-color: var(--danger-500);
}

.toast-warning {
  border-left-color: var(--warning-500);
}

.toast-info {
  border-left-color: var(--info-500);
}

.toast-icon {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
}

.toast-content {
  flex: 1;
}

.toast-title {
  font-weight: var(--font-semibold);
  color: var(--text-primary);
  margin-bottom: var(--space-1);
}

.toast-message {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.toast-close {
  flex-shrink: 0;
  padding: var(--space-1);
  background: none;
  border: none;
  color: var(--text-tertiary);
  cursor: pointer;
  border-radius: var(--radius-md);
  transition: all 0.15s ease;
}

.toast-close:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

/* 空状态 */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-12) var(--space-6);
  text-align: center;
}

.empty-state-icon {
  width: 64px;
  height: 64px;
  margin-bottom: var(--space-4);
  color: var(--text-tertiary);
}

.empty-state-title {
  font-size: var(--text-lg);
  font-weight: var(--font-semibold);
  color: var(--text-primary);
  margin-bottom: var(--space-2);
}

.empty-state-description {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  max-width: 300px;
}
```

### 3.7 模态框组件

```css
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  padding: var(--space-4);
  animation: fadeIn 0.2s ease;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.modal {
  background: var(--bg-primary);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  max-width: 480px;
  width: 100%;
  animation: scaleIn 0.2s ease;
}

@keyframes scaleIn {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-5);
  border-bottom: 1px solid var(--border-light);
}

.modal-title {
  font-size: var(--text-lg);
  font-weight: var(--font-semibold);
  color: var(--text-primary);
}

.modal-body {
  padding: var(--space-5);
}

.modal-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-5);
  border-top: 1px solid var(--border-light);
  background: var(--bg-secondary);
  border-radius: 0 0 var(--radius-xl) var(--radius-xl);
}
```

---

## 4. 布局优化

### 4.1 整体布局

```css
/* 页面容器 */
.page {
  min-height: 100vh;
  background: linear-gradient(135deg, var(--bg-gradient-start) 0%, var(--bg-gradient-end) 100%);
}

.container {
  max-width: 1200px;
  margin: 0 auto;
  padding: var(--space-6);
}

/* 头部 */
.header {
  margin-bottom: var(--space-6);
}

.header-title {
  font-size: var(--text-2xl);
  font-weight: var(--font-bold);
  color: var(--text-primary);
  margin-bottom: var(--space-1);
}

.header-subtitle {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

/* 工具栏 */
.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-6);
  padding: var(--space-4);
  background: var(--bg-primary);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
}

.toolbar-input {
  flex: 1;
  min-width: 0;
}

/* 主内容区 */
.main-content {
  background: var(--bg-primary);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
  overflow: hidden;
}
```

### 4.2 路由卡片布局

```css
.route-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.route-card {
  background: var(--bg-secondary);
  border: 1px solid var(--border-light);
  border-radius: var(--radius-xl);
  overflow: hidden;
  transition: all 0.15s ease;
}

.route-card:hover {
  border-color: var(--border-medium);
  box-shadow: var(--shadow-md);
}

.route-card.dragging {
  opacity: 0.5;
}

.route-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  background: var(--bg-primary);
  border-bottom: 1px solid var(--border-light);
}

.route-title {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.route-number {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  background: var(--primary-100);
  color: var(--primary-700);
  font-size: var(--text-sm);
  font-weight: var(--font-semibold);
  border-radius: var(--radius-lg);
}

.route-id {
  font-weight: var(--font-semibold);
  color: var(--text-primary);
}

.route-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.route-body {
  padding: var(--space-5);
}

.route-fields {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-4);
}

.route-fields .full-width {
  grid-column: 1 / -1;
}

@media (max-width: 640px) {
  .route-fields {
    grid-template-columns: 1fr;
  }
}
```

### 4.3 响应式适配

```css
/* 断点定义 */
/* sm: 640px */
/* md: 768px */
/* lg: 1024px */
/* xl: 1280px */

/* 移动端优化 */
@media (max-width: 768px) {
  .container {
    padding: var(--space-4);
  }

  .header-title {
    font-size: var(--text-xl);
  }

  .toolbar {
    flex-direction: column;
    align-items: stretch;
  }

  .toolbar-input {
    width: 100%;
  }

  .tabs {
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: none;
  }

  .tabs::-webkit-scrollbar {
    display: none;
  }

  .tab {
    white-space: nowrap;
    flex-shrink: 0;
  }

  .route-header {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .route-actions {
    width: 100%;
    justify-content: flex-end;
  }

  .toast-container {
    left: var(--space-4);
    right: var(--space-4);
    max-width: none;
  }
}

/* 暗色模式切换按钮 */
.theme-toggle {
  position: fixed;
  bottom: var(--space-4);
  right: var(--space-4);
  width: 48px;
  height: 48px;
  border-radius: var(--radius-full);
  background: var(--bg-primary);
  border: 1px solid var(--border-light);
  box-shadow: var(--shadow-lg);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s ease;
  z-index: 100;
}

.theme-toggle:hover {
  transform: scale(1.1);
  box-shadow: var(--shadow-lg), 0 0 20px rgba(20, 184, 166, 0.3);
}

.theme-toggle svg {
  width: 24px;
  height: 24px;
  color: var(--text-secondary);
}
```

---

## 5. 交互优化

### 5.1 加载状态

```javascript
// 加载状态管理
class LoadingState {
  constructor(button) {
    this.button = button;
    this.originalText = button.textContent;
  }

  start() {
    this.button.disabled = true;
    this.button.classList.add('btn-loading');
    this.button.dataset.originalText = this.originalText;
  }

  stop() {
    this.button.disabled = false;
    this.button.classList.remove('btn-loading');
  }
}

// 骨架屏
.skeleton {
  background: linear-gradient(90deg, var(--bg-tertiary) 25%, var(--bg-secondary) 50%, var(--bg-tertiary) 75%);
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
  border-radius: var(--radius-md);
}

@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
```

### 5.2 表单验证

```javascript
// 实时验证
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
      case 'pattern':
        return !value || new RegExp(param).test(value) ? null : '格式不正确';
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
```

### 5.3 确认对话框

```javascript
// 删除确认
function confirmDelete(title, message, onConfirm) {
  const modal = document.createElement('div');
  modal.className = 'modal-overlay';
  modal.innerHTML = `
    <div class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
      <div class="modal-header">
        <h3 id="modal-title" class="modal-title">${title}</h3>
        <button class="btn-ghost btn-sm" onclick="this.closest('.modal-overlay').remove()">
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

  // ESC关闭
  modal.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      modal.remove();
    }
  });

  // 点击遮罩关闭
  modal.addEventListener('click', (e) => {
    if (e.target === modal) {
      modal.remove();
    }
  });

  // 聚焦第一个按钮
  modal.querySelector('button').focus();
}

// 使用示例
function removeRoute(index) {
  confirmDelete(
    '删除路由',
    '确定要删除此路由吗？此操作不可撤销。',
    () => {
      cfg.routes.splice(index, 1);
      renderRoutes();
      showToast('路由已删除', 'success');
    }
  );
}
```

### 5.4 Toast通知系统

```javascript
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

    // 自动移除
    if (duration > 0) {
      setTimeout(() => {
        toast.style.animation = 'slideIn 0.3s ease reverse';
        setTimeout(() => toast.remove(), 300);
      }, duration);
    }

    return toast;
  }
}

// 使用示例
Toast.show('配置已保存', 'success');
Toast.show('加载失败，请重试', 'error');
```

### 5.5 自动保存

```javascript
class AutoSave {
  constructor(config, options = {}) {
    this.config = config;
    this.interval = options.interval || 30000; // 30秒
    this.key = options.key || 'ai_gw_admin_draft';
    this.timer = null;
    this.lastSaved = null;
  }

  start() {
    this.timer = setInterval(() => this.save(), this.interval);
    window.addEventListener('beforeunload', () => this.save());
  }

  stop() {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }

  save() {
    const data = {
      config: this.config,
      timestamp: Date.now()
    };
    localStorage.setItem(this.key, JSON.stringify(data));
    this.lastSaved = new Date();
  }

  load() {
    const data = localStorage.getItem(this.key);
    if (data) {
      try {
        const parsed = JSON.parse(data);
        return parsed.config;
      } catch (e) {
        return null;
      }
    }
    return null;
  }

  clear() {
    localStorage.removeItem(this.key);
  }

  hasDraft() {
    return !!localStorage.getItem(this.key);
  }
}

// 使用示例
const autoSave = new AutoSave(cfg, { interval: 30000 });
autoSave.start();
```

---

## 6. 暗色模式实现

```javascript
// 主题管理
class ThemeManager {
  static STORAGE_KEY = 'ai_gw_theme';

  static init() {
    const savedTheme = localStorage.getItem(this.STORAGE_KEY);
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;

    if (savedTheme) {
      this.setTheme(savedTheme);
    } else if (prefersDark) {
      this.setTheme('dark');
    }

    // 监听系统主题变化
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
      if (!localStorage.getItem(this.STORAGE_KEY)) {
        this.setTheme(e.matches ? 'dark' : 'light');
      }
    });
  }

  static setTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem(this.STORAGE_KEY, theme);
  }

  static toggle() {
    const current = document.documentElement.getAttribute('data-theme');
    const next = current === 'dark' ? 'light' : 'dark';
    this.setTheme(next);
  }

  static getCurrent() {
    return document.documentElement.getAttribute('data-theme') || 'light';
  }
}

// 初始化
document.addEventListener('DOMContentLoaded', () => {
  ThemeManager.init();

  // 绑定切换按钮
  const toggleBtn = document.getElementById('theme-toggle');
  if (toggleBtn) {
    toggleBtn.addEventListener('click', () => ThemeManager.toggle());
  }
});
```

---

## 7. 可访问性改进

```css
/* 焦点样式 */
:focus-visible {
  outline: 2px solid var(--primary-500);
  outline-offset: 2px;
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}

/* 高对比度模式支持 */
@media (prefers-contrast: high) {
  :root {
    --border-medium: #000;
    --text-primary: #000;
  }

  .btn-primary {
    border: 2px solid #000;
  }
}
```

```javascript
// ARIA增强
function enhanceAccessibility() {
  // 为标签页添加ARIA属性
  const tabs = document.querySelectorAll('.tab');
  const panels = document.querySelectorAll('.tab-panel');

  tabs.forEach((tab, index) => {
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-selected', tab.classList.contains('active'));
    tab.setAttribute('aria-controls', panels[index]?.id);
    tab.setAttribute('tabindex', tab.classList.contains('active') ? '0' : '-1');
  });

  panels.forEach((panel, index) => {
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-labelledby', tabs[index]?.id);
  });

  // 键盘导航
  document.querySelector('.tabs')?.addEventListener('keydown', (e) => {
    const activeTab = document.querySelector('.tab.active');
    const tabsArray = Array.from(tabs);
    const currentIndex = tabsArray.indexOf(activeTab);

    let newIndex;
    switch (e.key) {
      case 'ArrowLeft':
        newIndex = currentIndex > 0 ? currentIndex - 1 : tabsArray.length - 1;
        break;
      case 'ArrowRight':
        newIndex = currentIndex < tabsArray.length - 1 ? currentIndex + 1 : 0;
        break;
      case 'Home':
        newIndex = 0;
        break;
      case 'End':
        newIndex = tabsArray.length - 1;
        break;
      default:
        return;
    }

    e.preventDefault();
    tabsArray[newIndex].click();
    tabsArray[newIndex].focus();
  });
}
```

---

## 8. 实施建议

### 8.1 优先级排序

**P0 - 核心体验**
1. 统一间距和颜色系统
2. 添加加载状态
3. 删除操作确认
4. 表单验证反馈

**P1 - 体验提升**
1. Toast通知系统
2. 响应式布局优化
3. 开关组件替换布尔值下拉框
4. 输入框焦点样式

**P2 - 高级功能**
1. 暗色模式
2. 自动保存
3. 键盘导航
4. 可访问性增强

### 8.2 代码组织建议

建议将CSS按组件拆分为独立模块：

```
admin-ui/
├── base.css          # 变量、重置样式
├── components.css    # 按钮、输入框、卡片等组件
├── layout.css        # 布局相关
├── utilities.css     # 工具类
└── dark-mode.css     # 暗色模式覆盖
```

### 8.3 性能优化

1. **CSS优化**: 使用CSS变量减少重复代码
2. **动画优化**: 使用transform和opacity实现动画，启用GPU加速
3. **事件委托**: 动态元素使用事件委托减少监听器数量
4. **防抖节流**: 输入验证使用防抖，滚动事件使用节流

---

## 9. 完整示例代码

以下是一个完整的优化后的路由卡片HTML结构示例：

```html
<div class="route-card" data-idx="0">
  <div class="route-header">
    <div class="route-title">
      <span class="route-number">1</span>
      <span class="route-id">openai-route</span>
    </div>
    <div class="route-actions">
      <button class="btn btn-ghost btn-sm" title="展开/收起">
        <svg><!-- 展开图标 --></svg>
      </button>
      <button class="btn btn-danger btn-sm" onclick="removeRoute(0)">
        <svg><!-- 删除图标 --></svg>
        删除
      </button>
    </div>
  </div>
  <div class="route-body">
    <div class="route-fields">
      <div class="field">
        <label class="field-label">
          ID
          <span class="required" aria-label="必填">*</span>
        </label>
        <input class="input" value="openai-route" data-validate="required" required />
        <div class="field-help">路由唯一标识，用于日志和监控</div>
      </div>

      <div class="field">
        <label class="field-label">Prefix</label>
        <input class="input" value="/v1/chat" placeholder="/api" />
      </div>

      <div class="field full-width">
        <label class="field-label">Base URL</label>
        <input class="input input-success" value="https://api.openai.com" data-validate="required,url" />
      </div>

      <div class="field">
        <label class="field-label">Strip Prefix</label>
        <label class="toggle">
          <input type="checkbox" class="toggle-input" checked />
          <span class="toggle-slider" aria-hidden="true"></span>
          <span class="toggle-label">启用</span>
        </label>
      </div>

      <div class="field full-width">
        <label class="field-label">Inject Headers</label>
        <textarea class="input textarea" rows="4" placeholder="每行一个，格式: name: value"></textarea>
        <div class="field-help">向上游服务注入的自定义请求头</div>
      </div>
    </div>
  </div>
</div>
```

---

## 10. 总结

本设计方案针对AI Gateway Admin UI进行了全面的UI/UX优化，主要改进点包括：

1. **视觉设计**: 建立了完整的颜色系统、字体规范和间距系统，提升界面美观度和一致性
2. **组件设计**: 设计了按钮、输入框、卡片、标签页等核心组件，提供丰富的变体和状态
3. **交互优化**: 增加了加载状态、表单验证、确认对话框、Toast通知等交互反馈机制
4. **响应式适配**: 针对移动端优化了布局和交互，确保在各种设备上都有良好的体验
5. **暗色模式**: 提供了完整的暗色模式支持，满足用户在不同环境下的使用需求
6. **可访问性**: 增强了键盘导航、焦点样式、ARIA标签等，提升无障碍体验

建议按优先级逐步实施这些改进，优先解决核心体验问题，再逐步添加高级功能。

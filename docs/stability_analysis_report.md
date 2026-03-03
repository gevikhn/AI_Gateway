# AI Gateway 路由稳定性分析报告

**分析日期**: 2026-03-03
**分析范围**: src/server.rs, src/proxy.rs, src/concurrency.rs, src/ratelimit.rs, src/lib.rs
**分析重点**: 错误处理、资源泄漏、超时机制、限流熔断、并发控制、优雅关闭

---

## 1. 当前稳定性机制概览

### 1.1 已实现的稳定性机制

| 机制 | 实现状态 | 位置 | 说明 |
|------|---------|------|------|
| 并发控制 | ✅ 已实现 | concurrency.rs | 基于tokio::Semaphore的上下游并发限制 |
| 限流 | ✅ 已实现 | ratelimit.rs | 固定窗口算法，支持多级配置 |
| 请求超时 | ✅ 已实现 | server.rs:1089 | 使用tokio::time::timeout_at |
| 连接超时 | ✅ 已实现 | server.rs:1515 | reqwest connect_timeout配置 |
| 响应流超时 | ⚠️ 部分实现 | server.rs:1558 | 非SSE响应有超时，SSE无超时 |
| 错误分类 | ✅ 已实现 | server.rs:1494 | UpstreamError区分超时和请求错误 |
| 热重载 | ✅ 已实现 | server.rs:31 | 使用ArcSwap实现原子配置切换 |
| 优雅关闭 | ❌ 缺失 | - | 无SIGTERM处理机制 |

### 1.2 整体架构评估

```
┌─────────────────────────────────────────────────────────────┐
│                        请求处理流程                          │
├─────────────────────────────────────────────────────────────┤
│  1. 路由匹配 (proxy.rs)                                      │
│     └── 前缀匹配，边界检查                                    │
│                                                              │
│  2. 认证 (api_keys.rs)                                       │
│     └── API Key验证，路由权限检查                            │
│                                                              │
│  3. 限流 (ratelimit.rs)                                      │
│     └── 固定窗口计数器，多级配置继承                          │
│                                                              │
│  4. 并发控制 (concurrency.rs)                                │
│     └── Semaphore获取，支持API Key级别限制                   │
│                                                              │
│  5. 上游请求 (server.rs)                                     │
│     ├── 连接池复用 (reqwest::Client)                         │
│     ├── 连接超时控制                                         │
│     ├── 请求超时控制                                         │
│     └── 响应流处理 (含/不含deadline)                         │
│                                                              │
│  6. 响应处理 (server.rs)                                     │
│     ├── 响应头部清理                                         │
│     ├── CORS处理                                             │
│     └── Metrics记录 (RAII模式)                               │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. 关键风险点详细分析

### 2.1 🔴 高风险：并发配置解析Bug

**位置**: `src/concurrency.rs:91`

**问题代码**:
```rust
let downstream_limit = api_key_config
    .and_then(|c| c.downstream_max_inflight)
    .or_else(|| self.downstream_semaphore.as_ref().map(|s| s.available_permits()));
    //                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //                                         返回当前可用许可数，不是总容量！
```

**风险描述**:
当获取全局下游并发限制时，代码调用 `available_permits()` 返回的是**当前可用的许可数**，而不是信号量的总容量。这会导致：
- 高并发时返回错误的限制值（接近0）
- 配置解析结果随时间变化，不可预测
- 可能导致限流判断错误

**修复建议**:
```rust
// 方案1：存储总容量而不是依赖available_permits
pub struct ConcurrencyController {
    downstream_semaphore: Option<Arc<Semaphore>>,
    downstream_total_limit: Option<usize>, // 添加这个字段
    // ...
}

impl ConcurrencyController {
    pub fn new(config: &AppConfig) -> Option<Self> {
        let downstream_limit = config
            .concurrency
            .as_ref()
            .and_then(|c| c.downstream_max_inflight);

        Some(Self {
            downstream_semaphore: downstream_limit.map(|limit| Arc::new(Semaphore::new(limit))),
            downstream_total_limit: downstream_limit, // 保存总容量
            // ...
        })
    }

    pub fn resolve_config(&self, api_key: Option<&str>, route: &RouteConfig) -> ResolvedConcurrencyConfig {
        let api_key_config = api_key.and_then(|key| self.api_key_configs.get(key));

        let downstream_limit = api_key_config
            .and_then(|c| c.downstream_max_inflight)
            .or(self.downstream_total_limit); // 使用保存的总容量

        // ...
    }
}
```

---

### 2.2 🔴 高风险：限流器内存泄漏

**位置**: `src/ratelimit.rs:49-53`

**问题代码**:
```rust
let mut limiters = self.key_limiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

let limiter = limiters
    .entry(api_key.to_string())
    .or_insert_with(|| RateLimiter::new(config.per_minute));
```

**风险描述**:
- `key_limiters` HashMap存储每个API key的限流器
- 没有清理机制，如果API key数量无限增长（如使用动态生成的key），会导致内存泄漏
- 在长时间运行的生产环境中，可能耗尽内存

**修复建议**:
```rust
use std::time::Instant;
use std::collections::HashMap;

// 添加最后访问时间
struct RateLimiterEntry {
    limiter: RateLimiter,
    last_accessed: Instant,
}

pub struct RateLimiterManager {
    global_config: Option<RateLimitConfig>,
    key_limiters: Mutex<HashMap<String, RateLimiterEntry>>,
    last_cleanup: Mutex<Instant>,
}

impl RateLimiterManager {
    pub fn check(&self, api_key: &str, api_key_config: Option<&RateLimitConfig>, route_config: Option<&RateLimitConfig>) -> RateLimitDecision {
        // 定期清理（每10分钟）
        self.maybe_cleanup();

        let mut limiters = self.key_limiters.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();

        let entry = limiters.entry(api_key.to_string())
            .and_modify(|e| e.last_accessed = now)
            .or_insert_with(|| RateLimiterEntry {
                limiter: RateLimiter::new(config.per_minute),
                last_accessed: now,
            });

        entry.limiter.check_internal(current_epoch_seconds())
    }

    fn maybe_cleanup(&self) {
        let mut last_cleanup = self.last_cleanup.lock().unwrap();
        let now = Instant::now();

        // 每10分钟清理一次
        if now.duration_since(*last_cleanup).as_secs() > 600 {
            if let Ok(mut limiters) = self.key_limiters.try_lock() {
                // 清理超过1小时未访问的限流器
                limiters.retain(|_, entry| {
                    now.duration_since(entry.last_accessed).as_secs() < 3600
                });
                *last_cleanup = now;
            }
        }
    }
}
```

---

### 2.3 🔴 高风险：并发信号量内存泄漏

**位置**: `src/concurrency.rs:133-138` 和 `206-211`

**问题描述**:
与限流器类似，`upstream_semaphores` HashMap存储每个上游key的信号量，没有清理机制。

**修复建议**:
```rust
use std::time::Instant;

struct SemaphoreEntry {
    semaphore: Arc<Semaphore>,
    last_accessed: Instant,
}

pub struct ConcurrencyController {
    downstream_semaphore: Option<Arc<Semaphore>>,
    downstream_total_limit: Option<usize>,
    upstream_default_limit: Option<usize>,
    upstream_semaphores: Mutex<HashMap<String, SemaphoreEntry>>,
    api_key_configs: HashMap<String, ApiKeyConcurrencyConfig>,
    last_cleanup: Mutex<Instant>,
}

// 在获取信号量时更新访问时间
pub async fn acquire_upstream(&self, route: &RouteConfig) -> Result<Option<OwnedSemaphorePermit>, ConcurrencyError> {
    // ... 前面的代码 ...

    let semaphore = {
        let mut semaphores = self.upstream_semaphores.lock().await;
        let now = Instant::now();

        // 定期清理（简单实现）
        if self.should_cleanup(&now) {
            semaphores.retain(|_, entry| {
                now.duration_since(entry.last_accessed).as_secs() < 3600
            });
        }

        semaphores
            .entry(semaphore_key.clone())
            .and_modify(|e| e.last_accessed = now)
            .or_insert_with(|| SemaphoreEntry {
                semaphore: Arc::new(Semaphore::new(limit)),
                last_accessed: now,
            })
            .semaphore
            .clone()
    };

    // ...
}
```

---

### 2.4 🔴 高风险：缺少优雅关闭机制

**位置**: `src/server.rs:116-155`

**问题描述**:
服务器启动代码没有处理SIGTERM/SIGINT信号，也没有优雅关闭机制：
```rust
// 当前代码
axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
    .await
    .map_err(|err| format!("server error: {err}"))
```

**风险**:
- 容器编排环境（Kubernetes）发送SIGTERM后，服务器立即终止，导致inflight请求中断
- 没有给正在处理的请求完成的时间
- 可能导致客户端收到连接重置错误

**修复建议**:
```rust
use tokio::signal;
use std::time::Duration;

pub async fn run_server(config: Arc<AppConfig>, config_path: Option<String>) -> Result<(), String> {
    let listen_addr: SocketAddr = config
        .listen
        .parse()
        .map_err(|err| format!("invalid listen address `{}`: {err}", config.listen))?;

    let app = build_app(config.clone(), config_path.map(PathBuf::from)).await?;

    // 创建shutdown信号监听
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // 在后台任务中监听信号
    tokio::spawn(async move {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to create SIGTERM handler");
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .expect("Failed to create SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => info!("Received SIGTERM, starting graceful shutdown..."),
            _ = sigint.recv() => info!("Received SIGINT, starting graceful shutdown..."),
        }

        let _ = shutdown_tx.send(true);
    });

    if let Some(tls_config) = &config.inbound_tls {
        // ... TLS配置 ...

        let handle = axum_server::bind_rustls(listen_addr, rustls_config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>());

        // 使用graceful shutdown
        handle
            .with_graceful_shutdown(shutdown_signal(shutdown_rx))
            .await
            .map_err(|err| format!("server error: {err}"))
    } else {
        let listener = tokio::net::TcpListener::bind(listen_addr)
            .await
            .map_err(|err| format!("failed to bind `{listen_addr}`: {err}"))?;

        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal(shutdown_rx))
            .await
            .map_err(|err| format!("server error: {err}"))
    }
}

async fn shutdown_signal(mut rx: tokio::sync::watch::Receiver<bool>) {
    // 等待shutdown信号
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            break;
        }
    }

    // 给inflight请求30秒完成
    info!("Waiting 30 seconds for inflight requests to complete...");
    tokio::time::sleep(Duration::from_secs(30)).await;
}
```

---

### 2.5 🟡 中风险：SSE连接无超时保护

**位置**: `src/server.rs:1251-1255`

**问题代码**:
```rust
let stream = if is_sse {
    stream  // SSE响应不应用deadline
} else {
    enforce_response_deadline(stream, deadline)
};
```

**风险描述**:
SSE（Server-Sent Events）连接被跳过deadline检查，这可能导致：
- 客户端断开但服务器未检测到的"僵尸连接"累积
- 服务器资源被长期占用
- 内存泄漏（每个连接占用缓冲区）

**修复建议**:
```rust
// 为SSE连接添加最大持续时间（如2小时）和保活检查
const SSE_MAX_DURATION: Duration = Duration::from_secs(7200); // 2小时
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30); // 30秒保活

fn enforce_sse_deadline(stream: ProxyBodyStream, deadline: Instant) -> ProxyBodyStream {
    let max_deadline = Instant::now() + SSE_MAX_DURATION;
    let effective_deadline = if deadline < max_deadline { deadline } else { max_deadline };

    enforce_response_deadline(stream, effective_deadline)
}

// 或者使用保活机制
struct SseKeepalive {
    last_activity: Arc<AtomicInstant>,
}

impl SseKeepalive {
    fn check_activity(&self) -> bool {
        let last = self.last_activity.load(Ordering::Relaxed);
        let now = Instant::now();
        // 如果超过60秒没有活动，发送保活注释
        now.duration_since(last).as_secs() < 60
    }
}
```

---

### 2.6 🟡 中风险：后台任务无限制累积

**位置**: `src/server.rs:885-896` 和 `1207`

**问题代码**:
```rust
// 在错误处理中
let token_clone = token.clone();
let manager = Arc::clone(api_key_manager);
tokio::spawn(async move {
    manager
        .report_request_result(&token_clone, /* ... */)
        .await;
});

// 在Drop中
impl Drop for ResponseCompletionGuard {
    fn drop(&mut self) {
        // ...
        tokio::spawn(async move {
            manager.report_request_result(/* ... */).await;
        });
    }
}
```

**风险描述**:
- 每次请求失败或完成都会spawn一个后台任务
- 如果请求量很大（如10k RPS），可能累积大量后台任务
- 可能导致任务调度器过载

**修复建议**:
```rust
use tokio::sync::mpsc;
use std::sync::LazyLock;

// 使用全局channel批量处理
static REPORT_SENDER: LazyLock<mpsc::Sender<ReportTask>> = LazyLock::new(|| {
    let (tx, mut rx) = mpsc::channel::<ReportTask>(10000);

    // 启动单个后台任务处理所有上报
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(100);
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                Some(task) = rx.recv() => {
                    batch.push(task);
                    if batch.len() >= 100 {
                        process_batch(std::mem::take(&mut batch)).await;
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        process_batch(std::mem::take(&mut batch)).await;
                    }
                }
            }
        }
    });

    tx
});

struct ReportTask {
    token: String,
    result: RequestResult,
}

// 在需要上报时，发送到channel而不是spawn新任务
async fn report_request_result(token: &str, result: RequestResult) {
    let task = ReportTask {
        token: token.to_string(),
        result,
    };

    // 使用try_send避免阻塞，如果channel满则丢弃
    let _ = REPORT_SENDER.try_send(task);
}
```

---

### 2.7 🟡 中风险：连接池配置缺失

**位置**: `src/server.rs:1513-1527`

**问题代码**:
```rust
fn build_upstream_client(upstream: &UpstreamConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(upstream.connect_timeout_ms));

    if let Some(proxy) = &upstream.proxy {
        // ...
    }

    builder.build().map_err(|err| format!("failed to build reqwest client: {err}"))
}
```

**风险描述**:
没有配置：
- 连接池最大空闲连接数
- 空闲连接超时时间
- 连接复用策略

这可能导致：
- 连接数无限增长
- 长时间空闲连接占用资源
- 对端关闭的连接未清理

**修复建议**:
```rust
fn build_upstream_client(upstream: &UpstreamConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(upstream.connect_timeout_ms))
        // 添加连接池配置
        .pool_max_idle_per_host(32)  // 每个主机最大空闲连接
        .pool_idle_timeout(Duration::from_secs(90))  // 空闲连接超时
        .tcp_keepalive(Duration::from_secs(60))  // TCP保活
        .timeout(Duration::from_millis(upstream.request_timeout_ms)); // 全局请求超时

    if let Some(proxy) = &upstream.proxy {
        // ...
    }

    builder.build().map_err(|err| format!("failed to build reqwest client: {err}"))
}
```

---

### 2.8 🟡 中风险：限流算法临界突发

**位置**: `src/ratelimit.rs:132-154`

**问题描述**:
使用固定窗口算法，在窗口边界处可能有2倍流量突发：
- 时间 00:59 来一波请求（达到限制）
- 时间 01:00 窗口重置，又来一波请求（达到限制）
- 实际上在1秒内处理了2倍限制的请求

**修复建议**:
```rust
// 使用滑动窗口算法
pub struct SlidingWindowLimiter {
    window_size_secs: u64,
    max_requests: u64,
    windows: Arc<Mutex<Vec<(u64, u64)>>>, // (窗口开始时间, 计数)
}

impl SlidingWindowLimiter {
    pub fn check(&self, now: u64) -> RateLimitDecision {
        let mut windows = self.state.lock().unwrap();

        // 清理过期窗口
        let cutoff = now.saturating_sub(self.window_size_secs);
        windows.retain(|(start, _)| *start > cutoff);

        // 计算当前窗口内的请求总数
        let current_count: u64 = windows.iter().map(|(_, count)| count).sum();

        if current_count >= self.max_requests {
            return RateLimitDecision::Rejected {
                retry_after_secs: self.calculate_retry_after(&windows, now),
            };
        }

        // 记录当前请求
        windows.push((now, 1));
        RateLimitDecision::Allowed
    }
}
```

---

## 3. 可执行的稳定性增强代码

### 3.1 统一超时配置结构

```rust
// src/config.rs 添加
#[derive(Debug, Clone, Deserialize)]
pub struct TimeoutConfig {
    /// 连接建立超时（毫秒）
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,

    /// 请求总超时（毫秒）
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,

    /// 空闲连接超时（秒）
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,

    /// SSE连接最大持续时间（分钟）
    #[serde(default = "default_sse_max_duration_mins")]
    pub sse_max_duration_mins: u64,

    /// 优雅关闭等待时间（秒）
    #[serde(default = "default_graceful_shutdown_secs")]
    pub graceful_shutdown_secs: u64,
}

fn default_connect_timeout_ms() -> u64 { 10_000 }
fn default_request_timeout_ms() -> u64 { 60_000 }
fn default_idle_timeout_secs() -> u64 { 90 }
fn default_sse_max_duration_mins() -> u64 { 120 }
fn default_graceful_shutdown_secs() -> u64 { 30 }
```

### 3.2 增强版限流器（带清理）

```rust
// src/ratelimit.rs
use std::time::{Duration, Instant};
use dashmap::DashMap; // 使用DashMap替代Mutex<HashMap>

pub struct RateLimiterManager {
    global_config: Option<RateLimitConfig>,
    key_limiters: DashMap<String, RateLimiterEntry>, // 并发安全的HashMap
    cleanup_interval: Duration,
    entry_ttl: Duration,
}

struct RateLimiterEntry {
    limiter: RateLimiter,
    last_accessed: Instant,
}

impl RateLimiterManager {
    pub fn new(global_config: Option<RateLimitConfig>) -> Self {
        let manager = Self {
            global_config,
            key_limiters: DashMap::new(),
            cleanup_interval: Duration::from_secs(600), // 10分钟
            entry_ttl: Duration::from_secs(3600),       // 1小时
        };

        // 启动后台清理任务
        manager.start_cleanup_task();
        manager
    }

    fn start_cleanup_task(&self) {
        let limiters = self.key_limiters.clone();
        let interval = self.cleanup_interval;
        let ttl = self.entry_ttl;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;
                let now = Instant::now();

                limiters.retain(|_, entry| {
                    now.duration_since(entry.last_accessed) < ttl
                });

                tracing::info!("Rate limiter cleanup completed, remaining entries: {}", limiters.len());
            }
        });
    }
}
```

### 3.3 请求上下文和取消令牌

```rust
// src/server.rs
use tokio_util::sync::CancellationToken;

pub struct RequestContext {
    pub request_id: String,
    pub start_time: Instant,
    pub cancellation_token: CancellationToken,
    pub timeout_duration: Duration,
}

impl RequestContext {
    pub fn new(request_id: String, timeout_ms: u64) -> Self {
        Self {
            request_id,
            start_time: Instant::now(),
            cancellation_token: CancellationToken::new(),
            timeout_duration: Duration::from_millis(timeout_ms),
        }
    }

    pub fn create_timeout_future(&self) -> tokio::time::Sleep {
        tokio::time::sleep(self.timeout_duration)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }
}

// 在请求处理中使用
async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
) -> Response<Body> {
    let request_id = observability::extract_or_generate_request_id(request.headers());
    let ctx = RequestContext::new(request_id.clone(), 60_000); // 60秒全局超时

    // 在关键检查点检查取消
    if ctx.is_cancelled() {
        return json_error(StatusCode::REQUEST_TIMEOUT, "request_cancelled");
    }

    // ...
}
```

### 3.4 健康检查和熔断器

```rust
// src/circuit_breaker.rs
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::time::{Duration, Instant};

pub struct CircuitBreaker {
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,
    state: AtomicU32, // 0=Closed, 1=Open, 2=HalfOpen
    failure_count: AtomicU32,
    success_count: AtomicU32,
    last_failure_time: Mutex<Option<Instant>>,
}

impl CircuitBreaker {
    pub async fn call<F, Fut, T>(&self, f: F) -> Result<T, CircuitBreakerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
    {
        match self.state.load(Ordering::Relaxed) {
            0 => self.call_closed(f).await,  // Closed
            1 => {  // Open
                let last = *self.last_failure_time.lock().unwrap();
                if let Some(time) = last {
                    if Instant::now().duration_since(time) > self.timeout {
                        self.state.store(2, Ordering::Relaxed); // 转为HalfOpen
                        self.call_half_open(f).await
                    } else {
                        Err(CircuitBreakerError::Open)
                    }
                } else {
                    Err(CircuitBreakerError::Open)
                }
            }
            2 => self.call_half_open(f).await,  // HalfOpen
            _ => unreachable!(),
        }
    }

    async fn call_closed<F, Fut, T>(&self, f: F) -> Result<T, CircuitBreakerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
    {
        match f().await {
            Ok(result) => {
                self.failure_count.store(0, Ordering::Relaxed);
                Ok(result)
            }
            Err(_) => {
                let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= self.failure_threshold {
                    self.state.store(1, Ordering::Relaxed);
                    *self.last_failure_time.lock().unwrap() = Some(Instant::now());
                }
                Err(CircuitBreakerError::Failure)
            }
        }
    }
}
```

---

## 4. 监控和告警建议

### 4.1 关键指标

```rust
// 添加到 observability.rs
pub struct StabilityMetrics {
    // 资源使用
    pub inflight_requests: Gauge,
    pub semaphore_wait_time: Histogram,
    pub rate_limiter_entries: Gauge,
    pub concurrency_semaphore_entries: Gauge,

    // 错误率
    pub upstream_timeouts: Counter,
    pub upstream_connect_errors: Counter,
    pub circuit_breaker_opens: Counter,
    pub request_cancellations: Counter,

    // 性能
    pub request_queue_time: Histogram, // 从接收到开始处理的时间
    pub response_stream_duration: Histogram, // 流响应持续时间
}
```

### 4.2 告警阈值

| 指标 | 警告阈值 | 严重阈值 | 说明 |
|------|---------|---------|------|
| inflight_requests | >80%容量 | >95%容量 | 并发接近上限 |
| upstream_timeouts | >1% | >5% | 上游超时率 |
| rate_limiter_entries | >10万 | >50万 | 限流器内存泄漏 |
| request_queue_time | >100ms | >500ms | 请求排队时间过长 |
| circuit_breaker_opens | >10/分钟 | >50/分钟 | 熔断频繁触发 |

---

## 5. 总结和行动项

### 5.1 修复优先级

**P0（立即修复）**:
1. 修复 `concurrency.rs:91` 的 `available_permits()` bug
2. 为 `ratelimit.rs` 添加限流器清理机制
3. 为 `concurrency.rs` 添加信号量清理机制
4. 实现优雅关闭机制

**P1（本周修复）**:
5. 为SSE连接添加超时保护
6. 优化后台任务上报机制（使用channel批量处理）
7. 配置连接池参数

**P2（下月修复）**:
8. 实现滑动窗口限流算法
9. 添加熔断器机制
10. 实现请求上下文和取消令牌

### 5.2 配置建议

```yaml
# 推荐的稳定性配置
gateway:
  timeouts:
    connect_ms: 10000
    request_ms: 60000
    idle_secs: 90
    sse_max_duration_mins: 120
    graceful_shutdown_secs: 30

  limits:
    max_idle_connections_per_host: 32
    rate_limiter_cleanup_interval_mins: 10
    rate_limiter_entry_ttl_mins: 60
    max_semaphore_entries: 100000

  circuit_breaker:
    failure_threshold: 5
    success_threshold: 3
    timeout_secs: 30
```

### 5.3 代码审查建议

- 所有 `spawn` 调用必须有任务句柄管理或超时控制
- 所有 `HashMap` 缓存必须有大小限制和清理机制
- 所有流处理必须有超时或取消机制
- 所有配置值必须在合理范围内验证

---

**报告生成时间**: 2026-03-03
**分析师**: Claude Code Stability Analyzer
**版本**: 1.0

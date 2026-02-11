# AI Gateway Lite（Rust）设计说明

## 0. 目标与定位

这是一个“个人自用”的轻量 AI Gateway，用于：

- **隐藏真实上游 API Key**（客户端只持有网关 Token）
- **统一入口 + 路由转发**（例如 `/openai/*` → `https://api.openai.com/*`）
- **按配置注入/替换上游鉴权 Header**（支持任意 Header 形式，如 `x-api-key: ...`）
- **请求/响应流式透传**（包含 SSE，避免大包内存聚合）
- **尽量隐藏用户 IP 给上游**（不透传 XFF/Forwarded，所有请求从网关出网）
- **支持入站 HTTPS**（可加载证书/私钥，或自动生成自签名证书）

非目标（刻意不做）：

- 多租户计费/账单系统
- 复杂策略引擎（RBAC、ABAC）
- 分布式高可用、全链路追踪、大规模日志管道

> 该工具建议作为：本机/家用服务器/小型 VPS 上运行的反向代理服务。

### 0.1 实现阶段约束（当前）

当前已实现（Phase 1 + Phase 2 部分能力）：

- 多 route 前缀匹配与 URL 重写
- 入站 token 鉴权
- 上游 header 注入/覆盖 + 敏感头移除
- 请求/响应流式透传（含 SSE）
- 启动时加载配置（静态配置）
- 下游固定窗口限流（按 token + route）
- 并发保护（下游全局 + 上游按 route + key）
- 可观测性：结构化日志、`/metrics`、低采样 tracing（OTLP 可选）

仍暂缓到后续阶段：

- 配置热加载
- 复杂重试策略

------

## 1. 核心行为定义

以 `/openai` 为例：

客户端请求：

- `POST http://<gateway>/openai/v1/chat/completions`
- Header：`Authorization: Bearer <GW_TOKEN>`

网关处理流程（必须）：

1. **路由匹配**：基于 `prefix=/openai` 命中路由
   - 未命中任何路由时返回 `404 {"error":"route_not_found"}`
2. **鉴权**：校验 `<GW_TOKEN>` 是否为配置允许值（或命中 allowlist）
3. **重写 URL**：将 `/openai` strip 掉后拼接到上游 base url
   - upstream_url = `https://api.openai.com` + `/v1/chat/completions` + `?query`
4. **Header 处理**：
   - 删除/覆盖入站的敏感 header（尤其是 `Authorization`、`X-Forwarded-For` 等）
   - 按配置**注入上游鉴权 header**（可为任意 header 名称与格式）
5. **Body 透传**：请求体直接流式转发（不整体缓存）
6. **响应透传（含 SSE）**：响应体流式回传，保持状态码与必要头信息

------

## 2. 多上游 API 支持：统一抽象

不同上游的“鉴权注入方式”不统一：

- 有的用 `Authorization: Bearer XXX`
- 有的用 `x-api-key: XXX`（如 Claude/Anthropic）
- 可能还需要额外固定头（例如版本头、beta header 等）

因此网关应提供一个统一机制：**Upstream Header Injection**。

### 2.1 Upstream Header Injection 规则

每条路由可配置：

- `inject_headers`: 需要注入到上游请求的 header 列表
- `remove_headers`: 转发前需要移除的 header 列表（默认包含 `authorization`, `x-forwarded-for`, `forwarded`, `cf-connecting-ip`, `true-client-ip` 等）
- `inject_headers[].value` 支持从环境变量引用或从配置 secrets 引用（建议默认从 env 取，避免明文写入文件）

示例需求（必须支持）：

- Claude/Anthropic 形式：`x-api-key: $ANTHROPIC_API_KEY`

------

## 3. 配置文件规范（YAML）

> 设计原则：**可读、可演进、少字段**（第一阶段静态加载，后续可扩展热加载）。
> 机密建议来自环境变量，避免写死在配置里。

### 3.1 顶层结构

```yaml
listen: "127.0.0.1:8080"

# 可选：开启 user -> gateway HTTPS
inbound_tls:
  # 若 cert_path/key_path 同时提供，则直接加载
  # cert_path: "./certs/server.crt"
  # key_path: "./certs/server.key"
  # 若不提供 cert/key，则使用自签名证书路径：
  # 文件存在则加载，不存在则自动生成并落盘
  self_signed_cert_path: "./certs/gateway-selfsigned.crt"
  self_signed_key_path: "./certs/gateway-selfsigned.key"

# 网关入站鉴权（客户端用）
gateway_auth:
  # 支持多个 token（个人用通常 1 个即可）
  tokens:
    - "gw_example_token_1"
  # 从哪些 header 取 token（按顺序尝试）
  # 默认推荐 Authorization: Bearer <token>
  token_sources:
    - type: "authorization_bearer"
    - type: "header"
      name: "x-gw-token"

routes:
  - id: "openai"
    prefix: "/openai"
    upstream:
      base_url: "https://api.openai.com"
      strip_prefix: true
      connect_timeout_ms: 10000
      request_timeout_ms: 60000
      # 可选：覆盖全局上游按 key 并发上限（每个 key）
      upstream_key_max_inflight: 8
      # 可选：gateway -> upstream 出站代理
      proxy:
        protocol: "http" # http / https / socks
        address: "127.0.0.1:7890"
        username: "${PROXY_USERNAME}"
        password: "${PROXY_PASSWORD}"
      # 上游鉴权注入（可自定义 header）
      inject_headers:
        - name: "authorization"
          value: "Bearer ${OPENAI_API_KEY}"
      # 转发前移除（不把客户端 token/真实ip传上游）
      remove_headers:
        - "authorization"
        - "x-forwarded-for"
        - "forwarded"
        - "cf-connecting-ip"
        - "true-client-ip"
      # 是否把 x-forwarded-for 传给上游（默认 false）
      forward_xff: false

  - id: "anthropic"
    prefix: "/claude"
    upstream:
      base_url: "https://api.anthropic.com"
      strip_prefix: true
      connect_timeout_ms: 10000
      request_timeout_ms: 60000
      proxy:
        protocol: "socks"
        address: "127.0.0.1:1080"
      inject_headers:
        - name: "x-api-key"
          value: "${ANTHROPIC_API_KEY}"
        # 可选：允许配置额外固定头（如版本头）
        - name: "anthropic-version"
          value: "2023-06-01"
      remove_headers:
        - "authorization"
        - "x-api-key"
        - "x-forwarded-for"
        - "forwarded"
      forward_xff: false

cors:
  enabled: false
  # 如果你要浏览器直接访问网关再打开
  allow_origins: ["https://your.site"]
  allow_headers: ["authorization", "content-type", "x-gw-token"]
  allow_methods: ["GET", "POST", "OPTIONS"]
  expose_headers: []

rate_limit:
  per_minute: 120

concurrency:
  downstream_max_inflight: 100
  upstream_per_key_max_inflight: 8

observability:
  logging:
    level: "info"
    format: "json" # json / text
    to_stdout: true
    file:
      enabled: true
      dir: "./logs"
      prefix: "ai-gw-lite"
      rotation: "daily" # minutely / hourly / daily / never
      max_files: 7
  metrics:
    enabled: true
    path: "/metrics"
    token: "${GW_METRICS_TOKEN}"
  tracing:
    enabled: true
    sample_ratio: 0.05
    otlp:
      endpoint: "http://127.0.0.1:4317"
      timeout_ms: 3000
```

> 当前已实现 `rate_limit` 与 `concurrency`；`reload` 与重试能力仍为后续阶段。

### 3.2 值插值规则（必须）

`inject_headers[].value` 与 `gateway_auth.tokens[]` 支持：

- `${ENV_NAME}`：从环境变量读取（建议默认）
- 若 env 不存在：启动时报错或按配置策略决定（建议启动失败，避免静默无鉴权）

### 3.3 上游代理配置（可选）

`routes[].upstream.proxy` 用于配置 gateway 到上游之间的代理链路：

- `protocol`: `http` / `https` / `socks`
- `address`: `host:port`
- `username` 与 `password`: 可选，若配置必须同时提供

实现要求：

- 代理能力按路由生效（每条 route 可独立配置）
- 不配置 `proxy` 时保持直连行为

### 3.4 入站 TLS 配置（可选）

`inbound_tls` 用于配置 `user -> gateway` 的 HTTPS 监听能力。

- `cert_path` / `key_path`：可选，若提供必须成对出现
- `self_signed_cert_path` / `self_signed_key_path`：自签名证书与私钥落盘路径（有默认值）

实现要求：

- 不配置 `inbound_tls` 时保持 HTTP 监听
- 配置 `inbound_tls` 且提供 `cert_path`/`key_path` 时，加载指定证书
- 配置 `inbound_tls` 且未提供 `cert_path`/`key_path` 时：
  - 若 `self_signed_*` 文件已存在，直接加载
  - 若不存在，自动生成自签名证书并加载

### 3.5 CORS 配置（可选）

`cors.enabled=true` 时，网关会处理浏览器 preflight（`OPTIONS`）并在常规响应上注入 CORS 响应头。

实现要求：

- preflight 通过时返回 `204`，并携带：
  - `Access-Control-Allow-Origin`
  - `Access-Control-Allow-Methods`
  - `Access-Control-Allow-Headers`
- 常规响应（含错误响应）在 origin 命中时携带：
  - `Access-Control-Allow-Origin`
  - 可选 `Access-Control-Expose-Headers`
- `allow_origins` 推荐填写完整 origin（如 `https://fy.ciallo.fans`）
- 兼容无协议写法（如 `fy.ciallo.fans`）用于匹配请求 origin 的 host

### 3.6 限流与并发配置（Phase 2 已实现部分）

`rate_limit`（下游限流）：

- `per_minute`：每分钟请求上限（`> 0`）
- 维度：`token + route`
- 超限返回：`429 {"error":"rate_limited"}`
- 响应头：`Retry-After`

`concurrency`（并发保护）：

- `downstream_max_inflight`：下游全局并发上限（`> 0`）
- `upstream_per_key_max_inflight`：上游按 route + key 并发上限（`> 0`）
- `routes[].upstream.upstream_key_max_inflight`：按路由覆盖上游并发上限
- 上游 key 识别 header 固定为：`authorization`、`x-api-key`

并发超限返回：

- 下游：`503 {"error":"downstream_concurrency_exceeded"}`
- 上游：`503 {"error":"upstream_concurrency_exceeded"}`

### 3.7 可观测性配置（已实现）

- `observability.logging`
  - `level`：日志级别过滤（默认 `info`）
  - `format`：`json` 或 `text`（默认 `json`）
  - `to_stdout`：是否输出到控制台（默认 `true`）
  - `file`：文件日志配置（目录、前缀、滚动周期、保留数量）
- `observability.metrics`
  - `enabled=true` 时启用 metrics 端点
  - `path` 默认 `/metrics`，必须以 `/` 开头且不能与 `/healthz` 冲突
  - `token` 为独立 metrics 鉴权 token（不复用 `GW_TOKEN`）
- `observability.tracing`
  - `enabled` 控制 tracing 开关
  - `sample_ratio` 范围 `[0.0, 1.0]`（默认 `0.05`）
  - `otlp` 可选；配置 `endpoint` 时导出到 OTLP collector

------

## 4. HTTP/转发细节规范

### 4.1 URL 拼接与路径重写

- 命中路由 `prefix` 后：
  - `strip_prefix=true`：从 `req.path` 去掉 `prefix`，剩余部分为 `rest_path`
  - 若 `rest_path` 为空，则置为 `/`
- `upstream_url = upstream.base_url + rest_path + ("?" + query if exists)`

边界：

- `prefix` 必须以 `/` 开头
- 路由匹配使用**最长前缀优先**
- 前缀必须满足**路径段边界**：
  - `/openai` 可匹配 `/openai`、`/openai/...`
  - `/openai` 不可匹配 `/openai2`
- `rest_path` 必须保证以 `/` 开头
- `base_url` 尾部 `/` 与 `rest_path` 头部 `/` 拼接后不得产生双斜杠（`//`）

### 4.2 Header 处理顺序（推荐）

请求转发前：

1. 复制入站 headers（个人工具可先全复制再删）
2. 移除 hop-by-hop headers（大小写不敏感）：
   - `connection`
   - `keep-alive`
   - `proxy-authenticate`
   - `proxy-authorization`
   - `te`
   - `trailer`
   - `transfer-encoding`
   - `upgrade`
3. 移除 `remove_headers`（大小写不敏感）
4. 若 `forward_xff=false`：确保 `x-forwarded-for`、`forwarded` 等不存在
5. 应用 `inject_headers`：
   - 若目标 header 已存在，**覆盖**（建议覆盖，避免污染）
6. 设置正确的 `Host`（通常由 HTTP client 自动设置；若使用 hyper 客户端需注意）

响应回写客户端前：

1. 保持上游状态码不变
2. 复制上游响应 headers 后移除 hop-by-hop headers
3. 不改写 `text/event-stream` 的响应头与分块行为

**日志/错误输出严禁打印**：

- 入站 `Authorization`
- 注入后的任何密钥 header

### 4.3 Body 与响应流式透传（必须）

- 入站请求体：以 stream 方式转发到上游（不要 `collect` 到内存）
- 上游响应体：以 stream 方式写回客户端
- 支持 SSE：不对 chunk 做聚合、不对 `text/event-stream` 做改写

### 4.4 超时与重试（个人工具建议）

- 第一阶段建议拆分超时语义：
  - `connect_timeout_ms`：连接上游超时（建议 5s~10s）
  - `request_timeout_ms`：非流式请求总超时（建议 60s）
  - 对 SSE/长流：不设置总超时，避免中途被网关切断
- 第一阶段不启用自动重试（避免重复提交副作用请求）

------

## 5. 鉴权（入站）规范

### 5.1 Token 提取

按 `gateway_auth.token_sources` 顺序尝试：

- `authorization_bearer`：解析 `Authorization: Bearer <token>`
- `header`：读取指定 header 值

### 5.2 校验规则

- token 必须存在且在 allowlist 内
- 校验失败返回 `401 Unauthorized`
- 为避免信息泄漏，错误消息统一：
  - `{"error":"unauthorized"}`

------

## 6. 第二阶段：限流与并发控制（已实现）

### 6.1 固定窗口限流

- key = `{token}:{route_id}:{minute_bucket}`
- 计数器保存在内存 `HashMap`，按分钟窗口轮转清理
- 超过 `per_minute` 返回 `429 {"error":"rate_limited"}`
- 返回头：
  - `Retry-After: <seconds_to_next_minute>`

### 6.2 并发限制

- 下游：全局 `Semaphore(downstream_max_inflight)`
- 上游：按“route + 上游 key”分组的 `Semaphore(upstream_per_key_max_inflight)`
  - key 仅从 `upstream.inject_headers[].value` 提取，header 识别顺序固定为 `authorization`、`x-api-key`
  - 可由 `routes[].upstream.upstream_key_max_inflight` 覆盖默认上限
- 每个请求开始时 acquire，在响应体生命周期结束时 release（包含 SSE 长流）
- 超限返回 `503 Service Unavailable`：
  - 下游：`{"error":"downstream_concurrency_exceeded"}`
  - 上游：`{"error":"upstream_concurrency_exceeded"}`

------

## 7. 第二阶段：动态路由/配置热加载（暂缓）

> 当前版本仍不实现本节能力，以下内容作为后续扩展设计保留。

你要求“实时动态绑定转发路径”，个人工具推荐两种简单方式：

### 7.1 文件监听热加载（推荐）

- 使用 `notify` crate 监听配置文件变更
- 重新 parse 配置，校验通过后用 `ArcSwap` 原子替换运行时配置
- 校验失败：保留旧配置，打印错误日志

### 7.2 每次请求检查 mtime（更简单）

- 适合低 QPS
- 如果 mtime 改变则 reload（仍然需要原子替换）

------

## 8. 组件与工程结构（Rust）

推荐用 **axum（server） + hyper（client）** 或 **axum + reqwest**。

**注意**：reqwest 也能流式，但要确保不无意 `bytes().await` 之类读取全量。

建议模块划分：

- `config.rs`：YAML 解析、插值（env）、校验
- `auth.rs`：入站 token 提取与校验
- `proxy.rs`：路由匹配、URL 重写、header 处理辅助函数
- `server.rs`：HTTP 入口、请求处理流程、上游转发与错误映射
- `observability.rs`：tracing 初始化、metrics 注册与 request-id 工具
- `main.rs`：启动参数解析、配置加载、服务启动
- `ratelimit.rs`：固定窗口限流计数器
- `concurrency.rs`：下游/上游并发保护
- `reload.rs`（第二阶段）：热加载（notify + ArcSwap）

运行时共享状态（Arc）：

- 当前：`Arc<AppConfig>` + `RateLimiter` + `ConcurrencyController` + `ObservabilityRuntime`
- 后续可演进：`ArcSwap<AppConfig>`（用于热加载）

------

## 9. 最小测试清单（交付 Codex 时强制要求）

1. 路由正确性：
   - `/openai/v1/models` → `https://api.openai.com/v1/models`
2. 路由边界：
   - `/openai2/v1/models` 不应命中 `prefix=/openai`
   - 多条可匹配路由时应命中最长前缀
3. strip_prefix 边界：
   - `/openai` → `/`
4. 入站鉴权：
   - 无 token / 错 token 返回 401
5. Header 注入覆盖：
   - 客户端传了 `Authorization`，上游收到的应为注入值
6. 不透传用户 IP：
   - 上游请求中不存在 `X-Forwarded-For/Forwarded/CF-Connecting-IP`
7. hop-by-hop 头处理：
   - 请求与响应均不应携带 `connection/transfer-encoding/upgrade` 等 hop-by-hop headers
8. SSE 流式：
   - `text/event-stream` 能边产生边回传，客户端不中断
9. 超时行为：
   - 上游连接超时返回可识别错误（建议 504）
   - SSE/长流不会被 `request_timeout_ms` 误切断
10. 限流行为（Phase 2）：
   - 同 token + route 在同一分钟内超过阈值后返回 429，含 `Retry-After`
11. 并发行为（Phase 2）：
   - 下游并发超限返回 503
   - 上游相同 key 并发超限返回 503，不同 key 可并行
12. 可观测性行为：
   - `/metrics` 未授权返回 401，授权后返回 Prometheus 指标文本
13. request-id 行为：
   - 客户端提供 `x-request-id` 时网关回传同值；未提供时网关自动生成

------

## 10. 示例：多 Provider 鉴权 Header 注入

在文档层面强调：**不要把“上游 key”称为“api key 鉴权”**（避免混淆）。客户端永远只持有 `GW_TOKEN`。

- 对 OpenAI：常见注入
  - `authorization: Bearer ${OPENAI_API_KEY}`
- 对 Anthropic（Claude）：常见注入
  - `x-api-key: ${ANTHROPIC_API_KEY}`
  - 以及可选固定头（按你需要配置）

------

## 11. Codex 工程实现交付要求（摘要）

Codex 实现应交付：

- 可执行二进制：`ai-gw-lite`
- 支持参数：
  - `--config /path/to/config.yaml`
- 第一阶段支持：
  - 多 route（prefix 匹配）
  - 路由段边界匹配 + 最长前缀优先
  - 入站 token 校验
  - 注入自定义 headers（支持 env 插值）
  - 移除敏感 headers（大小写不敏感）
  - 移除 hop-by-hop headers（请求与响应两侧）
  - 请求/响应流式透传（含 SSE）
  - 基础超时控制（connect/request 分离）

第二阶段已支持：

- 限流（下游，固定窗口）
- 并发控制（下游全局 + 上游按 key）

第二阶段暂缓项：

- 配置热加载（notify）
- 自动重试策略

------

## 12. 当前实现状态（2026-02-11）

已完成项（代码已落地）：

- `axum + reqwest` 主链路已接入（服务启动、fallback 代理、上游流式转发）
- 路由段边界匹配 + 最长前缀优先
- 入站鉴权（Bearer / 自定义 header 来源）
- 请求与响应两侧 hop-by-hop 头清洗
- 上游 header 注入覆盖与敏感头移除
- 上游代理（http/https/socks）与代理认证支持
- 入站 HTTPS：支持加载 cert/key，或自动生成并复用自签名证书
- CORS：支持 preflight 与常规响应头注入
- SSE 透传与基础超时映射
- 下游限流：固定窗口（`token + route + minute`），超限返回 429 + `Retry-After`
- 并发保护：
  - 下游全局并发上限
  - 上游按 key 并发上限（支持按路由覆盖）
- 单元测试 + e2e 测试覆盖核心 DoD

说明：

- 本节用于标注“当前代码实际状态”，若后续实现变化请同步更新。

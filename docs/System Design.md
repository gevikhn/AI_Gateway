# AI Gateway Lite（Rust）设计说明

## 0. 目标与定位

这是一个“个人自用”的轻量 AI Gateway，用于：

- **隐藏真实上游 API Key**（客户端只持有网关 Token）
- **统一入口 + 路由转发**（例如 `/openai/*` → `https://api.openai.com/*`）
- **按配置注入/替换上游鉴权 Header**（支持任意 Header 形式，如 `x-api-key: ...`）
- **请求/响应流式透传**（包含 SSE，避免大包内存聚合）
- **尽量隐藏用户 IP 给上游**（不透传 XFF/Forwarded，所有请求从网关出网）

非目标（刻意不做）：

- 多租户计费/账单系统
- 复杂策略引擎（RBAC、ABAC）
- 分布式高可用、全链路追踪、大规模日志管道

> 该工具建议作为：本机/家用服务器/小型 VPS 上运行的反向代理服务。

### 0.1 实现阶段约束（当前）

第一阶段（本次实现）只做“基础路由转发”：

- 多 route 前缀匹配与 URL 重写
- 入站 token 鉴权
- 上游 header 注入/覆盖 + 敏感头移除
- 请求/响应流式透传（含 SSE）
- 启动时加载配置（静态配置）

暂缓到后续阶段：

- 限流与并发控制
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
```

> 第一阶段不实现 `rate_limit`、`concurrency`、`reload`，配置中可暂不出现这些字段。

### 3.2 值插值规则（必须）

`inject_headers[].value` 与 `gateway_auth.tokens[]` 支持：

- `${ENV_NAME}`：从环境变量读取（建议默认）
- 若 env 不存在：启动时报错或按配置策略决定（建议启动失败，避免静默无鉴权）

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

## 6. 第二阶段：限流与并发控制（暂缓）

> 第一阶段不实现本节能力，以下内容作为后续扩展设计保留。

### 6.1 固定窗口限流（推荐最简）

- key = `{token}:{route_id}:{minute_bucket}`
- 计数器保存在内存 `HashMap`，每分钟轮转清理
- 超过 `per_minute` 返回 `429 Too Many Requests`
- 返回头：
  - `Retry-After: <seconds_to_next_minute>`（可选）

### 6.2 并发限制（强烈建议）

- 全局一个 `Semaphore(max_inflight)`
- 每个请求开始 acquire，结束 release
- 超限返回 `503 Service Unavailable` 或 `429`（任选其一，建议 503）

------

## 7. 第二阶段：动态路由/配置热加载（暂缓）

> 第一阶段不实现本节能力，以下内容作为后续扩展设计保留。

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
- `proxy.rs`：构造上游请求、header 处理、body/response 流式转发
- `main.rs`：启动、路由挂载、全局状态
- `ratelimit.rs`（第二阶段）：固定窗口计数器
- `reload.rs`（第二阶段）：热加载（notify + ArcSwap）

运行时共享状态（Arc）：

- 第一阶段：当前配置 `Arc<AppConfig>`
- 第二阶段：可切换为 `ArcSwap<AppConfig>` 并加入限流器、并发 semaphore

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

第二阶段（暂缓）：

- 限流 + 并发控制
- 配置热加载（notify）

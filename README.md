# AI Gateway Lite (Rust)

一个面向个人/小团队的轻量 AI 代理网关，用于统一入口、隐藏上游密钥、进行鉴权与安全头处理，并支持流式透传（含 SSE）。

## 1. 功能概览

当前已实现（Phase 1 + Phase 2 部分能力）：
- 多路由前缀转发（最长前缀优先 + 路径段边界）
- 入站 `GW_TOKEN` 鉴权（Bearer 或自定义 Header）
- 入站 HTTP/HTTPS（TLS 可选）
- 上游 `inject_headers` 注入/覆盖
- 敏感头与 hop-by-hop 头移除
- 请求/响应流式透传（SSE 不做聚合改写）
- 超时控制（`connect_timeout_ms` / `request_timeout_ms`）
- 下游固定窗口限流（按 token + route，分钟窗口）
- 并发保护：
  - 下游全局并发上限
  - 上游按 route + key 并发上限（支持按路由覆盖）

暂未实现（Phase 2）：
- 配置热加载
- 自动重试策略

## 2. 快速开始

### 2.1 环境要求
- Rust stable（建议 1.80+）
- 可访问目标上游 API 的网络环境

### 2.2 编译

开发构建：

```bash
cargo build
```

发布构建：

```bash
cargo build --release
```

生成的可执行文件：
- Linux/macOS: `target/release/ai-gw-lite`
- Windows: `target/release/ai-gw-lite.exe`

### 2.3 准备配置

项目内有示例配置：`config/dev.yaml`。你也可以自定义 `config.yaml`，运行时通过 `--config` 指定。

### 2.4 启动

```bash
cargo run -- --config config/dev.yaml
```

或直接运行发布二进制：

```bash
./target/release/ai-gw-lite --config /path/to/config.yaml
```

Windows:

```powershell
.\target\release\ai-gw-lite.exe --config .\config\dev.yaml
```

### 2.5 健康检查

```bash
curl http://127.0.0.1:8080/healthz
```

若开启 `inbound_tls`，使用：

```bash
curl -k https://127.0.0.1:8080/healthz
```

预期返回：

```json
{"status":"ok"}
```

## 3. `config.yaml` 详细说明

## 3.1 完整示例

```yaml
listen: "127.0.0.1:8080"

# 可选：开启入站 HTTPS
# inbound_tls:
#   cert_path: "./certs/server.crt"
#   key_path: "./certs/server.key"
#   # 若不提供 cert/key，会自动使用下列自签名路径：
#   # 若文件存在则直接加载，不存在则自动生成
#   self_signed_cert_path: "./certs/gateway-selfsigned.crt"
#   self_signed_key_path: "./certs/gateway-selfsigned.key"

gateway_auth:
  tokens:
    - "${GW_TOKEN}" # 建议从环境变量注入
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
      # 可选：按路由配置出站代理
      proxy:
        protocol: "http" # http | https | socks
        address: "127.0.0.1:7890"
        username: "${PROXY_USERNAME}"
        password: "${PROXY_PASSWORD}"
      inject_headers:
        - name: "authorization"
          value: "Bearer ${OPENAI_API_KEY}"
      remove_headers:
        - "authorization"
        - "x-forwarded-for"
        - "forwarded"
        - "cf-connecting-ip"
        - "true-client-ip"
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
  allow_origins: []
  allow_headers: []
  allow_methods: []
  expose_headers: []

rate_limit:
  per_minute: 120

concurrency:
  downstream_max_inflight: 100
  upstream_per_key_max_inflight: 8
```

### 3.2 顶层字段

| Key | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `listen` | `string` | 是 | 无 | 网关监听地址，格式 `host:port`，如 `127.0.0.1:8080`。 |
| `inbound_tls` | `object` | 否 | `null` | 入站 HTTPS 配置；配置后网关使用 HTTPS 监听。 |
| `gateway_auth` | `object` | 是 | 无 | 入站鉴权配置。 |
| `routes` | `array` | 是 | 无 | 路由转发规则，至少 1 条。 |
| `cors` | `object` | 否 | `null` | 浏览器跨域配置（支持 preflight 与常规响应头注入）。 |
| `rate_limit` | `object` | 否 | `null` | 下游限流配置（固定窗口，分钟级）。 |
| `concurrency` | `object` | 否 | `null` | 并发保护配置（下游全局 + 上游按 route + key）。 |

### 3.3 `inbound_tls` 字段（可选）

| Key | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `cert_path` | `string` | 否 | `null` | 证书文件路径（PEM）。 |
| `key_path` | `string` | 否 | `null` | 私钥文件路径（PEM）。 |
| `self_signed_cert_path` | `string` | 否 | `certs/gateway-selfsigned.crt` | 自签名证书文件路径。 |
| `self_signed_key_path` | `string` | 否 | `certs/gateway-selfsigned.key` | 自签名私钥文件路径。 |

规则：
- `cert_path` 与 `key_path` 要么同时配置，要么都不配置。
- 若同时配置，网关直接加载指定证书和私钥。
- 若都不配置，网关优先加载 `self_signed_*` 路径的现有文件；不存在时自动生成自签名证书并落盘。

### 3.4 `gateway_auth` 字段

| Key | 类型 | 必填 | 默认值 | 可选值/限制 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `tokens` | `array<string>` | 是 | 无 | 至少 1 个，不能为空字符串 | 允许访问网关的 token 白名单。 |
| `token_sources` | `array<object>` | 否 | `[{"type":"authorization_bearer"}]` | 顺序生效 | 按顺序尝试提取 token。 |

`token_sources` 子项支持：

1. `{"type": "authorization_bearer"}`  
从 `Authorization: Bearer <token>` 提取。

2. `{"type": "header", "name": "x-gw-token"}`  
从指定 Header 提取（`name` 必填）。

### 3.5 `routes` 字段

每个 `route` 包含：

| Key | 类型 | 必填 | 默认值 | 可选值/限制 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `id` | `string` | 是 | 无 | 全局唯一，非空 | 路由标识。 |
| `prefix` | `string` | 是 | 无 | 必须以 `/` 开头；除 `/` 外不能以 `/` 结尾；全局唯一 | 路由前缀。 |
| `upstream` | `object` | 是 | 无 | - | 上游转发配置。 |

#### 路由匹配规则
- 最长前缀优先（`/openai/v1` 优先于 `/openai`）
- 路径段边界匹配：
  - `/openai` 匹配 `/openai` 和 `/openai/...`
  - `/openai` 不匹配 `/openai2/...`

### 3.6 `upstream` 字段

| Key | 类型 | 必填 | 默认值 | 可选值/限制 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `base_url` | `string` | 是 | 无 | 非空，建议完整 URL（`https://...`） | 上游基地址。 |
| `strip_prefix` | `bool` | 否 | `true` | `true/false` | 是否从请求路径中移除 `prefix` 后再拼接。 |
| `connect_timeout_ms` | `u64` | 否 | `10000` | `> 0` | 建立上游连接超时。 |
| `request_timeout_ms` | `u64` | 否 | `60000` | `> 0` | 请求总预算（详见超时语义）。 |
| `inject_headers` | `array<object>` | 否 | `[]` | Header 名和值需合法 | 注入到上游请求；同名会覆盖。 |
| `remove_headers` | `array<string>` | 否 | `[]` | 大小写不敏感 | 转发前移除的请求头。 |
| `forward_xff` | `bool` | 否 | `false` | `true/false` | 是否保留/传递 `x-forwarded-for` 等来源 IP 头。 |
| `proxy` | `object` | 否 | `null` | 协议为 `http/https/socks` | 按路由配置 gateway 到上游的出站代理。 |
| `upstream_key_max_inflight` | `usize` | 否 | `null` | `> 0` | 覆盖全局上游按 route + key 并发上限（每个 key）。 |

#### `inject_headers` 子项

| Key | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | `string` | 是 | 要注入的 header 名。 |
| `value` | `string` | 是 | 要注入的 header 值。 |

示例：

```yaml
inject_headers:
  - name: "authorization"
    value: "Bearer ${OPENAI_API_KEY}"
```

#### `proxy` 子项（可选）

| Key | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `protocol` | `string` | 是 | 无 | 代理协议：`http` / `https` / `socks`。 |
| `address` | `string` | 是 | 无 | 代理地址，格式 `host:port`。 |
| `username` | `string` | 否 | `null` | 代理认证用户名。 |
| `password` | `string` | 否 | `null` | 代理认证密码。 |

约束：
- `username` 与 `password` 必须同时出现或同时省略。
- 建议通过 `${ENV_VAR}` 注入代理凭据，避免明文。

### 3.7 `cors` 字段

`cors.enabled=true` 时，网关会：
- 处理 `OPTIONS` preflight 请求并返回 CORS 响应头
- 在普通响应（包括错误响应）中注入 `Access-Control-Allow-Origin`

| Key | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `enabled` | `bool` | `false` | 是否启用 CORS。 |
| `allow_origins` | `array<string>` | `[]` | 允许的来源列表，建议填写完整 origin（如 `https://fy.ciallo.fans`）。 |
| `allow_headers` | `array<string>` | `[]` | preflight 允许的请求头。 |
| `allow_methods` | `array<string>` | `[]` | preflight 允许的请求方法。 |
| `expose_headers` | `array<string>` | `[]` | 暴露给浏览器 JS 的响应头。 |

说明：
- `allow_origins` 支持 `*`。
- 为兼容存量配置，`allow_origins` 也接受不带协议的 host（如 `fy.ciallo.fans`），请求时会回写真实 `Origin`。

### 3.8 `rate_limit` 字段（可选）

| Key | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `per_minute` | `u64` | 无 | 每分钟允许的请求数（`> 0`）。 |

行为：
- 作用于下游请求（client -> gateway）。
- 维度为 `token + route`。
- 超限返回 `429 {"error":"rate_limited"}`，并带 `Retry-After`。

### 3.9 `concurrency` 字段（可选）

| Key | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `downstream_max_inflight` | `usize` | `null` | 下游全局并发上限（`> 0`）。 |
| `upstream_per_key_max_inflight` | `usize` | `null` | 上游按 route + key 并发上限（`> 0`）。 |

行为：
- `downstream_max_inflight` 超限返回 `503 {"error":"downstream_concurrency_exceeded"}`。
- `upstream_per_key_max_inflight` 超限返回 `503 {"error":"upstream_concurrency_exceeded"}`。
- 上游 key 只来源于 YAML：`routes[].upstream.inject_headers[].value`（不读取客户端请求头）。
- 识别的 key header 固定为：`authorization`、`x-api-key`（按该顺序匹配）。
- `routes[].upstream.upstream_key_max_inflight` 可覆盖全局上游并发上限。

### 3.10 环境变量插值规则 `${ENV_NAME}`

- 配置文件中出现 `${ENV_NAME}` 会在加载时替换为系统环境变量值。
- 若环境变量不存在，启动失败。
- 建议所有密钥（如 `GW_TOKEN`、上游 API key）都通过环境变量注入。

示例（Linux/macOS）：

```bash
export GW_TOKEN="your_gw_token"
export OPENAI_API_KEY="sk-..."
```

示例（Windows PowerShell）：

```powershell
$env:GW_TOKEN="your_gw_token"
$env:OPENAI_API_KEY="sk-..."
```

提示：
- 某些包含反斜杠的值（如 Windows 路径）建议用单引号包裹，避免 YAML 转义干扰。

## 4. 行为与错误码

常见网关响应：

| HTTP 状态码 | Body | 含义 |
| --- | --- | --- |
| `401` | `{"error":"unauthorized"}` | token 缺失或不在白名单。 |
| `404` | `{"error":"route_not_found"}` | 未命中任何路由。 |
| `429` | `{"error":"rate_limited"}` | 下游请求触发限流。 |
| `503` | `{"error":"downstream_concurrency_exceeded"}` / `{"error":"upstream_concurrency_exceeded"}` | 触发并发保护。 |
| `502` | `{"error":"upstream_connect_error"}` 等 | 上游连接失败或请求失败。 |
| `504` | `{"error":"upstream_timeout"}` | 请求超时。 |

超时语义（当前实现）：
- `connect_timeout_ms`：建立连接阶段超时。
- `request_timeout_ms`：
  - 非 SSE：覆盖上游响应头与响应体阶段（超时会中断流）。
  - SSE：仅约束请求建立/响应头阶段，不对后续事件流施加总超时。

## 5. 编译、测试与质量检查

```bash
cargo fmt --all
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## 6. 使用示例

### 6.1 访问 OpenAI 路由

```bash
curl -X POST "http://127.0.0.1:8080/openai/v1/chat/completions" \
  -H "Authorization: Bearer <GW_TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}'
```

### 6.2 使用自定义 token header

```bash
curl "http://127.0.0.1:8080/openai/v1/models" \
  -H "x-gw-token: <GW_TOKEN>"
```

## 7. 部署指南

### 7.1 Linux + systemd（推荐）

1. 编译并放置二进制：

```bash
cargo build --release
sudo mkdir -p /opt/ai-gw-lite
sudo cp target/release/ai-gw-lite /opt/ai-gw-lite/
sudo cp config/dev.yaml /opt/ai-gw-lite/config.yaml
```

2. 创建服务文件 `/etc/systemd/system/ai-gw-lite.service`：

```ini
[Unit]
Description=AI Gateway Lite
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/ai-gw-lite
Environment=GW_TOKEN=your_gw_token
Environment=OPENAI_API_KEY=sk-xxx
ExecStart=/opt/ai-gw-lite/ai-gw-lite --config /opt/ai-gw-lite/config.yaml
Restart=always
RestartSec=3
User=www-data
Group=www-data

[Install]
WantedBy=multi-user.target
```

3. 启动服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable ai-gw-lite
sudo systemctl start ai-gw-lite
sudo systemctl status ai-gw-lite
```

### 7.2 Windows（基础方式）

```powershell
$env:GW_TOKEN="your_gw_token"
$env:OPENAI_API_KEY="sk-xxx"
.\target\release\ai-gw-lite.exe --config .\config\dev.yaml
```

生产部署建议使用 Windows Service 管理器（如 NSSM）托管该进程，并配置自动重启。

## 8. 安全建议

- 不要把真实密钥写入仓库。
- 优先通过 `${ENV_VAR}` 注入机密。
- 默认保持 `forward_xff: false`。
- 日志中不要输出授权头或密钥内容。

## 9. 已知限制

- 当前未实现配置热加载与自动重试。

---

后续扩展建议优先考虑：配置热加载与重试策略的可控开关。

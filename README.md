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
- 轻量观测页（`/metrics/ui`）与窗口统计接口（`/metrics/summary`）
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

Linux 也支持安装命令（需要 root）：

```bash
sudo ./target/release/ai-gw-lite --install
```

该命令会自动：
- 创建配置目录 `/etc/ai_gw_lite`
- 创建默认配置文件 `/etc/ai_gw_lite/conf.yaml`（若已存在则保留）
- 创建并启用 `/etc/systemd/system/ai-gw-lite.service`

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

### 2.6 可观测性（Metrics + 内置观测页）

若在配置中启用了 `observability.metrics.enabled=true`，可通过独立 token 访问 `/metrics`：

```bash
curl http://127.0.0.1:8080/metrics \
  -H "Authorization: Bearer <GW_METRICS_TOKEN>"
```

Prometheus 抓取示例：

```yaml
scrape_configs:
  - job_name: "ai-gateway"
    metrics_path: /metrics
    static_configs:
      - targets: ["127.0.0.1:8080"]
    authorization:
      type: Bearer
      credentials: "<GW_METRICS_TOKEN>"
```

内置轻量观测页（HTML + JS）：

- 页面：`/metrics/ui`
- 数据接口：`/metrics/summary`（需 `Authorization: Bearer <GW_METRICS_TOKEN>`）
- 页面展示：
  - 每个 route 最近 `1h/24h` 请求数
  - 每个 route 当前并发与最近 `1h/24h` 并发峰值
  - 按 `GW_TOKEN`（脱敏标签）统计最近 `1h/24h` 请求数

直接浏览器打开：

```text
http://127.0.0.1:8080/metrics/ui
```

命令行拉取摘要：

```bash
curl http://127.0.0.1:8080/metrics/summary \
  -H "Authorization: Bearer <GW_METRICS_TOKEN>"
```

## 3. `config.yaml` 详细说明
[配置说明.md](配置说明)

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

1. 编译二进制：

```bash
cargo build --release
```

2. 执行安装命令（自动创建 service 与默认配置）：

```bash
sudo ./target/release/ai-gw-lite --install
```

安装结果：
- service 文件：`/etc/systemd/system/ai-gw-lite.service`
- 配置文件：`/etc/ai_gw_lite/conf.yaml`
- service 启动命令：`--config /etc/ai_gw_lite/conf.yaml`

3. 编辑配置并设置环境变量（示例）：

```bash
sudo vi /etc/ai_gw_lite/conf.yaml
export GW_TOKEN="your_gw_token"
export OPENAI_API_KEY="sk-xxx"
```

4. 启动服务：

```bash
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
- `GW_METRICS_TOKEN` 与业务 `GW_TOKEN` 应分离配置、定期轮换。

## 9. 已知限制

- 当前未实现配置热加载与自动重试。

---

后续扩展建议优先考虑：配置热加载与重试策略的可控开关。

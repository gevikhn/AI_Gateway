# AI Gateway Lite 实施计划（Phase 1 / Phase 2）

## 1. 目标与范围

本计划用于落地 `docs/System Design.md` 的 Phase 1/2 能力，并作为项目执行的唯一任务追踪文档。

Phase 1 范围:
- 多路由前缀匹配（最长前缀优先 + 路径段边界）
- 入站 `GW_TOKEN` 鉴权
- URL 重写与上游地址拼接
- 请求/响应 header 清洗与注入覆盖
- 请求/响应流式透传（含 SSE）
- 基础超时控制（`connect_timeout_ms` / `request_timeout_ms`）

Phase 2 范围（按需启用）:
- 限流（针对下游请求）
- 并发保护（下游与上游，且上游按 key 维度）

不在当前实现范围:
- 配置热加载
- 自动重试

## 2. 当前状态快照

- 日期: 2026-02-11
- 已完成:
  - Rust 工程初始化（`Cargo.toml`、`src/`、`tests/`、`config/dev.yaml`）
  - 配置解析、鉴权、路由与 header 处理基础函数骨架
  - 基础单测与 smoke test
  - axum 服务主干、请求入口链路、真实上游流式转发已接入
  - 基于 mock upstream 的 e2e 测试（header/SSE/timeout）已接入
  - Phase 1 DoD 测试矩阵补齐（含响应侧 hop-by-hop 与连接错误映射）
- 当前缺口:
  - 第二阶段剩余能力：配置热加载、自动重试

## 3. 里程碑与任务分解

状态枚举:
- `TODO`
- `IN_PROGRESS`
- `DONE`

| ID | 任务 | 状态 | 产出文件 | 验收标准 |
| --- | --- | --- | --- | --- |
| M0 | 项目初始化与骨架 | DONE | `Cargo.toml`, `src/*`, `config/dev.yaml`, `tests/smoke.rs` | `cargo test` 通过，基础配置可加载 |
| M1 | 接入 axum 服务与启动流程 | DONE | `src/main.rs`, `src/server.rs` | 进程可监听 `listen`，支持 `--config` 启动 |
| M2 | 请求入口链路（路由匹配 + 鉴权） | DONE | `src/main.rs`, `src/server.rs`, `src/proxy.rs`, `src/auth.rs` | 未命中路由返回 404；鉴权失败返回 401 |
| M3 | 上游转发实现（streaming + SSE） | DONE | `src/server.rs`, `tests/gateway_e2e.rs` | 请求体不聚合；SSE 连续透传 |
| M4 | header 规则完整落地 | DONE | `src/proxy.rs`, `src/server.rs`, `tests/gateway_e2e.rs` | hop-by-hop 双向移除；`inject_headers` 覆盖 |
| M5 | 超时与错误映射 | DONE | `src/server.rs`, `tests/gateway_e2e.rs` | 区分连接超时与请求超时；错误响应稳定 |
| M6 | 测试矩阵补齐（Phase 1 DoD） | DONE | `tests/*`, `src/*` | 覆盖 AGENTS DoD 清单，`cargo test` 全绿 |
| M7 | 文档收口与交付说明 | DONE | `AGENTS.md`, `docs/System Design.md`, `plan.md` | 设计、规范、实施状态一致 |
| M8 | 代码规范约束增强（Rust best practices） | DONE | `AGENTS.md`, `plan.md` | 增加 2000 行限制与 Rust 社区最佳实践约束 |
| M9 | 人类使用手册（README） | DONE | `README.md`, `plan.md` | 提供配置、编译、运行、部署、排障说明 |
| M10 | 上游代理能力（http/https/socks + 认证） | DONE | `src/config.rs`, `src/server.rs`, `tests/*`, `README.md`, `config/dev.yaml`, `docs/System Design.md`, `Cargo.toml`, `plan.md` | 可按路由配置出站代理并通过测试验证 |
| M11 | 入站 HTTPS（证书加载 + 自签名自动生成） | DONE | `src/config.rs`, `src/server.rs`, `src/tls.rs`, `src/lib.rs`, `tests/*`, `README.md`, `config/dev.yaml`, `docs/System Design.md`, `Cargo.toml`, `plan.md` | 支持 TLS 配置，未提供证书时自动生成并复用 |
| M12 | CORS 实际生效（预检 + 响应头） | DONE | `src/server.rs`, `tests/*`, `README.md`, `docs/System Design.md`, `plan.md` | 浏览器跨域请求与 preflight 可按配置通过 |
| M13 | Phase 2：限流与并发控制 | DONE | `src/config.rs`, `src/server.rs`, `src/ratelimit.rs`, `src/concurrency.rs`, `tests/*`, `README.md`, `docs/System Design.md`, `config/dev.yaml`, `plan.md` | 具备下游限流；具备下游并发保护；具备上游按 key 并发保护并通过测试 |
| M14 | Phase 2：上游 key 来源收敛为 YAML 注入值 | DONE | `src/config.rs`, `src/concurrency.rs`, `src/server.rs`, `tests/*`, `README.md`, `docs/System Design.md`, `plan.md` | 上游并发 key 仅从 `inject_headers.value` 提取，且测试覆盖 |
| M15 | Phase 2：移除 `concurrency.upstream_key_headers` 配置项 | DONE | `src/config.rs`, `src/concurrency.rs`, `tests/*`, `README.md`, `docs/System Design.md`, `config/dev.yaml`, `plan.md` | 配置项被移除，行为保持稳定并通过测试 |

## 4. 详细实施步骤（执行顺序）

### Step A: 服务主干接入
- 选择 `axum + reqwest`（server + client）实现最小可用 HTTP 链路。
- 在 `main.rs` 中完成:
  - 配置加载
  - 全局共享状态（`Arc<AppConfig>`）
  - 通配路由挂载与 handler 入口
- 输出:
  - 服务可启动并响应健康请求/错误请求。

### Step B: 路由与鉴权入口
- 在 handler 中执行顺序:
  1. 路由匹配
  2. 入站 token 校验
  3. URL 重写
  4. header 处理
  5. 上游请求构造与发送
  6. 响应回传
- 输出:
  - 404/401 行为与设计文档一致。

### Step C: 真正的流式转发
- 通过 `reqwest`/`axum` body stream 转发请求体与响应体，禁止收集全量字节。
- 确保 `text/event-stream` 不被改写，chunk 持续输出。
- 输出:
  - SSE 场景下客户端持续收到事件。

### Step D: header 与超时规则闭环
- 请求侧:
  - 移除 hop-by-hop headers
  - 移除 `remove_headers`
  - `forward_xff=false` 时移除 IP 暴露头
  - 应用 `inject_headers` 覆盖同名头
- 响应侧:
  - 移除 hop-by-hop headers
- 超时:
  - 连接超时、请求超时分别配置并映射错误码
- 输出:
  - 行为通过单测/集成测试验证。

### Step E: 测试与交付
- 新增集成测试:
  - 路由边界
  - 401/404
  - header 注入覆盖
  - hop-by-hop 双向移除
  - SSE 持续流
  - 超时行为
- 输出:
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - 关键行为样例说明

## 5. 强制更新规则（必须遵守）

每次任务改动必须两次更新本文件:

1. 改动开始前（Before Change）:
- 在“里程碑表”将对应任务状态改为 `IN_PROGRESS`
- 写明本次拟改动文件与目标

2. 改动完成后（After Change）:
- 将任务状态更新为 `DONE` 或回退为 `TODO`
- 记录实际改动文件
- 记录验证命令与结果（如 `cargo test`、`cargo clippy`）
- 记录剩余风险/后续事项

未完成以上更新时，不应视为任务完成。

## 6. 执行记录（Work Log）

> 按时间倒序追加，每条记录必须包含：任务 ID、变更摘要、验证命令、结果。

### 2026-02-11
- 任务: M15
- 变更（After Change）:
  - `src/config.rs` 移除 `ConcurrencyConfig.upstream_key_headers` 字段
  - `src/config.rs` 增加 `ConcurrencyConfig` 的 `deny_unknown_fields`，旧配置项将直接报错（避免静默忽略）
  - 上游 key 识别规则改为内置固定值：`authorization`、`x-api-key`
  - `src/concurrency.rs` 去除对可配置 header 列表的依赖，改为固定识别并仅从 `inject_headers.value` 提取
  - 更新测试：
    - 移除旧字段相关断言
    - 新增“旧字段应报 unknown field”校验用例
  - 同步更新 `README.md`、`docs/System Design.md`、`config/dev.yaml`，删除 `upstream_key_headers` 配置说明与示例
- 实际改动文件:
  - `src/config.rs`
  - `src/concurrency.rs`
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 无新增剩余事项；沿用当前 Phase 2 未实现项（热加载、自动重试）

### 2026-02-11
- 任务: M15
- 变更（Before Change）:
  - 计划移除 `concurrency.upstream_key_headers` 配置项
  - 上游并发 key 识别改为内置固定规则（`authorization`、`x-api-key`）
  - 同步调整配置校验、并发提取逻辑、测试与文档
- 拟改动文件:
  - `src/config.rs`
  - `src/concurrency.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-11
- 任务: M14
- 变更（After Change）:
  - `src/concurrency.rs` 调整上游并发 key 提取逻辑：
    - 不再读取客户端请求头
    - 仅从 `routes[].upstream.inject_headers` 中按 `concurrency.upstream_key_headers` 顺序匹配并提取 key
  - `src/server.rs` 并发控制调用改为仅传入 route（不传请求头）
  - `src/config.rs` 增加约束：
    - 启用上游并发限制（全局或路由级）时，route 必须在 `upstream.inject_headers` 中配置可识别 key header 且 value 非空
  - 更新测试：
    - `src/concurrency.rs` 单测改为基于 YAML 注入值验证按 key 分组
    - `tests/gateway_e2e.rs` 改为两条 route（不同注入 key）验证相同 key 限制、不同 key 并行
    - `src/config.rs` 新增缺少注入 key 的校验失败用例
  - 更新文档与示例注释，明确“上游 key 仅来源于 YAML 注入值”
- 实际改动文件:
  - `src/config.rs`
  - `src/concurrency.rs`
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 无新增剩余事项；沿用当前 Phase 2 未实现项（热加载、自动重试）

### 2026-02-11
- 任务: M14
- 变更（Before Change）:
  - 计划将“上游并发 key 提取来源”收敛为 YAML 配置注入值
  - 禁止从客户端请求头提取上游 key
  - 增加配置校验：启用上游并发限制时，路由必须在 `inject_headers` 提供可识别的 key header
  - 同步更新测试与文档说明
- 拟改动文件:
  - `src/config.rs`
  - `src/concurrency.rs`
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-11
- 任务: M13
- 变更（After Change）:
  - 新增 `src/ratelimit.rs`：固定窗口限流器（按 `token + route + minute`），超限返回 `429 {"error":"rate_limited"}` 且带 `Retry-After`
  - 新增 `src/concurrency.rs`：并发控制器
    - 下游全局并发上限（`downstream_max_inflight`）
    - 上游按 key 并发上限（`upstream_per_key_max_inflight`，可被 `routes[].upstream.upstream_key_max_inflight` 覆盖）
    - key 提取来源可配置（`upstream_key_headers`）
  - `src/server.rs` 接入限流与并发控制链路，并确保 permit 生命周期覆盖响应流（含 SSE）
  - `src/config.rs` 扩展并校验新配置：`rate_limit`、`concurrency`、`upstream_key_max_inflight`
  - `tests/gateway_e2e.rs` 补充 Phase 2 集成测试：
    - 下游限流返回 429
    - 下游并发超限返回 503
    - 上游相同 key 并发超限返回 503，不同 key 可并行
  - 同步更新 `README.md`、`docs/System Design.md`、`config/dev.yaml` 的配置与行为说明
- 实际改动文件:
  - `src/auth.rs`
  - `src/config.rs`
  - `src/concurrency.rs`
  - `src/lib.rs`
  - `src/proxy.rs`
  - `src/ratelimit.rs`
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `tests/inbound_tls_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - Phase 2 尚未实现配置热加载与自动重试

### 2026-02-11
- 任务: M13
- 变更（Before Change）:
  - 计划进入 Phase 2，落地限流与并发控制能力
  - 计划实现下游固定窗口限流（每分钟），超限返回 `429 {"error":"rate_limited"}`
  - 计划实现并发保护：
    - 下游并发上限（全局）
    - 上游并发上限（按 route + upstream key 维度）
  - 计划补充配置解析/校验、运行时组件与集成测试，并更新文档与示例配置
- 拟改动文件:
  - `src/config.rs`
  - `src/server.rs`
  - `src/lib.rs`
  - `src/ratelimit.rs`
  - `src/concurrency.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-11
- 任务: M12
- 变更（After Change）:
  - 在 `src/server.rs` 落地 CORS 运行时逻辑：
    - 识别并处理 preflight (`OPTIONS + Origin + Access-Control-Request-Method`)
    - preflight 成功返回 `204` 并设置 `Access-Control-Allow-Origin/Methods/Headers`
    - 常规响应（含错误响应）按命中 origin 注入 `Access-Control-Allow-Origin`
    - 支持 `Access-Control-Expose-Headers`
  - 对 `allow_origins` 增加兼容匹配：支持完整 origin 与无协议 host 写法（如 `fy.ciallo.fans`）
  - 新增 e2e 测试：
    - preflight 无需鉴权即可通过并返回正确 CORS 头
    - `allow_origins` 配置 host 写法时，响应可正确回写 `Origin`
  - 更新 `README.md` 与 `docs/System Design.md`，将 CORS 从“仅解析”更新为“已生效”
- 实际改动文件:
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 当前 CORS 未实现 `allow_credentials` 与 `max_age`，如前端后续需要 Cookie 凭据可新增显式配置项

### 2026-02-11
- 任务: M12
- 变更（Before Change）:
  - 计划将现有仅“解析不生效”的 `cors` 配置改为运行时生效
  - 计划实现 preflight (`OPTIONS`) 处理，并在常规响应上注入 CORS 相关响应头
  - 计划补充集成测试覆盖跨域成功路径与预检路径，并同步更新文档说明
- 拟改动文件:
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-11
- 任务: M11
- 变更（After Change）:
  - 在顶层配置新增 `inbound_tls`，支持 `cert_path`/`key_path` 与 `self_signed_*` 路径配置
  - 增加 TLS 配置校验：证书与私钥必须成对出现；路径不得为空
  - 新增 `src/tls.rs`，实现证书路径解析逻辑：
    - 若配置 cert/key：直接加载
    - 若未配置 cert/key 且已存在自签名文件：直接加载
    - 若未配置 cert/key 且文件不存在：自动生成并落盘
  - `src/server.rs` 接入 HTTPS 启动路径（`axum-server + rustls`），并在 TLS 启动前安装 rustls crypto provider
  - 新增入站 HTTPS e2e 测试，验证自动生成自签名证书并可通过 HTTPS 访问
  - 同步更新 `README.md`、`docs/System Design.md`、`config/dev.yaml` 配置示例与说明
  - 调整 `tests/gateway_e2e.rs` 两个超时用例预算，降低时间敏感抖动
- 实际改动文件:
  - `Cargo.toml`
  - `src/config.rs`
  - `src/server.rs`
  - `src/tls.rs`
  - `src/lib.rs`
  - `tests/inbound_tls_e2e.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 当前 HTTPS 为“开启后仅 HTTPS 监听”，如需 HTTP+HTTPS 同时监听可在后续任务扩展双监听模式

### 2026-02-11
- 任务: M11
- 变更（Before Change）:
  - 计划为 `user -> gateway` 入站链路增加 HTTPS 监听能力（保持 HTTP 默认兼容）
  - 计划支持配置证书与私钥路径；若未配置则自动生成并复用本地自签名证书
  - 计划补充 TLS 配置校验、证书生成/加载逻辑及测试，并同步文档与示例配置
- 拟改动文件:
  - `Cargo.toml`
  - `src/config.rs`
  - `src/server.rs`
  - `src/tls.rs`
  - `src/lib.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-11
- 任务: M10
- 变更（After Change）:
  - 在 `UpstreamConfig` 增加可选 `proxy` 配置，支持 `protocol=http|https|socks`、`address`、`username/password`
  - 新增配置校验：代理地址不可为空，用户名和密码必须成对出现且不能为空
  - 在按路由构建 `reqwest::Client` 时接入代理配置，`socks` 映射为 `socks5h`，支持代理认证
  - 为 `reqwest` 启用 `socks` feature
  - 新增测试：代理配置解析/校验、代理 URL 构建、HTTP 代理 + Basic 认证链路 e2e
  - 同步更新 `README.md`、`docs/System Design.md`、`config/dev.yaml` 的代理配置说明
- 实际改动文件:
  - `Cargo.toml`
  - `src/config.rs`
  - `src/server.rs`
  - `src/proxy.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `docs/System Design.md`
  - `config/dev.yaml`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 当前仅对 HTTP 代理链路做了端到端验证；`https/socks` 通过配置与 client 构建单测覆盖，后续可补充真实代理服务的 e2e

### 2026-02-11
- 任务: M10
- 变更（Before Change）:
  - 计划为 gateway -> upstream 链路增加可配置代理能力，支持 `http`/`https`/`socks` 三类代理协议
  - 计划支持代理用户名/密码配置，并接入到按路由构建的 `reqwest::Client`
  - 计划补充配置校验与测试（含代理鉴权透传验证）并同步更新文档示例
- 拟改动文件:
  - `src/config.rs`
  - `src/server.rs`
  - `src/proxy.rs`
  - `tests/gateway_e2e.rs`
  - `README.md`
  - `config/dev.yaml`
  - `Cargo.toml`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M9
- 变更（After Change）:
  - 新增 `README.md`，面向“人类”提供完整使用手册
  - 补充 `config.yaml` 全字段说明（类型、默认值、限制、作用、示例）
  - 补充编译、运行、部署（Linux systemd / Windows）与常见错误码、排障建议
  - 文档内容已按当前代码行为对齐（含 SSE 与 timeout 语义、CORS 当前状态）
- 实际改动文件:
  - `README.md`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 若未来新增 Phase 2 能力，请同步扩展 README 与配置章节

### 2026-02-10
- 任务: M9
- 变更（Before Change）:
  - 计划新增面向“人类”的 `README.md` 使用手册
  - 重点覆盖：`config.yaml` 全字段说明、编译运行、部署方式、排障与安全建议
  - 内容需与当前代码行为一致（包含默认值、限制、已实现与未实现项）
- 拟改动文件:
  - `README.md`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M5
- 变更（After Change）:
  - `src/server.rs` 引入按路由预构建并复用的上游 `reqwest::Client`，避免每请求重建客户端
  - `src/server.rs` 将 `request_timeout_ms` 从“仅 send 阶段”扩展到非 SSE 响应体读取阶段（SSE 保持不施加总超时）
  - `tests/gateway_e2e.rs` 新增“非 SSE 响应体卡顿时在超时预算内终止”用例
  - `tests/gateway_e2e.rs` 新增“小 request timeout 下 SSE 仍持续透传”回归用例
  - 同步调整 `build_app` 构建路径与相关测试调用
- 实际改动文件:
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 当前非 SSE 响应体若在 headers 后超时会以流错误中断（而非统一 504），后续可按产品语义继续细化

### 2026-02-10
- 任务: M5
- 变更（Before Change）:
  - 计划修复 `request_timeout_ms` 仅覆盖上游 headers 阶段的问题，补齐非 SSE 响应体阶段超时约束
  - 计划将上游 `reqwest::Client` 改为按路由复用，避免每请求重建客户端
  - 计划补充对应测试，验证非 SSE 卡顿体超时行为
- 拟改动文件:
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
  - `plan.md`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M7
- 变更（After Change）:
  - 更新 `AGENTS.md` 项目结构，补充 `src/server.rs` 并修正 `proxy.rs` 职责描述
  - 更新 `docs/System Design.md` 模块划分，新增“当前实现状态（2026-02-10）”章节
  - 更新 `plan.md` 里程碑状态与步骤文案，使实现路径与当前技术选型一致
- 实际改动文件:
  - `AGENTS.md`
  - `docs/System Design.md`
  - `plan.md`
- 验证:
  - `rg -n "server.rs|axum|reqwest|M7|DONE" AGENTS.md docs/System Design.md plan.md`
- 结果: DONE
- 剩余事项:
  - 如进入第二阶段，新增 M9+ 里程碑并按同样规则维护执行记录

### 2026-02-10
- 任务: M7
- 变更（Before Change）:
  - 计划完成文档收口：同步 `AGENTS.md` 与 `docs/System Design.md` 到当前代码实现
  - 重点修正模块结构描述（补充 `src/server.rs`）与实现状态说明
  - 完成后将 M7 状态标记为 `DONE`
- 拟改动文件:
  - `AGENTS.md`
  - `docs/System Design.md`
  - `plan.md`
- 验证:
  - `rg -n "server.rs|axum|reqwest|M7|DONE" AGENTS.md docs/System Design.md plan.md`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M6
- 变更（After Change）:
  - 在 `src/proxy.rs` 新增响应侧 hop-by-hop 头清洗单测
  - 在 `tests/gateway_e2e.rs` 新增上游连接失败映射到 502 的 e2e 用例
  - 保留既有 e2e 用例（header 注入/SSE/timeout），完成 DoD 矩阵收口
- 实际改动文件:
  - `src/proxy.rs`
  - `tests/gateway_e2e.rs`
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: DONE
- 剩余事项:
  - 继续推进 M7：整理阶段交付总结与文档一致性收口

### 2026-02-10
- 任务: M6
- 变更（Before Change）:
  - 计划补充 DoD 缺失测试：响应侧 hop-by-hop headers 移除验证
  - 计划补充上游连接失败（connect error）错误映射测试
  - 计划根据测试结果微调代理实现并完成 M6 收口
- 拟改动文件:
  - `tests/gateway_e2e.rs`
  - `src/server.rs`（如需）
  - `plan.md`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M1/M2/M3/M4/M5
- 变更（After Change）:
  - 新增 `src/server.rs`，实现 axum 服务主干、healthz、fallback 代理入口
  - `src/main.rs` 切换为异步启动流程，按 `--config` 读取配置并启动服务
  - 接入真实上游请求转发（reqwest），请求/响应 body 流式透传
  - 落地 404/401 错误响应、header 清洗/注入、超时与上游错误映射
  - 新增 `tests/gateway_e2e.rs`，覆盖注入头、SSE 透传与超时映射
  - 更新 `Cargo.toml` 依赖与 `src/lib.rs` 模块导出
- 实际改动文件:
  - `Cargo.toml`
  - `src/main.rs`
  - `src/lib.rs`
  - `src/server.rs`
  - `tests/gateway_e2e.rs`
- 验证:
  - `cargo fmt --all`
  - `cargo test`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo run -- --help`
- 结果: DONE
- 剩余事项:
  - M6 继续补齐 DoD 边界用例（尤其响应侧 hop-by-hop 头验证）

### 2026-02-10
- 任务: M1/M2/M3/M4/M5
- 变更（Before Change）:
  - 计划接入 axum 服务主干与 `--config` 启动运行链路
  - 计划实现请求入口（路由匹配、鉴权、404/401 响应）
  - 计划接入真实上游流式转发（含 SSE）与 header 规则
  - 计划完成超时与上游错误映射
- 拟改动文件:
  - `Cargo.toml`
  - `src/main.rs`
  - `src/lib.rs`
  - `src/proxy.rs`
  - `tests/*`
- 验证:
  - 完成后执行 `cargo fmt --all`、`cargo test`、`cargo clippy --all-targets --all-features -- -D warnings`
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M8
- 变更（After Change）:
  - 已在 `AGENTS.md` 添加“单文件不超过 2000 行”硬约束
  - 已新增 Rust 社区最佳实践约束（格式化/lint、错误处理、unsafe、异步阻塞、测试、模块职责）
  - 已在 PR checklist 增加“2000 行限制”检查项
- 验证:
  - `rg -n "2000|Rust 最佳实践约束|Community Baseline|PR 必填检查项" AGENTS.md plan.md`
- 结果: DONE
- 剩余事项:
  - 后续如引入 CI，可将“单文件行数 <= 2000”自动化为检查脚本

### 2026-02-10
- 任务: M8
- 变更（Before Change）:
  - 计划更新 `AGENTS.md`，新增“单文件不超过 2000 行”强约束
  - 计划补充 Rust 社区最佳实践代码约束（格式化、lint、错误处理、unsafe、测试等）
  - 关联更新 `plan.md` 状态与执行记录
- 验证:
  - N/A（文档改动准备阶段）
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M7
- 变更:
  - 新增 `plan.md` 作为实施执行计划与记录文档
  - 更新 `AGENTS.md`，增加“改动前后必须更新 `plan.md`”硬约束
  - 在 PR checklist 中加入 `plan.md` 更新检查项
- 验证:
  - `AGENTS.md` 与 `plan.md` 内容一致，规则已落地
- 结果: IN_PROGRESS

### 2026-02-10
- 任务: M0
- 变更:
  - 初始化 Rust 项目结构并建立模块骨架
  - 新增基础配置文件与 smoke test
- 验证:
  - `cargo test` 通过
  - `cargo clippy --all-targets --all-features -- -D warnings` 通过
  - `cargo run -- --config config/dev.yaml` 可启动
- 结果: DONE

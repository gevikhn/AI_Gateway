# AI Gateway Lite 实施计划（Phase 1）

## 1. 目标与范围

本计划用于落地 `docs/System Design.md` 的 Phase 1 能力（基础路由转发），并作为项目执行的唯一任务追踪文档。

Phase 1 范围:
- 多路由前缀匹配（最长前缀优先 + 路径段边界）
- 入站 `GW_TOKEN` 鉴权
- URL 重写与上游地址拼接
- 请求/响应 header 清洗与注入覆盖
- 请求/响应流式透传（含 SSE）
- 基础超时控制（`connect_timeout_ms` / `request_timeout_ms`）

不在本阶段:
- 限流
- 并发保护
- 配置热加载
- 自动重试

## 2. 当前状态快照

- 日期: 2026-02-10
- 已完成:
  - Rust 工程初始化（`Cargo.toml`、`src/`、`tests/`、`config/dev.yaml`）
  - 配置解析、鉴权、路由与 header 处理基础函数骨架
  - 基础单测与 smoke test
  - axum 服务主干、请求入口链路、真实上游流式转发已接入
  - 基于 mock upstream 的 e2e 测试（header/SSE/timeout）已接入
  - Phase 1 DoD 测试矩阵补齐（含响应侧 hop-by-hop 与连接错误映射）
- 当前缺口:
  - 第一阶段核心目标已闭环；后续可按需求进入第二阶段能力开发

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

# Repository & Agent Guidelines / 仓库与代理协作规范

本仓库当前为 docs-first。`docs/System Design.md` 是设计事实源（source of truth）, 但仅在当前任务相关时按章节读取（MUST read only relevant sections, never full-file by default）；若本文件与其冲突，以 `docs/System Design.md` 为准。`plan.md` 是实施执行事实源（execution source of truth），用于追踪任务状态与验证结果。

## Current Delivery Scope / 当前交付范围

Phase 1（当前必须实现）:
- 多 route 前缀转发（multi-route prefix forwarding）
- 入站 `GW_TOKEN` 鉴权（inbound token auth）
- `inject_headers` 注入/覆盖与 `remove_headers` 移除（header injection/stripping）
- 请求/响应流式透传，包含 SSE（streaming pass-through with SSE）
- 基础超时控制：`connect_timeout_ms` 与 `request_timeout_ms`

Phase 2（仅 roadmap，暂不实现）:
- 限流（rate limiting）
- 并发保护（concurrency guard）
- 配置热加载（config hot reload）
- 自动重试（automatic retry）

硬约束:
- 未被任务显式要求时，不实现 Phase 2 能力（do not implement Phase 2 unless explicitly requested）。
- 在进行任何项目改动前，必须先更新 `plan.md`（将对应任务标记为 `IN_PROGRESS` 并记录目标）。
- 在完成任何项目改动后，必须再次更新 `plan.md`（状态、验证命令、结果、剩余事项）。
- 任意单个源码文件不得超过 2000 行（hard cap）；超过或接近上限时必须拆分模块。

## Project Structure / 项目结构

当前仓库以设计文档为主（docs-first）。当实现代码落地时，遵循以下结构：
- `src/main.rs`: 启动、状态装配、路由挂载
- `src/server.rs`: HTTP 入口、请求处理流程、上游转发与错误映射
- `src/config.rs`: YAML 解析、`${ENV_VAR}` 插值、配置校验
- `src/auth.rs`: `GW_TOKEN` 提取与校验
- `src/proxy.rs`: 路由匹配、URL 重写、header 处理辅助函数
- `tests/`: 跨模块集成测试（routing/auth/streaming/timeout）
- `src/ratelimit.rs`（Phase 2 optional）
- `src/reload.rs`（Phase 2 optional）
- `plan.md`: 实施计划、任务状态、执行记录（must-update during change lifecycle）

要求:
- 目录职责单一，避免将运行时代码与设计说明混放。

## Build, Test, Dev Commands / 构建与开发命令

在仓库根目录执行（run from repo root）:
- `cargo build`
- `cargo run -- --config <path-to-config.yaml>`
- `cargo test`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

前置条件:
- 若尚无 `Cargo.toml`，先初始化 Rust 工程（例如 `cargo init --bin`）。

## Coding Rules (Behavioral) / 编码规则（行为级约束）

路由与 URL:
- 路由匹配必须采用“最长前缀优先 + 路径段边界”（longest-prefix + segment boundary）。
- `/openai` 只能匹配 `/openai` 与 `/openai/...`，不得匹配 `/openai2`。
- `strip_prefix=true` 后，空路径必须重写为 `/`。
- `base_url` 与 `rest_path` 拼接不得出现 `//`。
- 未命中路由返回 `404 {"error":"route_not_found"}`。

鉴权:
- token 缺失或不匹配必须返回 `401 {"error":"unauthorized"}`。
- 仅按配置的 `token_sources` 顺序提取 token。

Header 处理:
- 请求与响应都必须移除 hop-by-hop headers（大小写不敏感）:
  - `connection`, `keep-alive`, `proxy-authenticate`, `proxy-authorization`
  - `te`, `trailer`, `transfer-encoding`, `upgrade`
- 请求转发前移除 `remove_headers`，并在需要时清理 `x-forwarded-for`/`forwarded`。
- 应用 `inject_headers` 时覆盖同名 header（overwrite existing header）。

流式与超时:
- 禁止把请求体或响应体整体 `collect` 到内存。
- SSE 不改写、不聚合 chunk（no rewrite/aggregation）。
- 区分 `connect_timeout_ms` 与 `request_timeout_ms`。
- 长流（例如 SSE）不得被总超时误切断。

安全与日志:
- 禁止记录入站 `Authorization`、`x-api-key` 或任何注入后的密钥头。
- 禁止在错误消息中返回密钥信息。

Rust 最佳实践约束（Community Baseline）:
- 必须通过 `cargo fmt --all` 与 `cargo clippy --all-targets --all-features -- -D warnings`。
- 生产路径禁止无理由使用 `unwrap`/`expect`/`panic!`；可恢复错误必须返回 `Result`。
- 优先使用明确错误类型（typed errors），避免仅依赖字符串错误。
- 异步上下文禁止阻塞调用；必须使用异步 API，必要时使用 `spawn_blocking` 隔离。
- `unsafe` 默认为禁止；确需使用时必须附带 `SAFETY` 注释、最小作用域与测试覆盖。
- 避免不必要 `clone`；优先借用（borrow）与迭代器，降低分配与拷贝。
- 新增或变更的公共行为必须补充测试（单测或集成测试）。
- 文件与模块保持单一职责（single responsibility），避免“大文件 + 大而全模块”。

## Testing Guidelines / 测试与 DoD（Phase 1）

PR 合并前至少覆盖以下场景（minimum DoD）:
1. 路由正确性：`/openai/v1/models` 正确转发到上游路径。
2. 路由边界：`/openai2/...` 不得命中 `prefix=/openai`。
3. strip_prefix 边界：`/openai` 转发后路径为 `/`。
4. 鉴权失败：无 token 或错误 token 返回 401。
5. Header 注入覆盖：客户端 `Authorization` 被上游注入值覆盖。
6. 用户 IP 不透传：上游看不到 `X-Forwarded-For/Forwarded/CF-Connecting-IP`。
7. hop-by-hop 头双向移除：请求与响应两侧都不携带。
8. SSE 流式透传：事件可持续回传且不中断。
9. 超时行为：连接超时可识别；长流不被 `request_timeout_ms` 误杀。

## Commit & PR Checklist / 提交与合并检查

提交信息建议使用 Conventional Commits（recommended）:
- `feat(proxy): add route boundary match`
- `fix(auth): normalize bearer token parsing`

PR 必填检查项:
- [ ] 标注本次范围：Phase 1 还是 Phase 2
- [ ] 说明与 `docs/System Design.md` 的对齐点
- [ ] 改动前后均已更新 `plan.md`（任务状态 + 验证记录）
- [ ] 无源码文件超过 2000 行，必要时已完成模块拆分
- [ ] 提供测试证据（`cargo test` 输出或等效证据）
- [ ] 提供关键代理行为样例（request/response 或 curl）

## Security & Config Tips / 安全与配置建议

- 禁止提交真实 `GW_TOKEN` 或上游 API key。
- 配置优先使用 `${ENV_VAR}` 引用机密。
- 默认保持 `forward_xff: false`，除非有明确需求。
- 任何日志与报错都不得泄露认证头或密钥值。

## Terminology / 术语统一

- `GW_TOKEN`: 客户端到网关的入站令牌（client-to-gateway token）
- `inject_headers`: 网关注入到上游的 headers
- `remove_headers`: 转发前需剔除的 headers
- `hop-by-hop headers`: 仅单跳有效、不可端到端转发的协议头

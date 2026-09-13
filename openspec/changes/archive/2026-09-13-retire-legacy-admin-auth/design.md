# Design: retire-legacy-admin-auth

## Context

`/api/admin/*` 现有两层防护：外层 `auth_guard`（server.rs，多用户 RBAC，`services.control`）+ 内层旧 env 密码层（admin.rs）。内层构件：`AdminState.password`（读 `SEBAS_WEBUI_PASSWORD`）、`AdminState.session_store`（独立 `SessionStore`）、`sebas_admin_session` cookie、`api_admin_auth_guard`、`/api/admin/{login,csrf,logout}`、`admin_mutation_guard` 的密码/CSRF 分支（`X-CSRF-Token` + `CsrfExtension`）、前端 `client.ts` 的 CSRF token 管道与三个调用方法（无任何视图调用它们）。另见 proposal.md Why。

关键事实：
- `SessionStore` 在 `admin_auth.rs`，**被 webui 会话复用**（`AuthHandle.session_store`）——删除的是 admin 层的*第二个实例*，不是类型本身。
- `SEBAS_CONTROL_SECRET` 是 watchdog 控制 RPC 秘钥（`control_admin_adapter` 用它连控制面），与本层的登录密码无关；不动。
- `SEBAS_WEBUI_PASSWORD` 在 `bootstrap_auth`（首启 root 引导）与 `/api/env` 策展清单中仍被使用——env 本身保留，退役的只是把它当控制面登录密码的用法。

## Goals / Non-Goals

- Goals：`/api/admin/*` 认证与授权单层化（RBAC 唯一执法）；删除死代码面（三端点、第二会话库、CSRF 管道）；规格与实现对齐。
- Non-Goals：不动 `/router/api/*` RBAC 豁免、`bootstrap_auth`、`webui-passwd`、`SEBAS_CONTROL_SECRET` 通道；不改 `SessionStore` 类型归属（仅文档）。

## Decisions

- **D1 删除而非保留 no-op 层**：未配 env 密码时内层本就全放行，配了反而制造双重登录与密码漂移两个缺陷；规格措辞与实现早已脱节。Alternative：把内层改接 `SEBAS_CONTROL_SECRET`——rejected，等于给 RBAC 再加一道与角色无关的共享秘密门，违背多用户模型。
- **D2 `admin_mutation_guard` 收敛为 POST-only + origin-only**：删密码/CSRF 分支后行为 = 现行「无密码模式」分支（无 Origin 或空 Origin 或 loopback Origin 放行，非 loopback Origin 403）。CSRF 防线由 SameSite=Lax cookie + `auth_guard` 的 Origin == Host 校验承担——与工作台其余写面同一套，不再维护两套 CSRF 机制。Alternative：保留 `X-CSRF-Token`——rejected，纯遗产，前端无调用者。
- **D3 `AdminState` 瘦身为 `{ adapter, started_at }`**：`session_store` 与 `password` 字段随内层退役；`AdminState::new(adapter)` 签名不变（调用点零改动）。`SessionStore` 实例仍由 `AuthHandle` 持有。`admin_auth.rs` 模块文档改写为「webui 会话 + 登录限速的共享存储」，不改文件名（纯改名噪音另立批次）。
- **D4 401/403 语义统一**：`/api/admin/*` 未认证 → `auth_guard` 的 JSON 401（`WWW-Authenticate: sebas-session`）；角色不足 → 403（RBAC 表）；跨源写 → 403。第二套 401 来源消失。
- **D5 前端只清 client.ts**：三个方法与 CSRF 管道无视图调用者；`isAuthExempt` 名单里 `/api/admin/login|logout` 两条随端点删除。

## Risks / Trade-offs

- [有部署把 `SEBAS_WEBUI_PASSWORD` 当控制面密码] → proposal BREAKING 标注 + Migration 说明；`bootstrap_auth` 语义不变，告警日志（短密码 warn）不动。
- [`auth = false` 沙箱下控制面 mutation 完全开放] → 与现状一致（无密码模式本就如此），非回归；AGENTS.md 沙箱菜谱不受影响。
- [外部脚本依赖 `/api/admin/login`] → 未正式发布产品的运维面，Migration 指向 `/api/auth/login`。

## Migration Plan

单批退役，回滚 = revert 提交。无数据迁移（内存会话，重启即失效；auth.db 不涉及）。

## Open Questions

无。

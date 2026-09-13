## Why

`/api/admin/*` 现在叠着两代鉴权：外层是多用户 RBAC（auth_guard 按 `services.control` 执法），内层还挂着旧单账户时代的 env 密码层（`AdminState.password` 读 `SEBAS_WEBUI_PASSWORD`、独立 `sebas_admin_session` cookie、`/api/admin/login|csrf|logout` 三端点、`X-CSRF-Token` 流程）。该层职能已被 RBAC 完全覆盖——未配 env 密码时整层空转放行；配置了反而制造两个已知别扭点：env 密码一鱼两吃（root 经用户管理改密后控制面仍认旧值）、已认证的 root/admin 需要第二把 cookie 才能动控制面。主规格也已与实现漂移：「Optional admin authentication」requirement 写的门禁变量是 `SEBAS_CONTROL_SECRET`，代码实际读的是 `SEBAS_WEBUI_PASSWORD`（后者仅是首启引导凭据）。多用户 RBAC 已落地，旧的该退役。

## What Changes

- **BREAKING** 移除 admin 控制面 env 密码登录：删除 `POST /api/admin/login`、`GET /api/admin/csrf`、`POST /api/admin/logout` 端点；`sebas_admin_session` cookie、`AdminState` 内置 SessionStore 与 `X-CSRF-Token` 校验一并退役
- `/api/admin/*` 授权唯一由 webui auth_guard 执法（有效会话 + `services.control` 角色）；非安全方法继续受同源（Origin == Host）校验
- `SEBAS_WEBUI_PASSWORD` 保留唯一职责：首启 root 引导（`bootstrap_auth`，行为不变）
- `SessionStore`（admin_auth.rs）保留——webui 会话与登录限速仍复用，仅更正其描述旧模型的模块文档
- 前端 api client 移除 `adminLogin` / `adminCsrf` / `adminLogout` 与 CSRF token（内存 + sessionStorage）管道
- 主规格 `webui`：「Optional admin authentication」requirement 退役，控制面门禁语义收敛进 RBAC 表述；「Mutation posture」去掉"配密码时要求 CSRF token"条款，保留 POST-only + origin 校验与诚实降级

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: 「Optional admin authentication」requirement 移除——`/api/admin/*` 的认证与授权改由「鉴权开关（auth）与首启用户引导」+ RBAC（`services.control`）唯一执法；无控制面 adapter 时的诚实降级（reads `adapter_ok: false`、mutations 503）原样保留；「Mutation posture」requirement 收敛

## Impact

- `sebas-webui/src/admin.rs`（guard、登录三端点、CSRF 守卫与相关测试）、`admin_auth.rs`（模块文档）、`server.rs`（如引用）、`sebas-webui/frontend/src/api/client.ts` 及其测试
- `openspec/specs/webui/spec.md`（经 delta，归档时同步）
- 兼容性：未配 `SEBAS_WEBUI_PASSWORD` 的部署行为零变化；配置了的部署从「双重登录」变为 RBAC 单层执法（root/admin 直达控制面）

## Non-goals

- 不动 `SEBAS_CONTROL_SECRET`（watchdog 控制 RPC 秘钥，独立通道，webui 借它连控制面 adapter 的语义不变）
- 不动 `/router/api/*` 的 RBAC 豁免（add-webui-multiuser-rbac design D3 既定决策）
- 不改 `bootstrap_auth` 引导路径与 `webui-passwd` CLI
- 不为旧 admin 会话 cookie 提供任何迁移或兼容形态

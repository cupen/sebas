# Design: add-webui-multiuser-rbac

## Context

现状（见 proposal.md - Why）：`sebas-webui/src/auth.rs` 是单账户 JSON 凭据
（`~/.sebas/webui-auth.json`，mtime 热重载）+ 无角色。开关与安全门已经
是目标形态（`[watchdog.webui] auth` 默认 true、服务端生效、非 loopback
联动），本次不动它们的语义，只换底层账户模型并加 RBAC。两条启动路径
（`sebas webui` 独立进程 `webui_cmd.rs`、`core --webui` 内嵌 `run.rs`）
都经 `webui_cmd::bootstrap_auth()` 装配，改动集中。

## Goals / Non-Goals

Goals：多用户 SQLite 存储、零用户首启设置页、RBAC 四角色执法、root
用户管理 UI、CLI/env 引导改写 auth.db。

Non-Goals（proposal 已列）：数据隔离、自定义角色、MFA/审计、动
`/api/admin/*` 自身的 control-secret 鉴权。另加设计边界：旧单账户系统
不做任何兼容——`webui-auth.json` / `SEBAS_WEBUI_AUTH_FILE` /
`SEBAS_WEBUI_TOKEN` / `{"secret"}` 单字段登录全部移除（未正式发布，
无存量用户）。

## Decisions

### D1: 独立 auth.db + rusqlite 同步连接 + Mutex

- 路径：默认 `~/.sebas/auth.db`，`SEBAS_WEBUI_AUTH_DB` 覆盖（沙箱隔离
  必备，与 `SEBAS_STATE_DB` 同一模式）。
- 复用 `sebas_state/db.rs` 的配方：rusqlite（workspace 已有 0.40 bundled，
  `sebas-webui/Cargo.toml` 新增依赖）、WAL、`busy_timeout=5s`、
  `foreign_keys=ON`、`user_version` 迁移。
- 单 `Connection` 包在 `std::sync::Mutex` 里，同步调用。鉴权流量是每请求
  一次主键查询（µs 级、进程内 SQLite），不值得 spawn_blocking 池或引入
  sqlx。备选：`SEBAS_STATE_DB` 里加表——被否，用户要求独立文件，且避免
  与状态库写放大互相干扰。

Schema（`user_version=1`）：

```sql
CREATE TABLE users (
  id          INTEGER PRIMARY KEY,
  username    TEXT NOT NULL UNIQUE COLLATE NOCASE,
  role        TEXT NOT NULL CHECK (role IN ('root','admin','member','viewer')),
  iterations  INTEGER NOT NULL,
  salt_hex    TEXT NOT NULL,
  hash_hex    TEXT NOT NULL,
  enabled     INTEGER NOT NULL DEFAULT 1,
  created_at_unix INTEGER NOT NULL,
  updated_at_unix INTEGER NOT NULL
);
```

角色是代码里的 enum，不建 roles/permissions 表（映射固定，见 D3）。
大小写不敏感唯一靠 `COLLATE NOCASE`。密码 PBKDF2-HMAC-SHA256 120k 次
迭代、随机盐，实现直接沿用现 `auth.rs` 的函数（已过 RFC 向量测试）。

### D2: 模块布局——`auth.rs` 演进 + 新 `user_store.rs` / `rbac.rs`

- `sebas-webui/src/user_store.rs`（新）：SQLite CRUD（`list/create/
  set_password/set_role/set_enabled/delete/get_by_username/get`），纯同步
  API + 错误枚举（`UsernameTaken`、`LastRoot`、`NotFound`…），单测友好。
- `sebas-webui/src/rbac.rs`（新）：`Role`、`Permission` enum 与
  `Role::permissions()` 映射（spec 表的代码化）。
- `sebas-webui/src/auth.rs`（重写）：`AuthHandle` 保留名字与大致外形
  （`enabled()/login()/logout()`），内部从「单凭据 + mtime 热重载」换成
  「`UserStore` + `SessionStore`」。热重载机制删除（DB 即活数据）。
  新增 `setup_root()`（零用户建 root，事务内二次校验零用户）；
  `login_secret()`/token 相关代码删除。
- `SessionStore`（admin_auth.rs）扩展：`Session` 加 `user_id`；新增
  `remove_all_for_user(user_id)`（禁用/删号/重置密码时踢会话）。
  admin 自己的会话 `user_id = 0`（不受用户管理影响）。

### D3: 权限执法——中央路径表，不做 per-handler extractor

沿用 `is_protected_path` 的既有风格：`server.rs` 里一张
`required_permission(path, method) -> Option<Permission>` 声明表，
`auth_guard` 认证通过后查表执法，不足即 403。备选：axum extension
extractor 挂每个 handler——更类型安全但要动几十个 handler 签名，且与
现有中间件风格割裂，否。`auth_guard` 同时把解析出的
`Identity { user_id, username, role }` 塞进 request extensions，
handler（如 user 管理）按需读取。

初始映射（实现时逐路由审计微调，写进同一张表）：

| 路由面 | 权限 |
|---|---|
| `/api/users*`（新） | `users.manage` |
| `/api/admin/*`、服务启停/升级/回滚 | `services.control` |
| `POST /api/settings`（卡片/显示偏好） | `settings.manage` |
| `/api/sessions*` 写、`/api/projects*` 写、pending、permissions/answer | `sessions.write` |
| 其余 `/api/*` GET、`/ws`、`/api/settings` GET | 认证即可（viewer 可读） |
| `/router/api/*`（BFF 读写） | 认证即可，不按角色执法 |

router 明确排除在 RBAC 之外：`/router/api/*` 只受登录门 + 自身守卫
（POST-only + origin 检查）；router 进程自身的下游 token 鉴权
（`[router] auth_token`，默认关闭）有独立体系，本变更不接不动
（后续单独做）。

`/api/admin/*` 自身的 control-secret 会话保持第二层不动；RBAC 只是
在其外再按角色拦截 viewer/member。

### D4: 首启引导顺序（替换现有 `bootstrap_auth`）

启动时按序：

1. 打开/初始化 auth.db（建文件 + schema）。
2. 零用户且 `SEBAS_WEBUI_USER`+`SEBAS_WEBUI_PASSWORD` 非空 → 建 root。
3. 仍零用户：loopback → 进入设置页模式；非 loopback → 配置错误拒启
   （`webui_cmd` 安全门改为「存在启用用户」判定）。
4. **删除**的旧路径（不做兼容）：自动生成随机密码、旧 `webui-auth.json`
   读取/迁移、`SEBAS_WEBUI_TOKEN`、`SEBAS_WEBUI_AUTH_FILE`。

`POST /api/auth/setup` 在 `UserStore` 事务内「数用户→建 root」原子完成，
并发双请求第二个撞唯一约束/非零用户 → 409。密码 <8 位 → 400（CLI
`webui-passwd` 维持现状 warn 不拦截，沿用既有姿态）。

### D5: 登录与会话

- `POST /api/auth/login`：只接受 `{"username","password"}`（按用户名查
  记录、验 PBKDF2，不存在的用户名跑一次哑哈希防枚举时序）；旧
  `{"secret"}` 形态缺失字段 → 400。
- 会话 cookie 语义不变（HttpOnly/SameSite=Lax/24h/按 IP 限速 5 次每
  30s）+ 同源校验不变；`Session` 绑 `user_id`。
- 每请求角色实时解析：`auth_guard` 用 session→user_id 查库（enabled？）
  ——改角色即时生效、禁用即踢，与 spec 一致。每请求一次主键查在
  进程内 SQLite 上开销可忽略；将来有需要再上代数缓存。
- `GET /api/auth/me` 增 `needs_setup` 与（认证后）`role`。

### D6: 用户管理 API（全 POST，沿用项目 mutation posture）

```
GET    /api/users                 列表（无哈希字段）
POST   /api/users                 {username, password, role}
POST   /api/users/{id}/password   {password}        （踢该用户会话）
POST   /api/users/{id}/role       {role}            （最后启用 root 保护）
POST   /api/users/{id}/enabled    {enabled}         （禁用踢会话）
DELETE /api/users/{id}            （清会话；不能删自己；最后 root 保护）
```

最后-root 保护在 `UserStore` 内判定（当前启用 root 数 ≤1 且目标是 root
的删/禁/降级 → `LastRoot` 错误），不依赖 handler 侧检查。

### D7: CLI `webui-passwd` 改写

`sebas webui-passwd --user <name> [--password-stdin|--password] [--role R]`：
写 auth.db；建第一个用户默认 root，之后默认 member（`--role` 可显式）。
不再读写任何 JSON 凭据文件。

### D8: 前端

- `login-view.ts`：用户名 + 密码两字段（去掉 hintUsername 单字段形态）。
- 新 `setup-view.ts`：首启建 root（用户名/密码/确认密码），成功即入工作台。
- `app-shell.ts`：`authState` 增 `'setup'`（由 `needs_setup` 驱动）；
  按 role 隐藏无权限入口（呈现层优化）。
- `settings-modal.ts` 增「用户管理」区（root 可见）：用户表 + 新建
  （用户名/密码/角色下拉）+ 行内操作（改角色、重置密码、启停、删除），
  最后-root 与自删错误就地展示 400 文案。
- `client.ts`：`api.users*` 封装；403 走通用错误提示。

## Risks / Trade-offs

- [每请求一次 DB 查询] → 进程内 SQLite 主键查，µs 级；真成瓶颈再加
  代数缓存（用户表变更时 bump 计数）。
- [setup 并发抢注] → 事务内零用户校验 + 用户名唯一约束，后者 409。
- [用户名枚举时序] → 不存在用户名跑哑 PBKDF2（沿用现有防侧信道模式）。
- [auth.db 损坏] → 与今日凭据损坏同姿态：登录/鉴权一律拒绝并报错日志，
  不静默降级为免鉴权。
- [旧形态全删的连带面] → 前端同步改两字段登录；tasks.py 沙箱
  （`SEBAS_WEBUI_AUTH_FILE` → `SEBAS_WEBUI_AUTH_DB`）、Playwright helper
  `detached.ts`、e2e/验收套件中凡用 token/单字段/admin-admin 的用例改走
  env 或 `webui-passwd` 引导；AGENTS.md 沙箱菜谱同步更新。
- [Windows 文件锁/杀进程残留 -wal] → WAL + busy_timeout 已是
  `sebas_state` 验证过的组合。
- [回滚] → 换回旧二进制即回旧单账户行为（旧代码继续读未动的
  `webui-auth.json`）；期间在 auth.db 里建的用户丢失，可接受并记录。

## Migration Plan

无迁移（未正式发布，旧系统不兼容）：auth.db 从零开始，旧
`webui-auth.json` 不读取不迁移（留着无害，可手工删除）。回滚即换回
旧二进制。配置无新增键（`auth` 开关语义不变）；新增 env
`SEBAS_WEBUI_AUTH_DB` 仅沙箱/测试需要。

## Open Questions

- 用户管理 UI 的视觉细节（表格 vs 列表卡）实现时随现有 settings 风格
  走，不影响契约。

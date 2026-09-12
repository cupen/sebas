## Why

WebUI 登录鉴权目前是全局单账户（`~/.sebas/webui-auth.json` 一个用户名+密码），
无第二个用户、无角色、无权限概念，且首启凭据靠自动生成随机密码或环境变量注入，
运营者无法在界面上自定义首个账户。需要升级为多用户 RBAC 体系。

## What Changes

- **多用户存储**：新建独立 SQLite 文件 `auth.db`（默认 `~/.sebas/auth.db`，
  `SEBAS_WEBUI_AUTH_DB` 覆盖），存用户表（用户名、角色、PBKDF2 哈希、盐、
  迭代次数、启用状态）。**绝不保存明文密码**。
- **首启 root 自定义**：鉴权开启且零用户时，WebUI 渲染首次设置页，
  运营者自定义 root 用户名+密码（替代现在自动生成 `admin`+随机密码的引导）。
- **RBAC 角色权限**：固定角色集 root / admin / member / viewer，
  角色→权限映射由服务端代码定义（用户管理、系统设置、服务控制、会话写、
  只读），路由层按权限执法。
- **用户管理**：root 在 WebUI 设置页管理用户——列表、添加（指定角色）、
  改角色、重置密码、禁用/启用、删除（root 自身与最后一个 root 受保护）。
- **旧系统不做兼容（未正式发布）**：**BREAKING**——移除自动生成随机
  密码的引导路径；登录只有 `{"username", "password"}` 一种形态
  （`{"secret"}` 单字段登录与 `SEBAS_WEBUI_TOKEN` 移除）；旧凭据文件
  `webui-auth.json` 与 `SEBAS_WEBUI_AUTH_FILE` 不再被读取或写入。
- **CLI/环境引导**：`sebas webui-passwd --user <name>` 改写 auth.db；
  `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` 引导 root（容器/公网部署）。
- **会话绑定用户**：会话 cookie 绑定用户 id+角色快照；禁用用户、重置密码
  可使其会话失效。
- 保留现状已满足的部分：服务端开关 `[watchdog.webui] auth` 默认 `true`、
  `false` 免登录、非 loopback 与开关联动的安全门（零用户时非 loopback
  拒绝启动，需先经环境变量或 CLI 建 root）。

## Capabilities

### New Capabilities
- `webui-user-management`: 多用户存储（auth.db）、首启 root 设置页、
  用户管理 API 与 UI、角色权限模型与会话绑定。

### Modified Capabilities
- `webui`: 「鉴权开关（auth）与凭据自动引导」改为零用户设置页引导
  （删自动生成随机密码路径，REMOVED+ADDED 整体替换）；「单字段登录」
  移除，替换为「多用户登录形态」（仅 `{"username","password"}`，
  REMOVED+ADDED）；「非 loopback bind 与开关联动」与「Local-only
  binding」改为按用户库判定并增加零用户拒绝启动条件；「Optional admin
  authentication」仅更新交叉引用。

## Impact

- `sebas-webui`：`auth.rs` 重写为 `AuthStore`（SQLite）；`server.rs`
  鉴权中间件挂权限执法；`api.rs` 新增 `/api/auth/setup`、`/api/users/*`；
  前端登录/设置/用户管理视图；Cargo 增 `rusqlite` 依赖。
- `src/webui_cmd.rs`：`bootstrap_auth` 重写（env 引导 root，删 JSON/token
  路径）；`webui-passwd` 改写 auth.db。
- 测试与文档：AGENTS.md 沙箱菜谱、`config/config.toml.example`、
  tasks.py 沙箱（`SEBAS_WEBUI_AUTH_FILE` → `SEBAS_WEBUI_AUTH_DB`，
  admin/admin 改走 env/webui-passwd 引导）、Playwright helper
  （`tests/testsuite-webui/tests/helpers/detached.ts`）、e2e/验收套件适配。

## Non-goals

- 不做按用户的数据隔离（会话/项目对所有登录用户共享，权限只约束操作能力）。
- 不做自定义角色/细粒度权限编辑 UI（角色集与映射固定在代码里）。
- 不做多因素认证、密码策略引擎、审计日志。
- 不动 `/api/admin/*` 控制面自身的 `SEBAS_CONTROL_SECRET` 鉴权
  （只把它纳入角色权限的覆盖范围）。
- router 不纳入 RBAC：`/router/api/*` BFF 仅要求登录并保持自身守卫；
  router 自身的下游 token 鉴权（`[router] auth_token`，默认关闭）有
  独立体系，暂不接不动（后续单独做）。

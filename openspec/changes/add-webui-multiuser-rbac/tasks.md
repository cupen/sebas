# Tasks: add-webui-multiuser-rbac

## 1. 存储层（user_store + rbac）

- [ ] 1.1 `sebas-webui/Cargo.toml` 加 `rusqlite`（workspace 0.40 bundled），
  `cargo check -p sebas-webui` 通过
- [ ] 1.2 新建 `sebas-webui/src/user_store.rs`：打开/初始化 auth.db
  （默认 `~/.sebas/auth.db`，`SEBAS_WEBUI_AUTH_DB` 覆盖；WAL +
  busy_timeout + user_version=1 schema），单测覆盖建库幂等与 env 覆盖
- [ ] 1.3 `user_store` CRUD：list/create/set_password/set_role/
  set_enabled/delete/get_by_username/get，含 `UsernameTaken`/`LastRoot`/
  `NotFound` 错误枚举；单测覆盖大小写不敏感唯一、最后启用 root 的
  删/禁/降级拒绝、哈希字段不出现在 list 输出
- [ ] 1.4 新建 `sebas-webui/src/rbac.rs`：`Role`/`Permission` enum 与
  `Role::permissions()` 映射（spec 表代码化），单测覆盖四角色的
  权限矩阵与未知角色字符串解析拒绝

## 2. AuthHandle 多用户化

- [ ] 2.1 重写 `sebas-webui/src/auth.rs`：`AuthHandle` 内部换
  `UserStore` + `SessionStore`，删 mtime 热重载与 `login_secret`/token
  相关代码；`login` 只接受用户名+密码（按用户名查库验 PBKDF2，不存在
  用户名跑哑哈希防枚举）；单测覆盖登录/禁用拒登/`LoginError` 三态
- [ ] 2.2 `AuthHandle` 新增 `setup_root()`（事务内零用户校验 + 建 root）
  与 `identity_for_session()`（session→user_id→enabled+role 实时解析）；
  单测覆盖并发 setup 只成一个 root、非零用户 setup 报冲突
- [ ] 2.3 `admin_auth.rs` 的 `Session` 加 `user_id`，
  `SessionStore` 增 `remove_all_for_user()`；单测覆盖按用户踢会话与
  admin 会话（user_id=0）不受影响

## 3. 服务端路由与引导

- [ ] 3.1 `server.rs`：`auth_guard` 认证后解析 `Identity` 入 request
  extensions，新增 `required_permission(path, method)` 中央表并 403 执法；
  逐路由审计映射（对照 design D3 表，`/router/api/*` 明确排除在角色
  执法外、仅登录门）；`auth_guard_tests` 扩展：viewer 写 403、member
  用户管理 403、admin 用户管理 403、root 通过、member 调
  `/router/api/*` 写不被角色拦截、禁用用户既有会话 401
- [ ] 3.2 `api.rs`：`POST /api/auth/setup`（零用户专属，409/400 语义）、
  `/api/auth/me` 增 `needs_setup` 与 `role`、`POST /api/auth/login` 只收
  `{"username","password"}`（`{"secret"}` → 400）、`/api/users` 全套端点
  （D6 面，全 POST + DELETE）；`api_endpoints_test.rs` 补齐各端点
  成功/越权/保护规则用例
- [ ] 3.3 重写 `webui_cmd.rs` 的 `bootstrap_auth`（design D4 顺序：
  建库 → env 建 root → loopback 设置页/非 loopback 拒启；自动随机
  密码、JSON 读取、`SEBAS_WEBUI_TOKEN`/`SEBAS_WEBUI_AUTH_FILE` 路径
  全删）；`auth_gate_tests` 改写：零用户非 loopback 拒启、env 引导后
  放行、旧 JSON 存在时被忽略（不读取不迁移）
- [ ] 3.4 `run.rs` 内嵌 webui 路径同步换新 bootstrap（保持同装配），
  `cargo test -p sebas --lib` 全绿

## 4. CLI

- [ ] 4.1 `sebas webui-passwd --user <name> [--role R]`：改写 auth.db
  （首个用户默认 root，其后默认 member；不再写 JSON）；更新
  `cli.rs` 帮助文本；集成测试覆盖新建/改密/角色默认值

## 5. 前端

- [ ] 5.1 `login-view.ts` 改用户名+密码两字段（去掉 hintUsername），
  `client.ts` 的 `authLogin` 改双字段载荷；`login-view` 相关测试更新
- [ ] 5.2 新建 `setup-view.ts` 首启设置页（用户名/密码/确认密码，
  弱密码与不一致就地报错），`app-shell.ts` 的 `authState` 增
  `'setup'`（`needs_setup` 驱动）；`app-shell.test.ts` 补两态断言
- [ ] 5.3 `settings-modal.ts` 增「用户管理」区（root 可见）：列表、
  新建（角色下拉）、改角色、重置密码、启停、删除，400/409 文案就地
  展示；`client.ts` 加 `api.users*`；`settings-modal.test.ts` 补渲染
  与提交用例
- [ ] 5.4 `app-shell.ts` 按 role 隐藏无权限入口（设置页分区、服务控制
  等）；`pnpm --dir sebas-webui/frontend test` 全绿

## 6. 测试设施与文档收尾

- [ ] 6.1 `tasks.py` 两个 webui 沙箱装配改走 `SEBAS_WEBUI_AUTH_DB` +
  env 引导 admin/admin（或 `webui-passwd`），Playwright helper
  （`tests/testsuite-webui/tests/helpers/detached.ts`）与登录用例改
  用户名+密码；`invoke testsuite-webui-sandbox` 手工冒烟通过
- [ ] 6.2 e2e（`testsuite_e2e_test.rs`）与验收套件里涉登录/鉴权的
  用例适配新流程；`invoke testsuite-e2e` 通过
- [ ] 6.3 文档：AGENTS.md 沙箱菜谱（`SEBAS_WEBUI_AUTH_FILE` →
  `SEBAS_WEBUI_AUTH_DB`、`SEBAS_WEBUI_TOKEN` 与随机密码路径删除）、
  `config/config.toml.example` 的 `auth` 注释更新（注意该文件当前有
  未提交的本地改动 `auth = false`，更新时保留操作者本地值）；
  `openspec validate add-webui-multiuser-rbac --strict` 通过

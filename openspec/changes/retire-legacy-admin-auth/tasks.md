## 1. 服务端退役（sebas-webui）

- [x] 1.1 admin.rs：删除 `AdminState.password` / `session_store` / `has_password()`、`api_admin_auth_guard`、`api_login_action`、`api_csrf_action`、`api_logout_action`、`LoginForm`、admin 版 `extract_session_cookie`、`SESSION_COOKIE_NAME`、`CsrfExtension`、`constant_time_eq`；`build_api_admin_router` 去掉 guard 层与三条路由；验证 `cargo check -p sebas-webui` 通过
- [x] 1.2 admin.rs：`admin_mutation_guard` 收敛为 POST-only + origin-only（删除密码/CSRF 分支，保留现行无密码模式语义）；模块文档改写（去掉 login 页与 env 密码描述）；验证守卫单测（POST-only、非 loopback origin 403）通过
- [x] 1.3 admin_auth.rs：模块文档改写为「webui 会话与登录限速的共享存储」（类型与实现不动）；验证 `cargo test -p sebas-webui --lib admin_auth` 通过
- [x] 1.4 server.rs / webui_cmd.rs 引用核对：确认无 `/api/admin/login` 残留引用、`bootstrap_auth` 行为不变；验证 `cargo test -p sebas-webui` 全绿（3 个失败为 main 上同现的 Windows 路径环境问题，与本变更无关）

## 2. 前端退役（sebas-webui/frontend）

- [x] 2.1 client.ts：删除 `adminLogin` / `adminCsrf` / `adminLogout`、`adminCsrfToken` 内存 + sessionStorage 管道、`csrfHeaders()` 及五处挂载点、`isAuthExempt` 名单中 `/api/admin/login|csrf` 两条；验证 tsc --noEmit 与 vitest 相关文件通过

## 3. 规格同步与验证

- [x] 3.1 specs delta 已建（`specs/webui/spec.md`：MODIFIED HTTP route surface / Mutation posture + REMOVED Optional admin authentication）；验证 `openspec validate --changes retire-legacy-admin-auth` 通过
- [x] 3.2 全量回归：`cargo test -p sebas-webui`（145 过；3 个失败为 main 上同现的环境问题）、frontend vitest（321 过；1 个失败为 main 上同现的 jsdom 存储环境问题）、tsc --noEmit 干净；grep 确认 `sebas_admin_session` / `X-CSRF` / `api/admin/login` 在 src 与 frontend 归零
- [x] 3.3 沙箱冒烟（AGENTS.md 菜谱手工执行，Windows 下 testsuite_e2e_test 因既有的 Linux-only `find_child_pid` 门控无法编译，属平台限制与本变更无关）：auth=false 形态 `/api/admin/services` 200 + `adapter_ok:false`、`POST /api/admin/restart` 503 诚实降级、`/api/admin/login|csrf` 404 退役；auth=true 形态未登录 401、`/api/auth/login` 建 root 会话后仅凭该会话 `GET services` 200、`POST restart` 503（RBAC 单层执法，无第二把 cookie）、跨源写 403；沙箱目录已删除、端口已释放

# Tasks — gate-agent-directory-writes

## 1. 服务端执法

- [x] 1.1 权限矩阵落码：agents 写（增/改/删）归 settings.manage 档；`/api/agents*` 的 POST/PUT/DELETE 路由挂执法守卫，member/viewer 403；GET 保持登录门。验证：webui 单测四角色 × {create, update, delete, list} 矩阵全绿；TESTSUITE_AUTH=1 形态补 viewer-写-agents-403 用例。
- [x] 1.2 禁用/角色即时生效路径对 agents 写同样成立（复用既有执法层，补一条断言即可）。

## 2. 前端呈现层

- [x] 2.1 按角色隐藏写入口：Agents 分区 New/Edit/Delete（<admin 不呈现）；viewer 的「新建会话」入口隐藏。验证：前端单测（role→入口可见性映射）+ GUI 手测 viewer/member/admin 三档截图。
- [x] 2.2 角色展示位：侧栏用户区展示当前角色（数据取 `/api/auth/me` 的 role），登出后无残留。验证：GUI 手测三档角色显示正确。

## 3. 回归与验收

- [x] 3.1 全量 `cargo test` + 前端单测过；既有 auth 套件（登录/setup/RBAC 既有用例）不回归。
- [x] 3.2 GUI 验收（auth 沙箱，admin/member/viewer 三档）：admin 增删 agent 成功；member/viewer 入口隐藏且直调被 403；viewer 无 New session 入口但可只读浏览会话与 agent 列表；侧栏角色显示正确。截图留档。

> 执行备注（2026-09-30）：截图留档在
> `C:/Users/cupen/AppData/Local/Temp/sebas-qa/qa-evidence/gate-agent-directory-writes/`
> （1-admin-workbench-role、2-admin-agent-created、3-member-new-session-present、
> 4-member-agents-readonly、5-viewer-no-new-session、6-viewer-agents-readonly、
> 7-after-logout-login-page，共 7 张；17 项 GUI 检查全过）。
> TESTSUITE_AUTH=1 形态的守卫用例落在 `tests/testsuite-webui/tests/agents-gate.spec.ts`
> （4 案全绿，已挂入 playwright.auth.config.ts 的 testMatch）。

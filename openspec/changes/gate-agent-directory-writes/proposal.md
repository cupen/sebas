# Proposal — gate-agent-directory-writes

## Why

第三轮 GUI QA（`qa-evidence/w4/`）实测：**viewer（最低角色）与 member 都能增删改全局 agent 目录**——bob（viewer）经 Settings → Agents 成功创建并删除 agent（绿横幅「已创建 …（免重启，创建会话下拉立即可选）」）。agent 目录是影响所有会话的执行体配置，权限矩阵（`webui-user-management`「RBAC 角色与权限执法」）却没有 `/api/agents` 的对应行，服务端未执法——这是矩阵缺口导致的越权写。同轮实测还发现 viewer 的「New session」入口未按角色隐藏，点击后才收 403，违反同 spec「前端 SHALL 按当前用户角色隐藏其无权限的入口」的呈现层要求。

## What Changes

- 权限矩阵补一行：**agent 目录写（增/改/删，agents.manage）归 `settings.manage` 档（root/admin）**；member/viewer 调用返回 403（服务端路由层执法，与既有执法同层）。
- 前端按角色隐藏 Agents 分区的写入口（New/Edit/Delete 按钮）——呈现层优化，不构成防线。
- viewer 的「New session」入口按角色隐藏（sessions.write 不含 viewer）。
- Roles 字段的 UI 呈现补齐：当前用户角色在界面上有展示位（现状 `/api/auth/me` 已返回 role，但 UI 无处显示）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui-user-management`：「RBAC 角色与权限执法」矩阵增加 agents.manage 行（root/admin）；补 viewer 无会话写入口隐藏的场景；「me 返回身份与角色」补充角色 UI 展示位要求。

## Impact

- 后端 `sebas-webui/src/routes.rs`（`/api/agents*` 写路径执法 403）；无 wire 形状变更（新增拒绝路径）。
- 前端 `sebas-webui/frontend/src`（settings-modal Agents 分区写入口、dashboard/New session 入口、当前用户角色展示位）。
- 验证：webui 单测（四角色 × agents CRUD 的执法矩阵）+ GUI 手测（viewer/member 被拒且入口隐藏、admin 可写）。

## Non-goals

- 不改 provider / model-aliases 的「仅登录门」豁免（规格明文保留，本期不动）。
- 不做角色自定义、细粒度 ACL 或按 agent 的授权；角色集仍固定四档。
- 不改 skills 写面（`/api/skills*`）的执法——现状与矩阵口径一致，留待后续 change 单独审视。

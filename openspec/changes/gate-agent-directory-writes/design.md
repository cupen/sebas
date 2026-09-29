# Design — gate-agent-directory-writes

## Context

QA 证据（w4）：bob（viewer）`8e_bob_agent_probe_result.png` 成功创建 agent；alice（member）`7c_alice_agent_after_create.png` 同样成功；二者都有 Edit/Delete 入口。现有矩阵无 `/api/agents` 行 → 路由层未执法。另：viewer 的 New session 入口可见可点（`8d_bob_session_created.png` 点击后 403），违反呈现层隐藏要求；`/api/auth/me` 已返回 role 但 UI 无展示位（`9_after_logout.png` 仅见用户名）。

## 关键决策

- **agents.manage 并入 settings.manage 档（root/admin），不新设独立权限位**：agent 目录与系统设置同属「影响全局行为的管理面」，权限模型保持「固定权限位 × 四角色」不膨胀；矩阵新增一行**引用同一档位**，实现上是同一执法函数的再标注。被否备选：给 member 开放 agents 写（理由「member 可建会话选 agent」）——否，执行体配置影响所有用户的会话安全（path/args 注入面），member 的 sessions.write 不该外溢到全局执行配置。
- **读保持登录门**：agent 列表本就是选 agent 建会话的前置数据，viewer 只读浏览无害且现状如此，只收写。
- **前端隐藏入口清单**（按角色，读 `/api/auth/me` 的 role）：Agents 分区 New/Edit/Delete（<admin 隐藏）；New session 入口（viewer 隐藏）。Users/Services 分区隐藏现状保持。
- **角色展示位**：侧栏用户区「退出 (username)」扩为「退出 (username · role)」级别的小改，不新增页面。

## 执法落点

`sebas-webui/src/routes.rs` 的 agents 路由族（POST/PUT/DELETE on `/api/agents*`）挂与 settings.manage 相同的执法守卫；GET 保持登录门。既有执法中间件/辅助函数复用，不引入第二套权限代码。

## 风险

- 既有 Playwright 套件若以默认（无 auth）形态跑，登录门关闭时执法路径不激活——回归无影响；TESTSUITE_AUTH=1 形态补一条 viewer-写-agents-403 的守卫用例。
- 免重启 agent 目录（store 行）行为不变，仅门控收紧。

## 假设

- root 与 admin 同档可写（矩阵 settings.manage 行现状即 root/admin），QA 未测 admin 写 agents——按矩阵语义补齐即可，不另设限制。
- skills/provider 写面维持现状（规格明文豁免 providers；skills 留待后续 change）。

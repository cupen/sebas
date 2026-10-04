# fix-webui-qa-round12 Proposal

## Why

第十二轮 GUI 全功能验收（QA-A 认证/用户/RBAC/项目/设置/技能簇 + QA-B 会话核心链路簇，
证据在 `C:\Users\cupen\AppData\Local\Temp\sebas-qa-r12\findings-a.md` 与
`findings-b.md`，沙箱 fake-claude + native `test/*` 场景模型）给出双重结论：round11
的 11 项修复 **10 项确认修复**（含 P1 技能删除门禁、native 生命周期、流式停止、
多行气泡、深链止停、WS 认证门禁、usage 刻度、文案歧义等），**1 项未修复**——回合
后台完成通知（round11 B-3）三轮独立验证零通知，升格为本轮 R12-B-2（P2）。另发现
新缺陷 2 个：crash 后会话头部 token 累计回退到首回合值且刷新不恢复（R12-B-1，P3）、
技能「同步」按钮对 member/viewer 可见而删除已收口（R12-A-1，P3，sync 会投影清理
backend 副本，属可触发删除效果的写操作）。打磨项 3 个：crash 恢复后每回合弹
「模型已切换：default → fake」假回执（O-2）、fakeacp 会话切模型无回执与
claude/native 不一致（O-3）、死会话深链首载仍集中 4 条 404 未达 round11 delta 的
「每次导航至多一次失败请求」（O-5）。同时补 round11 欠账的进程级回归（tasks 1.3/5.4）：
`invoke testsuite-e2e` **7 红**（pending 队列 3、home socket 派生 1、并发回合 1、
native 流式 1、取消终态条目 1），`invoke testsuite-acceptance` 绿。主链路（登录、
RBAC、项目、会话、审批四档、thinking、流式、异常/空回合、用量、持久化、主题）
全部验证可用，console pageerror = 0。

## What Changes

- 回合终点通知接线修复：非聚焦会话的回合完成/失败通知改由 `/ws` 全局会话事件流
  驱动（历史页徽章 Working→Done 的被动变化证明事件已到达前端，通知层未消费），
  废弃对前台聚焦/轮询的依赖；聚焦会话不弹的既有语义不变
- token 累计重启不回退：会话累计 token 以会话侧持久值为准，agent 子进程重启
  （crash 恢复）后的重报只做单调合并，不再覆盖已累计值
- 技能 sync 角色门禁：同步投影限 root/admin（复用 round10/11 接线的
  `settings.manage` 权限键），服务端路由守卫强制 + 前端技能页对只读角色隐藏
  「同步」控件（与删除同款口径）；rbac 矩阵不动
- 模型切换回执收口：回执只由操作者显式切换成功产生且对全部 agent 驱动一致
  （claude / native / fakeacp 补齐）；agent 子进程重启后的模型重报不产生回执
- 死会话深链首载去重：session + transcript 初始并行加载在首个 404 后止停依赖
  请求，达成 round11 delta「每次导航至多一次失败请求」的既有要求
- e2e 7 红回归修复：逐例诊断（现场保留在 `target/tests/sebas/testsuite_e2e/`）——
  pending 管理两形态的 pending 就绪超时与非队列持有通道 remove 的 503 语义、
  home 派生 channel socket 落点、双会话并发回合窗口、native 流式增量出现、
  停止回合终态条目形态（按既有 spec 对齐实现与测试期望）
- 回归验证收口：fake-claude + `test/*` 场景 GUI 抽查逐缺陷销账；`invoke
  testsuite-e2e` 与 `invoke testsuite-acceptance` 双绿；webui 浏览器套件相关旅程不回归

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: 分级通知层的回合终点通知钉死事件源——由全局会话事件流驱动，不依赖
  会话在当前标签页被打开过
- `agent-skills`: 技能同步投影与删除同门槛——限 root/admin，只读角色无同步控件
- `agent-workbench`: 新增会话 usage 累计的重启不变性；新增模型切换回执的真实性
  与跨驱动一致性

## Impact

- `sebas-webui/frontend/src/views/turn-notify.ts`（事件源改为共享 WS 会话事件流）、
  `shared-ws.ts`（会话事件分发给非聚焦会话）、`role-visibility.ts`（同步控件隐藏）
- `sebas-webui/src/server.rs`（skills sync 路由守卫）、`src/agent_backend.rs`
  （crash 重启后的 usage 单调合并、模型重报不广播回执）
- `tests/testsuite_e2e_test.rs`（7 红用例按诊断结论修实现或修测试期望）、
  `tests/skills_webui_test.rs`（sync 守卫 API 测试）、前端单测（turn-notify、
  role-visibility）
- 无 wire 形状破坏性变更；全部为行为修复、门禁收紧、呈现修复与测试对齐

## Non-goals

- 通知中心/铃铛收件箱：分级通知层既有瞬时 toast 语义够用，不扩新通道
- native 会话命名口径家族（key「飞书 ·」前缀、回复者标签「F fakeacp test/thinking」、
  头部「默认 agent」）：r11 已留观察，本轮复核仍在，维持留观
- usage 只统计 router 透传流量的口径、native bash 工具 Windows 平台边界、
  档位/模型切换回执的文案措辞形态：维持观察项
- rbac.rs 四档权限矩阵本身的重排（只复用既有 `settings.manage` 键接线）
- 休眠计数阈值语义（本轮无法快进验证，维持现状）

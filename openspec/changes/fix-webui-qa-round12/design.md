## Context

round11 已把回合完成通知的前端门控逻辑写进 `turn-notify.ts` 并带单测，但 QA-B 三轮
浏览器验证零通知；同一时段历史页会话卡徽章却能从 Working 被动翻到 Done——说明
`/ws` 推送的会话状态事件**已经到达前端**，通知层只是没有消费它。crash 路径上，
fake-claude 子进程重启后会话头部 token 从累计值回退到首回合值且持久化（reload 不恢复），
说明累计值的数据源是子进程上报的 cumulative 计数，重启即失忆。技能页在 round11 给
DELETE 挂了 `settings.manage` 守卫并对只读角色隐藏控件，但「同步」按钮（sync 会把
仓投影到 backend 落点并清理上次投影过、仓里已删的条目）两条防线都没有。模型切换回执
目前散落在各驱动路径：claude/native 成功切换有「模型已切换：X → Y」系统回执，
fakeacp 没有；crash 重启后子进程以默认模型重报，前端把它当成了切换，每回合弹
「模型已切换：default → fake」。进程级 e2e 7 红现场保留在
`target/tests/sebas/testsuite_e2e/`（kept-for-diagnosis 目录），acceptance 套件绿。

## Goals / Non-Goals

**Goals:**

- 非聚焦会话回合终点通知端到端可达（浏览器旅程可证），事件源钉死为 `/ws` 全局会话事件流
- crash 恢复后会话累计 token 不回退、继续单调累加
- 技能 sync 与 DELETE 同门槛（服务端 403 + 控件隐藏）
- 模型切换回执：只由显式切换成功产生、跨驱动一致、重启重报静默
- e2e 恢复 82/82，且逐例结论落设计记录
- 死会话深链首载满足既有「每次导航至多一次失败请求」

**Non-Goals:**

- 通知中心/收件箱、通知持久化（分级通知层瞬时语义不变）
- rbac 矩阵重排、native 命名口径家族、usage 统计口径（见 proposal Non-goals）

## Decisions

- **D1 通知事件源 = 共享 WS 的会话状态/回合终点事件**：`shared-ws.ts` 已把会话状态
  推送分发给订阅方（历史页徽章即消费者），turn-notify 改为同一事件流的消费者——
  按 session key 比对「当前聚焦会话」，非聚焦且终态为 done/failed 时按级弹 info/error
  瞬时条。否决「前端轮询非聚焦会话」：请求噪音大、多会话扩展性差、且与既有事件流重复。
  事件里没有的语义（完成 vs 失败的区分）以状态 slug 为准，不新增 wire 字段。
- **D2 token 累计所有权 = 会话侧持久值，子进程上报只做单调合并**：合并规则
  `merged = max(persisted, reported_cumulative)`（正常路径 reported 只增、合并即透传；
  crash 重启后 reported 从 0 重报、合并保住历史值，子进程再次超过历史值后无缝续增）。
  否决「persisted + reported 增量相加」：无法从 cumulative 重报中可靠提取增量，重复计数
  风险高。写回时机沿用现有元数据回填路径。
- **D3 sync 门禁复用 `settings.manage` 键**：服务端在 skills sync 路由挂与 DELETE 同款
  守卫（同一条 typed permission 错误路径、同款「权限不足」文案），前端
  `role-visibility` 把「同步」与「删除」同样对待。否决「新增 skills.sync 专用权限键」：
  round11 先例已把删除归入 `settings.manage`，同步是同类写操作，矩阵不动、键不增。
- **D4 切换回执的产生点收口到「操作者显式切换成功」**：webui 发起 set model 成功后
  由前端写入系统回执（与「权限模式已切换」回执同机制），driver 重报（含 crash 重启
  后的 default/fake、native 场景模型重报）一律不产生回执。这同时修平 fakeacp 缺回执
  （不再依赖各驱动自行广播）。拒绝路径维持既有错误卡不动。
- **D5 e2e 7 红逐例诊断、按既有 spec 对齐**：pending 两形态的超时与 503 语义、home
  socket 派生落点、并发回合窗口、native 流式首条目，优先怀疑 round11 工作区改动的
  回归（`agent_backend.rs`/`server.rs`），用 kept-for-diagnosis 现场与 `git stash` 对照
  定位；`receipt_phase_cancel` 的「停止终态条目」若测试期望（error 条目）与现行呈现
  （notice「回合已取消…」）冲突，以 session-lifecycle 既有 spec 文本为准裁决并记录。
  **裁决记录（5.2 实施结论）**：①超时类 3 例（pending 两形态就绪、并发回合窗口）
  复跑即绿，非代码回归；②503 语义：round8 2.2 起 native 承载影子队列，未知会话
  的类型化 404「会话不存在」是现行诚实契约，503「此后端不承载待执行队列」面由
  session_backend 单测承载——修测试期望；③home socket：Windows 侧 IPC 为按路径
  映射的命名管道、`core.sock` 文件永不落盘，断言改平台感知（unix 验文件、windows
  验 `run/` 派生目录）；④取消留痕：session-lifecycle spec「取消最终生效并留痕」
  只要求停止留痕可见——round8 7.3 的中性 notice 是 spec 合规形态，用例 6/7 改断言
  notice；⑤实现侧唯一修改在深链首载（见 5.3），e2e 断言之外无实现回退。
- **D6 深链首载去重在初始加载编排层**：session 与 transcript 首个请求任一 404 即进入
  既有「会话不可得」分支并取消同导航内的另一个在途/后续请求；不加新端点、不改轮询
  之外的语义。

## Risks / Trade-offs

- D1 依赖会话状态事件的 slug 语义（done/failed），若服务端事件在极端时序下缺发，
  通知随之缺发——与历史页徽章同源同命，不引入第二事件源；浏览器旅程以确定性场景兜底。
- D2 的 max 合并在「会话侧持久值被手动清零」类运维场景下会优先保留大值——当前不存在
  该场景，接受。
- D4 把回执产生点移到前端后，非 webui 入口（CLI/飞书）切模型不再有转录回执——回执
  本就是 webui 转录概念，其他入口无此呈现，无回归面。
- e2e 修复可能牵出 round11 改动的真实回归（尤其 native 流式与并发窗口），修复以
  既有 spec 为锚；若确认属 round11 缺陷，修在本 change、不回退 round11 已验收的
  GUI 行为。

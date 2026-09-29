# Design — fix-webui-qa-round2

## Context

2026-09-29 四簇 GUI 黑盒验收（40 点，报告在 `evidence/`）确认核心链路健康，遗留 4 P2 + 约 10 P3。本轮全部缺陷都发生在**既有能力面**上，无新架构。上轮 change `fix-webui-qa-findings`（18/22）已修未读徽标回归、崩溃 UI 终态、D3 pending-cancel 等；本 change 不与其重叠（见 proposal Non-goals）。

关键既有口径：`permission-flow` spec 明文 `allow`/`auto` 同映射 bypass tier（simplify-mode-menus 刚收敛四词菜单）——因此 M-C6 只修**描述文案**，不动语义。

## Goals / Non-Goals

- Goals：四个 P2 行为修复；P3 缺失与呈现修正；验收报告中的每一项缺陷在转写/设置面上有可复测的行为改善。
- Non-Goals：不做转写虚拟化重构（先用批量渲染缓解，见 D7）；不改 auth/RBAC；不动 add-about-build-info 的 BUILD 段；不重做四词模式菜单。

## Decisions

**D1 — D-B11 路径输入：删掉字面净化，校验后置去抖（前端 project-rail）**
Add-project 输入处理里有对输入值的破坏性字符过滤（反斜杠被整类剥掉）。修复 = 输入值原样保存原样提交；校验只在去抖窗（~300ms）与提交时发生，服务端 400 的 `cause` 字段如实透出到错误文案。备选「输入时实时调 API 校验」被否：每键击网络校验正是本轮观测到的 400 风暴成因。

**D2 — D-B215 崩溃快速终态：复用停滞看门狗的 force-settle 路径，子进程死亡时立即触发（sebas-dispatch / agent_backend）**
上轮 D5 修复让 UI 经 force-settle 快速出终态卡，但 dispatch FSM 的回合相位要等停滞看门狗（数百秒）才落终态。决策：子进程退出且回合仍在 working 时，直接走同一条 force-settle 收尾（终态 slug、session.updated 帧、composer 解锁），不新造第三条收尾路径。备选「缩短停滞阈值」被否：会误伤慢后端（slowbot 形态）的合法静默期。**风险**：误触发 → 以「子进程已退出」为唯一触发条件，静默但进程存活的回合不适用。

**D3 — D-C3 结果读达：决策后 tool_result 顶层化，父折叠不再连带（transcript-view）**
现状是 process 折叠内嵌 tool 折叠，点内层先折父层。决策：审批决策落地后，tool 结果条目提升到转写顶层（✓已执行/✗已拒绝 chip 常驻），process 折叠只包未决策前的过程帧；点击互不连带。**M1 余量在此收口**（旧 change 4.2 标注部分完成）。
单工具回合丢环后正文是独立投影缺陷：先查 sebas-dispatch 事件投影（engine events）是否把 tool_result 之后的 assistant 文本并入/丢弃，再在投影层修，不动 fake-claude。

**D4 — D-C5 并行审批：pending 列表驱动渲染，决策后条目保留（transcript-view + 可能涉 dispatch）**
现状一次只渲染一张卡。决策：转写按「当前挂起审批列表」渲染全部待批卡（引擎本就按 request_id 独立路由决策，见 permission-flow spec「Parallel tool calls each get their own request id」），决策后的条目与结果常驻。**风险**：若 wire/session 状态只暴露最新一条待批，则需后端把 pending 审批集合随 session 状态上行——实现时先探 wire 再定改动面，两条路径都在本 change 内完成。

**D5 — 未读分界线：聚焦时按 read anchor 补绘，读后即清（transcript-view + dashboard 徽标锚点）**
现状分界线只对「打开视图期间到达」的回合绘制（且错误回合才会出现），重聚焦路径不绘。决策：聚焦会话时若 localStorage read anchor < 当前条目数，在 anchor 位置补绘分界线，滚过即推进 anchor 清线；聚焦中贴底流式照旧不绘（现行正确行为保留）。注意保留 placeholder 首交换不绘线的既有规则。

**D6 — /compact 回执与单一分派（workbench-composer）**
Enter 与发送按钮是两条提交路径导致 slash 判定分叉。决策：收敛为单一 submit 函数（slash 前缀判定在函数内做一次），命令提交回执走既有 dispatch-commands 投影出转写条目；toast 只作辅助不再唯一反馈。

**D7 — flood 卡顿：增量批量渲染，不先上虚拟化（transcript-view）**
1200 条一次性 append 造成 ~1.2s 主线程阻塞。决策：摄入按帧分批 append（rAF/微任务分片，每片 ≤50 条），DOM 结构不变。虚拟化列为后备（若批量后仍 >300ms 冻结再立项）。

**D8 — M-A4 agent 表单补字段（settings-modal）**
表单增加 sessions_dir / work_dir / args 三字段，映射 store agents 表既有列（config 种子已携带同形数据，无 schema 变更）；args 用键值对编辑器（与 config 形态一致）；display name 编辑以存储值预填。D-A4 回填缺陷随本项一并修。

**D9 — 旧 change 账目校正（纯文档）**
`fix-webui-qa-findings/tasks.md`：3.2（D3）勾选完成并注「本轮验收实证已实现」；4.2（M1）注「折叠头反馈已存在，内容读达余量移入本 change D-C3」。不改其 spec。

## Risks / Trade-offs

- [D4 需要探明 pending 审批的 wire 现状] → 实现首任务即探 wire；若后端要动，先补 `session.updated`/状态投影的单测再改。
- [D2 与停滞看门狗竞态] → force-settle 幂等（现有实现即幂等），子进程死亡触发与看门狗触发共用同一把相位锁。
- [D5 分界线与 placeholder 首交换规则冲突] → 复用既有 anchor 建立时机，聚焦空会话仍不绘线。
- [D7 批量渲染对 drip 流式的 6ms 首包可见性回归] → 批量分片只影响大批量路径，条目数 ≤ 阈值时走现行直渲染；e2e 覆盖 drip 首包。

## Migration Plan

单二进制同发（core/webui/frontend 同仓同构建），无数据迁移；localStorage read anchor 格式不变。回滚 = revert 单个 merge commit。

## Open Questions

（无——D4 的 wire 现状是实现期第一个任务，答案不改变 spec 与任务拆分。）

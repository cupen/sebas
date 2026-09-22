## Context

第四轮 GUI 验收（真实 Chromium + fake-claude/fake-acp 沙箱）实测证据：

1. **接收回执阶段无停止控件（P1）**。fake-claude `hang` 场景：提交后 DOM 持续呈现
   `Send [disabled]`（空输入分支），无停止方块；transcript 状态章为「已收到（等待 agent
   开始输出）」。composer 的 `submitState()`（workbench-composer.ts ≈L317-330）仅在
   `turnInFlight` 时给 `stop/queued`，而 `turnInFlight` 派生自 working/engaged 态——
   接收回执阶段（prompt 已是最新转录单元、agent 首条目未落）不计入。该阶段可能持续
   很久：dispatch `[dispatch] turn_stall_timeout` 默认 600s（config.rs default_turn_
   stall_timeout），是唯一的兜底收尾。
2. **升级击杀后整个会话消失（P0）**。claude 会话（hello/perm×2/drip/stream/hang/goal 共
   7 个提交）发 `hang` 后：core.log `agent silent for 5m; escalating (interrupt 1/3)`；
   fake-claude journal 可见 wire 合同正确执行（interrupt → ack → `result
   error_during_execution` → exit(1)）；随后该会话从 `/api/sessions` 列表消失
   （recent_sessions 少 1、total_sessions 减 1）、`GET /api/sessions/<key>` 404。操作员
   视角：全部历史不可回看、无任何通知。删除发生在升级终止之后，具体代码点未在规划期
   定位——候选：claude driver 的 terminal-error 路径把「移除映射」扩大成了「移除会话
   记录」，或 webui 会话列表读路径把 terminal teardown 语义误用于记录删除。
3. **归档→恢复丢命名来源（P3）**。恢复后 API 行 `prompt_preview` 为空串、rail 显示原始
   引用 `web-1789856…`；归档前该会话首条消息为「跑一下命令」。
4. **打磨**：rail History 条目长路径（Windows 绝对路径）撑出横向滚动；390px 视口下会话
   头部权限模式章逐字竖排换行。

既有 spec 的相关语义（本轮 delta 是澄清/补场景，不是新语义）：
- agent-workbench「Submit control reflects submission and turn state」已把 in-flight 定义
  为 core-side truth，但枚举（working / spawn window / parked）漏了接收回执阶段；
- session-lifecycle「Terminal error teardown」清的是 card/allowlist/mapping 等活跃绑定，
  从未授权删除会话记录与转录；
- project-session-actions「Session rows are named by the first prompt」的命名来源链
  （label → 首条消息预览 → 短 id）在恢复路径上断链。

## Goals / Non-Goals

**Goals**
- 接收回执阶段停止控件可达，取消走既有 interrupt 语义。
- 升级击杀后：回合错误收尾、会话与转录保留、排队提交如实释放上报、无静默删除。
- 归档恢复保留命名来源；rail History 长路径截断；移动端模式章不断行。

**Non-Goals**
- 不改 `turn_stall_timeout` 的 600s 默认值（兜底语义不变，用户自救靠停止控件）。
- 不改 kill ladder 本身（阶梯、挂起条件、探测节奏均沿用既有 spec；探测借
  set_permission_mode 下发的 wire 噪音为观察项，不在本轮处理）。
- 不动四模式门控语义、pending-queue 语义、interrupt 的类型化拒绝契约。
- 不处理测试管线（IAB）对 shadow-DOM 组件点击 actionability 的兼容性。

## Key Decisions

- **D1 修前端判定而非加状态推送**：后端在接收回执阶段已有在飞 turn（interrupt 可用，
  空闲才 409），缺的只是 webui 的 in-flight 判定枚举。把「prompt 是最新转录单元且无
  agent 输出条目」（transcript-view `awaitingReceipt` 已派生同一事实）并入 in-flight，
  停止控件复用既有 interrupt 调用。备选「后端新增 turn 状态推送事件」被否：wire 已有
  事实（转录单元序列 + 会话状态），新增事件属重复事实源。
- **D2 定位与修复分层**：先在实现期定位删除点（driver terminal-error 路径 vs webui 读
  路径），再按「teardown 清绑定、记录保留」收敛；无论在哪层，都以既有
  `Error{terminal: true}` 单事件语义为边界，不新增事件类型。释放排队提交沿用
  「reported as not executed」既有语义。
- **M1 迁移而非重推导**：恢复是「重建会话 + 保留对话记录」，命名来源（preview/label）
  属于应保留的对话元数据，随归档快照迁移；不依赖恢复后重新截取首条消息（归档会话无
  消息可截）。

## Risks / Trade-offs

- 接收回执阶段并入 in-flight 后，空输入+该阶段的按钮从「禁用」变「可点停止」——存在
  与「starting 形态」的显示竞态；以既有优先级序（sending > starting(hasText) >
  in-flight）为准，写单测钉住。
- D2 的删除点若在 webui 读路径，修复后列表会出现「终态会话」新形态（此前被删）——
  rail/表格需容忍 failed 终态行的呈现；已由 transcript 的 errorEntryLabel
  （spawn/stall/generic）覆盖，不新增分类。

## Migration Plan

无数据迁移。恢复路径需兼容历史归档快照（无 preview 字段时回退现状行为——短 id）。

## Open Questions

- 无（规划期证据充分；删除点定位列为实现首项任务）。

## Appendix: D2 删除点定位（task 1.1 实现期记录）

**根因：删除发生在 dispatch 引擎的 terminal teardown 路径，不是 webui 读路径。**
webui 的列表/详情完全派生自引擎的映射表——映射被删，行即消失、详情即 404。

1. **删除点①（映射移除）**：`sebas-dispatch/src/engine/acp_events.rs:121` ——
   `AcpEvent::Error{terminal:true}` 臂调用 `self.map.remove_by_session(sid)`。
2. **删除点②（主动下行移除广播）**：`sebas-dispatch/src/engine/acp_events.rs:124` ——
   `self.publish_removed(&key)` 发 `SessionEvent::Removed`，detached 前端据事件丢行。
3. **读路径单一事实源**（映射没了 ⇒ 行/详情全没了）：
   `sebas-dispatch/src/engine/mod.rs:481`（`session_info_snapshot` ← `map.snapshot_all`）、
   `mod.rs:501`（`session_info_for` 无映射返回 `None`）、`mod.rs:645`（`session_turns`
   无映射返回 `None`）→ webui `sebas-webui/src/api.rs:213`（detail 查不到行 → 404
   "Session not found"）。
4. **转录并未被抹**：terminal 臂不调 `transcript_drop`（只有 `web_close_session`
   在 `mod.rs:2026` 调）——turn_log 条目仍在内存，只是不可达。QA 观察到的
   「6 个回合历史全丢」是**不可达**而非销毁。
5. **回合错误条目已落账**：`mod.rs:1150-1156` 对任何 Error 事件（含 terminal）
   在拆除前合成 `TurnEntry::error`（failure_class=generic），driver 的升级原因
   文案（"agent hung (no activity for 5m; 3 cancels failed)"，
   `sebas-acp/src/claude/driver.rs:542`）随条目携带。升级击杀链完整：
   `driver.rs:534-547`（阶梯）→ `driver.rs:780-791`（terminal 事件）→
   `acp_events.rs:98`（消费臂）。

**修复取向（task 1.2）**：terminal 臂把 `remove_by_session` + `publish_removed`
换成「退役为记录」——映射 Active→Dormant（同 sid，转录保持可寻址）、pending
显式清空并如实上报、命名来源（卡面 user_prompt）迁移进映射后丢弃卡态、
`publish_removed` 改 `publish_updated`。下一消息走既有 Dormant→resume 路径，
load 被拒时既有诚实回退（sebas-dk8.4）孵化全新子进程——「fresh session」
可观察语义不变。「移除映射」解释为移除**活跃绑定**（Active 形态）；列表行
作为记录以 Dormant 形态保留。

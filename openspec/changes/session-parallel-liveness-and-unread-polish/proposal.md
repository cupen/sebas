## Why

多会话在工作台里实际上不可并行使用：操作者向第二个会话发消息后，会话永远停在「排队中」，子进程从不拉起——项目↔会话本应是 1 对多、会话↔agent 子进程 1 对 1，如今只有第一个会话真正活看。同时未读徽标形同虚设：`session.updated` WS 帧把 `msg_count` 裁成 `session_id+status`（`sebas-webui/src/events.rs:31`），前端只能靠轮询兜底；且 rail 徽标用「段锚」而 transcript 已读缝用「时间戳锚」，两条锚线不同源，徽标增补断供。最后是 composer 与布局的三处视觉债：mode 选择框过宽且小写、工具行横向不对齐、浮岛间距过大。

## What Changes

- **会话并行活性（治并发）**：诊断并修复「首条消息入队后 spawn 永不发生/永不激活」的路径（`Out::WebSpawn` → `handle_web_spawn` → `acp_spawn_and_activate`），使 N 个会话可同时拥有活跃子进程（仅受既有 dispatch 容量约束）；任何会话的 spawn/激活失败 SHALL 以 typed 状态就地呈现并允许重试，不再把消息无限期扣在 staging 队列里。
- **未读徽标接通 + 会话相位 wire 下发**：`session.updated` 帧重构为 `{ session_id, status_slug, turn_engaged, msg_count, pending }`——旧 `status` 字段删除、四键总是携带（不玩「只在 true 时上 wire」的兼容保留），所有消费端同随 binary 发布同步重构。前端 rail 徽标、transcript 已读缝、聚焦写锚统一到「段计数」一条锚线；徽标未读时高亮增强（行底色加重 + 数字醒目）。后端生命周期 FSM（SEED→WORKING→DONE/FAILED/Dormant）每个相位 flip 都即时 `publish_updated`（`sebas-dispatch/src/engine/mod.rs:1069-1076`），投影层 `SessionStatus::derive`（`sebas-webui/src/models.rs:30-56`）已把 (MappingState, phase) 翻成操作者能懂的七个英文词（starting/queued/working/done/failed/waiting/dormant），但 WS 帧（`sebas-webui/src/events.rs:31`）只透 `session_id+status`，把这份精心算好的相位裁剩成两个字串。让 WS 帧按 flip 下发完整相位 + 占用布尔 + 计数 + 排队栈，开始/结束/泊车等每个生命周期节点前端即刻可见；bind 所有消费路径（rail 圆点、composer 提交控件、徽标）到同一份帧形状，删除 `status_slug==='working'` 之类的回退猜测分支。
- **提交形态可分辨**：composer 提交控件 SHALL 区分「子进程启动中」与「在跑回合的排队」，不再让启动等待伪装成排队提交。
- **Composer/布局视觉收敛 + mode 默认显式 ask**：mode 选择框宽度收紧（max-content + ~110px cap）、选项 Title Case（Ask/Edit/Allow/Auto）；**mode 缺省从「wire 上省略的 None」改为显式 `"ask"`**——控制面词表的缺省就是 ask（危险/执行类操作停下来逐次问、等批准，`sebas-node-link` `SessionMode::#[default] Ask` 的领域语义），此前的「UI 强行显示 + wire 仍空」伪方案被操作者否掉；**不留任何向后兼容层**（操作者拍板）：`desired_mode` 在 wire/内存模型中改必选 `String`（不再是 `Option`),DB 列同步改造，core 启动时对存量 `sebas.db` 一次性迁移 `desired_mode IS NULL → 'ask'`（幂等）,DB/内存/wire/UI 四层同一份字符串；创建会话 `POST /api/sessions` 缺省时服务端落 `desired_mode="ask"`、创建对话框预填 Ask、composer 真源值渲染（不再有空态）、0-turn 占位行真源即为 ask；每词到执行体的映射确定性——claude 后端四词全真映射（ask→default、edit→acceptEdits、allow/auto→bypassPermissions，spawn flag + 运行时 SetMode 双路）；通用 ACP 与 native 现行如实报「不支持 mode」，不假装生效。composer 底部工具行改稳定网格基线对齐；rail|主区、舞台|输入框浮岛间距从 space-3 收到 space-2，舞台/composer 列 padding 统一 space-2。
- **状态层级收敛：会话状态只挂 rail 一处的彩色圆点**：现状里「会话状态」同时在多处表达（rail 行首 `session-dot[data-status]`、transcript 顶部 session-head 卡片的**`<sebas-status-badge>` 文字徽标（右上角"Queued"的来源，`dashboard.ts:1122`）**、session-head 的 `data-status` 左边框、project-header 右上角 `X sessions · active/idle` + focused-link 内的第二枚 status-badge（`dashboard.ts:914`）），后几处消息密度低且会让操作者误读为「当前会话的状态指示」。本期把会话状态**收敛到 rail 行首彩色圆点一处**（颜色 token 沿用 `--sebas-status-*`，会话行的 queued/working/starting/waiting/failed/done/dormant 7 态不动语义）；session-head 卡片删掉 `sebas-status-badge` 挂载与 `data-status` 左边框（保留 chat/model/mode/actions 等真正的会话身份与操作信息）；project-header 右上角删除 `X sessions · active/idle` 徽标与 focused-link 内的 status-badge 复件（保留 chat_id 锚点链）。消息层面的"排队"原本就在 pending-stack（composer 上方），挪到哪都不合适——给它搬家不做，只是**核对会话头/会话卡/会话行任何一处不再把"排队"挂在状态栏上**（它只属于消息、不属于会话）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `session-lifecycle`：「Lazy spawn on first message」——spawn 指令的发出与激活 SHALL 不会被其他会话的在跑回合串行阻塞；spawn 失败进入可重试失败态而非无限滞留。
- `session-unread-badge`：「Chat message counting」「Unread badge on session rows」——计数随 `session.updated` 帧下发；徽标高亮增强；锚统一为段计数。
- `live-turn-stream`：「Streaming respects the read anchor」——流式已读推进与 rail 徽标共用段锚。
- `agent-workbench`：「Composer toolbar composition」修改 + 新增「Spawn liveness is visible at the composer」——mode 选择框紧凑化与 Title Case、**mode 缺省显式ask**（协议与 UI 同对齐，内存模型 `Option` 废、SQLite 一次性迁移 null→'ask'，不做「wire 空 + UI 硬显示」伪方案也不做读侧投影）、工具行网格对齐、提交形态区分启动/排队、浮岛间距收敛。
- `agent-workbench`（状态层级新增段）：「Session status lives in one place」——会话状态只挂 rail 行首彩色圆点（7 态色 token 沿用），session-head 卡片的 `data-status` 左边框下线，project-header 的 sessions/active·idle 徽标下线；"排队"只属于消息维度（pending-stack），不作为会话级状态再出现。

## Impact

- 后端：`sebas-webui/src/events.rs`（WS 帧加 `msg_count`）、`sebas-dispatch/src/engine/mod.rs` + `src/dispatch.rs`（spawn 链路诊断修复）、`sebas-acp`（若根因在驱动握手串行）。
- 前端：`workbench-composer.ts`（mode 选择框、工具行、启动/排队形态）、`project-rail.ts` + `unread-cursor.ts`（徽标高亮、段锚统一）、`transcript-view.ts`（已读缝写段锚）、`app-shell.ts` / `dashboard.ts`（间距 token 收敛）。
- 测试：e2e 双会话并行用例、WS 帧 wire 断言、composer 形态与间距快照断言。

## Non-goals

- 会话改名（rename）功能不做——占位回退名可分性由并行修复顺带解决。
- transcript 渲染管线（memo/keyed repeat/时间线单卡）归 `fix-webui-streaming-liveness`，不碰。
- WORKING 停滞看门狗（`turn_stall_timeout`）归 `fix-pending-queue-liveness`，不碰。
- 不新增跨浏览器服务端已读存储（锚仍 per-browser）。
- 远端节点会话的 spawn 并行语义本期仅如实遵循现状，不做远端并发优化。

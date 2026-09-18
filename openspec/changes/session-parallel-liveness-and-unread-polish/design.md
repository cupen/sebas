# Design: session-parallel-liveness-and-unread-polish

## Context

四个症状，三个互不相同的病灶：

1. **并行活性**：会话键唯一（`ChannelKey::web_new` = `web-{纳秒}-{seq}`，`sebas-channels/src/key.rs:86`），rail 直接投影 `/api/sessions`，后端不存在「同 key」路径——「多个会话 id 一样」的观感与「只有 1 个会话能跑」的真因都指向 spawn 链路：`web_send_message` 对 0-turn 占位走 `TextRoute::SpawnNew` → `publish_created` + `emit(Out::WebSpawn)`，`dispatch_out_without_feishu`（`src/dispatch.rs:41`）同步 `await handle_web_spawn → acp_spawn_and_activate`（含子进程握手 + 首轮）。若出站泵串行处理 `Out` 且握手的入站事件由同一条泵消费，第二个 `WebSpawn` 要么排队等第一个激活、要么与事件消费互等；操作者观感即「第二个会话的消息变成排队中、永不启动」。前端 `dashboard.willUpdate` 有 fire-and-forget `activateSession`（聚焦即拉起），但失败被 `.catch(() => undefined)` 吞掉，`SpawnFailed` 原因无处呈现。
2. **未读断供**：`SessionEvent::Updated` 本就携带全量 `SessionInfo`（含 `msg_count`），但 webui 转发时把帧裁成 `{session_id, status}`（`sebas-webui/src/events.rs:31`）——数据在车上、下不了车。锚线分叉：rail 徽标读 `anchor_count`（段），transcript 已读缝与流式推进写 `seenTs`（时间戳），`writeSeen` 的 `anchorCount` 参数大多数调用点不传。
3. **Composer/布局**：mode 用 `wa-select` 默认宽度跟随容器；选项字面量小写；`.composer-bottom` 左右两组件用 flex + `margin-left:auto`，不同控件本征高度不同即错位；间距三层叠加（app-shell `margin: space-3` + dashboard 列 padding `space-2` + nav `margin: space-3`），主规格「Composer toolbar composition」仍写 mode 在会话头——spec 与代码早已漂移（mode 已迁入 composer 底沿）。

避让边界：transcript 渲染管线归 `fix-webui-streaming-liveness`；WORKING 停滞看门狗与 `turn_engaged` 字段归 `fix-pending-queue-liveness`（其 `turn_engaged` 覆盖「spawn 窗口」，正是本 change 要在前端拆分为「启动中 vs 排队」的数据源之一）。

## Goals / Non-Goals

- Goals：spawn 链路并发修复 + 失败可重试可见；`session.updated` 帧带 `msg_count`；锚统一为段计数一条线；徽标高亮增强；composer 工具行紧凑对齐 + mode Title Case；浮岛间距收敛。
- Non-Goals：见 proposal（rename、transcript 管线、看门狗、服务端已读存储、远端并发优化）。

## Decisions

### D1 spawn 并发：根因已定位在出站泵同步等待，修复点 = 候选 A

诊断结论已由代码结构证明（无须 e2e 反证再定根因）：

- 出站泵是**单任务串行**：`src/run.rs:231-245` 一个 `tokio::spawn(async move { while let Some(out) = out_rx.recv().await { dispatch_out_without_feishu(...).await } })`；
- `dispatch_out_without_feishu`（`src/dispatch.rs:41-60`）对 `Out::WebSpawn`/`Out::SpawnResume` 同步 `await handle_web_spawn → acp_spawn_and_activate`（`src/session_boot.rs:116-188`），后者含子进程拉起 + ACP 握手，最长 `startup_timeout`（默认 30s，`src/config.rs:228-230`）；
- 同一个 mpsc（buffer=256，`src/config.rs:367-369`）上所有其他 key 的 `SendAcp`/`Spawn`/`Cancel` 全被这一次 `await` 串行。第二个会话的 spawn 指令只能等第一个激活完成才轮到派发，操作者观感就是「切过去发消息卡在排队」。

修复方案 = **候选 A（唯一采纳）**：`handle_web_spawn`/`handle_spawn_resume` 不再在出站泵任务内同步 await——`dispatch_out_without_feishu` 在 `WebSpawn`/`SpawnResume` 分支里把 spawn + activate 投递给**新建的独立任务**（`tokio::spawn`）后立刻 `Ok(())` 返回；泵继续消费队列。激活后的 `activate`/drain 本就由事件路径回调（`SessionMap` 状态翻转由 `publish_updated` 驱动），无序化风险点在 `fail_spawn` 与 `publish_updated` 的次序，由 spawn 任务内顺序保证。`SendAcp`/`Cancel` 等廉价指令仍在泵内同步（`mgr.send` 本身是 O(1) 投递不阻塞）。

- 候选 B（ACP manager 全局锁）——否：`SessionManager::send` 只投递不等回合完成（`sebas-acp/src/claude/manager.rs:219`），ACP manager 无跨会话锁。
- 候选 C（前端 activate 链）——否：前端激活只是触发路径，串行化源头在出站泵。
- 被否：接受并发=1、给排队上标签（操作者拍板否掉——治标）。

失败可重试：`SpawnFailed` 映射已存在；补齐「下一消息重试」已在 `route_text`（state.rs:485-493），缺的是 wire 呈现——`SessionRow` 带 `phase`/失败原因（现有 `SessionInfo` 字段透传），composer 侧聚焦失败会话时就地呈现原因。

### D2 wire：`session.updated` 帧重构为「相位事实」帧，只加不保留旧键

`WebUiEvent::SessionUpdated` 重构为 `{ session_id, status_slug, turn_engaged, msg_count, pending }`——操作者拍板"不考虑向后兼容，按最好效果做"。旧 `status: String` 字段**删除**（不再保留给人读的冗余 label——同一信息 `status_slug` 已携）；所有"serde 缺省 / 只在 true 时插键"的兼容保留**取消**，所有字段每次帧必带：

- **`session_id`**: key 标识（沿用）；
- **`status_slug`**: 七词相位 `starting | queued | working | done | failed | waiting | dormant`（`SessionStatus::derive` 投影的 operator-facing 单词，**不再派生 label**）；
- **`turn_engaged`**: 布尔，总是带（`true`/`false`)，不再"只在 true 时上 wire";
- **`msg_count`**: u64，总是带；
- **`pending`**: 原石 `PendingSubmission[]`（投递序），总是带——pending 本来就是 `SessionInfo` 上的字段，WS 帧不发等于让"排队表单"依赖详情轮询。

`session.created` 帧同样一次性发全四字段（新建占位 `status_slug: "starting" / turn_engaged: true / msg_count: 0 / pending: []`)，与 updated 帧同形。

效果：前端所有需要"这个会话当前状态"的路径**只消费 WS 帧 + 初次打开一次 HTTP 详情**；`dashboard.ts:867-870` 的 `turn_engaged` 回退链**删除**（不再保留"键缺省 → 回退 `status_slug==='working'`"的兼容分支）；`SessionRow` 快照 HTTP 响应结构同步重构（`models.rs` `SessionRow` 沿用同字段名集），与 WS 帧共用投影函数。

后端生命周期 FSM 已就绪（`sebas-dispatch/src/engine/mod.rs:1050-1076`），每次 flip 都 `publish_updated`——`publish_updated` 处把帧构造好塞 WS 即可，无引擎层改动。

被否：a) 保留旧 `status` 字段以防其他消费端——grep `SessionUpdated` 前端消费现状只 match status 字符串相等判断，全部由 status_slug 取代；保留则让前端在两个字段里二选一 fuzz 状态，违背"一处真相";b) `turn_engaged` 只在 true 时上 wire——省一个字节，代价是前端需要多一个分支处理 undefined，而布尔本身就是语言原生，没有这个减法的必要；c) 面向未来自破如下「每次帧必带所有字段」的约束——后续 change 若要加键，先加后双方协议一致再移除旧键。

### D3 锚统一：单键单字段段锚（seen_ts 废弃）

`unread-cursor` 存储结构**简化为只存 `anchor_count`**（u64，每会话一个），`seen_ts` 字段从读写路径**删除**——不再有"兼容旧值"的字段附着；存量 localStorage 里含 `seen_ts` 的老 JSON 对象在第一次写入时按"无 anchor"对待（读为 fully-read、不迁移），此后被覆写成纯 `{anchor_count}`。`writeSeen` 全部调用点（transcript 读到底、流式贴底推进）显式传当前 `msg_count`;`unreadCount` 仅依赖 `anchor_count`。`live-turn-stream` 的流式推进与聚焦写锚调同一函数。被否：时间戳锚换算段数（需查询历史条目，复杂且脆）；"保留 seen_ts 以防后悔"——向后兼容语义已废（操作者拍板）。

### D4 徽标高亮：accent 底数字 + 行底色 tint

未读行加背景强调（`--sebas-accent-soft` 同族 tint）+ 现有数字徽标提高对比（accent-strong 底 / accent-ink 字已具备，主要补行级强调与字号/字重微调）。不引入新色 token；`prefers-contrast` 场景靠既有焦点环兜底。被否：仅红点方案（用户拍板否——存在感不足）。

### D5 composer 工具行：`wa-select` 紧凑化 + 网格对齐

mode 下拉：`--wa-form-control-width: max-content` + `max-width: 110px`，选项字面量 Title Case（Ask/Edit/Allow/Auto），change 处理器继续发小写 wire 值（`setSessionMode` 词表不变，后端 `SESSION_MODES` 白名单只收小写，`sebas-webui/src/api.rs:1015`）。工具行：`.composer-bottom` 改 `display: grid; grid-template-columns: 1fr auto; align-items: center;`（或保持 flex 但两组件 `align-items: baseline`）——择一在实现时按 WA 组件本征高度实测取稳者。spec 锚定「共享基线」，实现自由度留给 tasks。

### D5b mode 缺省显式 ask：协议与 UI 同一真源（无读侧兼容层）

问题：此前提案（D5b 初版）的「wire 保持 None + UI 强行显示 Ask」是伪方案——显示与协议各说各话，操作者否掉。本期取**显式 ask 贯穿协议**，让四个词都成为控制面一等值；**不为存量数据保留读侧投影**（操作者拍板：不为兼容留债）。

协议对齐：
1. **存储语义改必选**：`desired_mode` 在 wire/内存模型中由 `Option<String>` 改为 `String`（DB 列同义改造），新会话创建缺省时服务端落 `"ask"`，再无 None 路径；
2. **存量 SQLite 迁移**：core 启动时对 `sebas.db` 做一次性迁移，把已存 `desired_mode IS NULL` 的会话行 `UPDATE` 为 `'ask'`；迁移幂等、与既有 sqlite 迁移路径同挂点，不做读侧投影；
3. **创建对话框预填** `mode = "ask"`（真源如此），wire 无条件发送；
4. **composer `currentMode` 始终从详情真读**，不再有「空态选择器」；0-turn 占位行真源同样是 `"ask"`（新建立即落值）；
5. **文档契约**：`api.rs` `CreateSessionRequest.mode`、`models.rs` `desired_mode`、`sebas-webui/src/session_backend.rs` 相关注释全部改为「`'ask' | 'edit' | 'allow' | 'auto'`，缺省 `'ask'`」，无「None 兼容」残句。

四词到执行体的确定性映射（每词都有真实效果，或如实报不支持）：
- **claude**（`sebas-acp/src/claude/driver.rs:864` `control_mode_to_permission_mode`）：ask→`PermissionMode::Default`（逐次询问）、edit→`AcceptEdits`、allow/auto→`BypassPermissions`。spawn 时 `control_mode_flag` 决定 argv `--permission-mode`（ask/default 不带 flag，因 CLI 默认即 default，语义等价）；运行时 `AcpCommand::SetMode` 显式下发 SDK `set_permission_mode`——切了真生效，首 prompt 前应用；
- **通用 ACP**（`sebas-acp/src/acp_driver/mod.rs:360`）：协议无 permission 词汇，如实回「当前 ACP agent 不支持，模式未变」——不假装；
- **native**（`src/agent_backend.rs` `set_session_mode`）：native 不承载 mode，如实报不可用——不假装。

为什么 ask 必须显式：
- 语义上 ask 是「每个受门控动作都要问」的确定性模式，不是「留给 agent 自己猜」;
- 执行上 claude 恰好 default==ask，但那是实现的巧合，不应让 UI 依赖「没传恰好对」——传给 driver 的值与显示的值必须同源；
- 审计上 `desired_mode="ask"` 是操作者能看到、能审计的姿态；None 不可见；
- **不做读侧投影**：让 DB 数据、内存模型、wire 协议、UI 四层对「ask」的表达是同一份字符串，不需要任何「此处 None 读成 ask」的特殊分支——这是操作者「最好的效果」的直接落地。

被否：a) **wire 保持 None + UI 硬显示 Ask**——操作者否（伪方案）；b) 沿用「不预填创建对话框、向后透传 None」——语义从未被消费，只是漏到 UI 冒泡成空选择器；c) 只把 DB 读侧归一化为 ask、不做迁移写——读侧投影是「兼容债务」，四层里说四种话；d) `desired_mode` 保持 Option 但「语义当 String」——类型签名撒谎；e) native/通用 ACP 为凑四词假映射到其他原生 ACL——不假装，保留如实上报路径。

### D6 间距：token 收敛不新增 token

app-shell `main` margin 与 nav margin 由 `--sebas-space-3`（12px）收至 `--sebas-space-2`（8px）；dashboard `.stage-col`/`.composer-col` padding 统一 `0 var(--sebas-space-2)`；`--divider-width` 保持 6px（拖拽把手可达）。快照断言用 CSS 变量值而非像素魔数。

### D6b 状态层级收敛：会话状态只挂 rail 圆点一处

现状多处重复承载「会话状态」：

1. **rail 行首** `session-dot[data-status]`（`project-rail.ts:270-276`）：7 个 slug（starting/queued/working/waiting/done/failed/dormant）共用 `--sebas-status-*` 颜色 token，这是**每行自己的状态、操作者读得最习惯的入口**；
2. **transcript 顶部 session-head 卡片 `<sebas-status-badge>`**（`dashboard.ts:1122`）：独立的文字徽标组件，把后端 `status_slug`/`status_label` 渲染成右上角可见的 "Queued"/"Working" 字样——这是操作者指认的"右上角 Queued"的来源；
3. **session-head 卡片 `data-status` 左边框**（`dashboard.ts:417-444`）：同一份 status 的第三种视觉重画；
4. **project-header 右上角** `X sessions · active/idle` 徽标（`dashboard.ts:897-903`）+ focused-link 内第二枚 `<sebas-status-badge>`（`dashboard.ts:914`）：项目级活跃度布尔 + 会话 slug 的第四处重复表达。

操作者反馈：右上角那段没有实际意义、queued/working/idle 该放到 rail 行首用色块表达；看到 "Queued" 字样出现在会话不该出现的位置。本期采纳：

- 保留 **rail `session-dot`** 为唯一状态承载，7 态色 token 与 slug 不重定义（`tokens.css:52-72`）；颜色已是用户期望的"黄/绿/灰等状态块"形式。
- 删 `dashboard.ts:1122` 的 session-head 内 `<sebas-status-badge>` 挂载；删 `.session-head[data-status='...']` 六条左边框色规则与模板里对应的 `data-status=${...}` 属性绑定（`dashboard.ts:417-444` 与其渲染处），session-head 卡片保留 chat / model / mode / actions 等真正属于"这张卡片"的信息。
- 删 project-header 的 `X sessions · active/idle` 徽标块（`dashboard.ts:897-903`），删 `:914` focused-link 内嵌的第二枚 status-badge（`focused-link` 保留 `chat_id` 锚点，不再复述 status）；保留 project 名、节点 chip、分支 pill——它们是真导航信息。
- **"排队"归位**：「消息排队」只活在 pending-stack（`agent-workbench/spec.md:609-651` 既有 Pending submissions stack，挂在 composer 上方），**不该作为会话级状态再出现**。本 change 核对实现里 `queued` slug 只用于 rail 行首圆点（与"回合在跑、这条消息已入队"的过渡相位一致），不在 session-head 的 status-badge / 左边框 / project-header / 任何会话级横幅上重复。
- **组件本体保留**：`<sebas-status-badge>` 自定义元素在 `components/status-badge.ts` 本体保留（`sessions.ts` 等旧视图仍用，`a11y.test.ts` 断言依赖）；本 change 仅把它从 dashboard 的两处挂载点撤出。若后续旧视图下线，组件可另行清理。

被否：a) 保留 project-header 徽标但缩小——信息密度没有本质变化，且"项目里有没有 active"不是 rail 读不出来（rail 相邻滑动一眼可见）；b) 保留 session-head 徽标 / 边框但弱化透明度——视觉上仍重复，违背"状态只挂一处"的收敛目标；c) rail 圆点旁加文字 slug 一并显示—— rail 行空间紧凑，且圆点+行底色对未读/等待已有组合表达，加文字反而是噪声；d) 顺手把 `sebas-status-badge` 组件整个下线——`sessions.ts` 等旧视图与 a11y 测试还在用，越出本 change 范围。

依赖：D6 间距收敛与本段正交，不涉及新增 token；仅从 dashboard.ts 删 CSS、模板分支与两处 `sebas-status-badge` 挂载，无后端改动。

### D7 规格勘误顺带

「Composer toolbar composition」主规格仍写「mode 在会话头」，delta 直接以代码现状（mode 在 composer 底沿左端）为基线改写——归档时一并修正主规格，不留两处矛盾。

## Risks / Trade-offs

- [spawn 任务化后错误次序乱] → 任务内保持「spawn → activate/fail」串行；e2e 断言 fail_spawn 与 transcript 错误条目次序。
- [WS 帧形状变更影响所有消费端] → 本 change **拍板不留向后兼容层**：`status` 字段删除、`turn_engaged`/`status_slug`/`msg_count`/`pending` 总是携带、frontend 同步重构消费路径——同一 binary 发 core+webui+frontend，wire 协议与前端版本同进同退，不存在旧前端配新 core 的组合。若未来 wire 再改形状，由后续 change 起新的 frame type 而不是在本协议里加兼容键。
- [段锚 `seen_ts` 废弃后旧浏览器 localStorage 里的 `{seen_ts, anchor_count?}` 形态] → 首次读取时按"无 anchor"对待（读为 fully-read），随后被纯 `{anchor_count}` 覆写；不做字段迁移、不保留 seen_ts 读路径。
- [间距收敛触碰 split-persist 记忆宽度] → 只动 margin/padding token，不动 clamp 边界与 localStorage 键。
- [D5 两种对齐实现取其一的实测] → tasks 里留断言驱动的择一步骤，spec 锚定结果（共享基线）不锚手段。
- [D5b 显式 ask 落地后存量 `desired_mode: null` 数据行的语义变更] → **一次性 SQLite 迁移**把 null 写为 `'ask'`，DB/内存/wire/UI 四层形为同一份字符串，无读侧投影；迁移幂等、与既有 sqlite 迁移路径同挂点。若某行数据在迁移前被旧 core 读到，旧 core 会把 None 透传成空选择器——这是可接受的发布窗口（core/webui 同 binary 发布）。
- [D5b 通用 ACP/native 会话的 mode 仍是「存了但不下发」] → 如实报不支持的既有路径保留，操作者在 composer 切 mode 时会看到「当前 agent 不支持 mode」的 transient 提示或 typed 错误，不假装生效。
- [D6b 删 project-header 徽标 / session-head 边框后，对"项目总在跑数/当前会话状态"阅读习惯的回归风险] → rail 行首圆点 7 态色已承载每会话的相位，项目级活跃度可扫 rail 一眼得出（这正是期望操作流）；删的是重复且低密度的视图层，不是信息源。若实现中 / 使用中发现某些旅程（如只看 stage 不看 rail）真依赖 session-head 边框色，在 tasks 4.2 的沙箱核验里如实加回来（作为 visual 而非 status 的提示），不在 spec 里承诺永久删除。

## Migration Plan

**一次性 SQLite 迁移（D5b）**：core 启动时对 `sebas.db` 执行 `UPDATE sessions SET desired_mode = 'ask' WHERE desired_mode IS NULL`（幂等，与既有 sqlite 迁移路径同挂点——`sebas-webui/src/db.rs` 或等价迁移表）。迁移与 wire 模型重构（`desired_mode: String`）随同一 binary 发布；发布窗口内读到旧 None 值的旧 core/旧前端会有降级行为（空选择器），发布闭窗后路径只剩"`ask` 是唯一缺省"一条。

**WS 帧形状与前端同进同退**：core/webui/frontend 同 binary 发布，wire 协议无独立版本号；回滚 = 回退 binary（SQLite 迁移幂等，可回滚后再次上线）。

**localStorage `unread-cursor`**:seen_ts 字段废弃，存量数据读为"无 anchor"（fully-read）并被后续写入覆写成纯 `{anchor_count}`；无主动清理。

## Open Questions

- spawn 任务化后「激活回调与下一轮 `publish_updated` 的次序」在 fake-claude e2e 中是否出现肉眼可见的时序闪烁——若出现，按 D1 任务内的串行保证收紧；不预设。
- D5 工具行对齐是 grid 还是 baseline-flex——由实现的 composer 单测在两种实现中选视觉更稳者（spec 只锚「共享基线」）。

## Context

动机见 `proposal.md`。本设计要处理的现状与约束：

- **两条队列，语义不同**：`MappingState::Spawning { pending: Vec<String> }`（`sebas-dispatch/src/state.rs:334-360`，上限 `MAX_PENDING = 16`，激活时 `pending.join("\n")` 合并成一条首 prompt，见 `src/session_boot.rs:322-332`）与 `turn_queue: HashMap<ChannelKey, VecDeque<QueuedTurn>>`（`state.rs:610-636`，逐条 FIFO + `/btw` 优先插队）。两者都没有 id。
- **两条提交路径，行为不一致**：Feishu 走 `inbound::continue_session`（`inbound.rs:697-705`，WORKING → `enqueue_turn` + ⏳）；WebUI 走 `engine::web_send_message` 的 `TextRoute::Continue` 分支（`engine/mod.rs:1198-1210`，**无 in-flight 检查**，提交即 `transcript_push(TurnEntry::prompt)`）。
- **transcript 是追加序扁平日志**：`transcript_push` 强制 `position = log.len()`（`engine/mod.rs:473-478`），所以「提交时就写 prompt」会让在跑的回合输出排到你的插话之后。
- **硬约束**：ACP 同一会话同时只能有一个 prompt 在飞（`src/session_boot.rs:322` 注释原话），所以两条队列都不能改成并发投递。
- **两种部署形态**：in-process（webui 内嵌）与 detached（webui 经 core session channel 驱动），队列的观察与管理两面都必须两种形态同真相。
- **生命周期**：`turn_log` 与 `turn_queue` 都在内存（`engine/mod.rs:256`；`session-persistence` 明确运行态不持久化）。

## Goals / Non-Goals

**Goals：**

- 每一次「已接受但未开始」的提交都可观察：有稳定 id、文本、顺序、处置方式。
- 两条队列的处置语义**保持不变**（staging = 合并，turn = 按序），只在观察层统一。
- WebUI 与 Feishu 走**同一条**排队判定，不再各写一份。
- 队列满、会话终结这类「消息没了」的事件一律有出口，不靠日志。

**Non-Goals：**

- 不改 staging 的合并语义（改它等于改 Feishu 的「连发合并」行为，BREAKING 且与 ACP 一条在飞规则相左）。
- 不给 turn 队列加容量上限（现状无上限；本 change 只把 staging 的既有上限变得可见）。
- 不做队列持久化（与 transcript 同生命周期，见 Non-goals in `proposal.md`）。
- 不动 transcript 的 chunk 粒度、不放行 `prompt` 条目给 SPA（下一个 change `workbench-conversation-view`）。

## Decisions

### D1：两条队列保留，观察层统一成 pending submission

- **选择**：新增视图概念 `PendingSubmission { id, text, position, disposition, priority }`，`disposition ∈ {staging, turn}`。`staging` = spawn 窗口内、将被合并进首条消息；`turn` = 流式期间排队、将作为独立回合按序执行。观察面只有这一个列表，处置差异用 `disposition` 表达。
- **备选**：把 staging 也改成逐条执行、只留一条队列——否决（改 Feishu 语义，且合并正是 ACP 一条在飞规则下「会话还不存在」时的正确解法）；只暴露 turn 队列——否决（spawn 握手期间仍有静默窗口，与本次目的相反）。

### D2：id 由 core 分配，per-session 单调计数，不持久化

- **选择**：`SessionMap` 为每个 key 持一个单调计数器，入队/暂存时分配 `u64` id；条目 drain（开轮或激活合并）即失效。id 只在「仍在队列里」时有意义。
- **理由**：`disposition` + 单调 id 让「操作一个已经开始跑的条目」可以被**确定性**识别（id 不在 pending 列表里 ≠ 未知 id，需按「已开始」拒绝），而不必比对文本。
- **备选**：数组下标——否决（reorder 之后下标指向漂移，删除会误伤）；UUID——否决（更长、无收益、跨重启也不需要稳定）。

### D3：in-flight 判定收敛到一处共享入口

- **选择**：抽出 engine 内的共享 `submit_turn(key, session_id, prompt, priority, origin)`：先判 card state 是否 WORKING → 是则 `enqueue_turn` 并回「排队」；否则 `seed_card` + `SendAcp::ContinueSession`。Feishu 的 `continue_session` 与 web 的 `TextRoute::Continue` 分支都改调它。
- **理由**：两条路径各写一份正是本次 bug 的成因；共享入口让「所有通道都受 back-pressure」成为结构保证，而不是两份实现的口头约定。
- **备选**：在 `route_text` 内判 WORKING——否决（`route_text` 只看映射状态，不持有 card state；把 FSM 状态混进路由层会让 Spawning/Dormant 分支更难懂）。

### D4：prompt 落 transcript 的时机 = 开轮

- **选择**：删除 web 路径提交时的 `transcript_push(TurnEntry::prompt)`；prompt 条目一律由 `seed_card` 在开轮时写入（Feishu 路径既有行为）。入队时改为发出一次 `PublishUpdated`，让堆叠区即时可见。
- **理由**：transcript 是追加序的，只有「开轮才写」才能保证一个回合的输出不被后提交的插话切开；spec 里「queued submission enters the transcript only when it starts」即此。
- **副作用**：`last_active_unix` 仍应随提交更新（recent 排序不变），所以 `publish_updated` 保留，只是不再伴随 transcript 写入。

### D5：溢出与终结都有出口

- **选择**：`route_text` 的满队列分支不再返回普通的 `Enqueued`，改为携带拒绝原因的变体；web 路径映射为 4xx（409），Feishu 路径发一条提示消息。会话终结（terminal error / close）时，core 在移除映射**之前**发出一条 `PendingDropped` 事件（携带被丢弃条目的 id + 文本），并让 `close` 响应带 `discarded_pending: N`。
- **理由**：一次提交要么被执行、要么被告知没执行；两种结局都不允许只留日志。
- **备选**：只在 UI 侧推断（"stack 变短了"）——否决（无法区分「开跑了」与「被丢了」，正是要消灭的歧义）。

### D6：观察面在 `SessionInfo` 上带全量 pending

- **选择**：`SessionInfo` 增 `pending: Vec<PendingSubmissionView>`；快照与每次会话事件都携带**全量**列表（上限 16 + 用户手打条数，量级极小）。
- **理由**：全量语义让客户端无需增量合并逻辑，天然幂等，且删除/重排/开跑/丢弃四种变化用同一条通道表达。
- **备选**：细粒度事件（`PendingAdded`/`PendingRemoved`…）——否决（客户端要维护序并发合并，收益不抵复杂度）。

### D7：管理操作是两条路径共用的 typed 拒绝

- **选择**：`remove_pending(id)` 与 `move_pending(id, to_index)`，在 session map 的单写锁内完成，与 `activate`/`drain_queue_if_terminal` 天然互斥。拒绝原因类型化：`Unknown` / `AlreadyStarted` / `PriorityConflict`（不能越过优先项）/ `OutOfRange`。in-process 直通 `SessionMap`；detached 走 core channel 新增的对应 op。
- **理由**：用户要「可删除 + 可拖拽排序」，而删除与 drain 存在竞态；把判定放在锁内 + 类型化拒绝，UI 才能如实区分「没这条」「已经跑了」「不能插到 /btw 前面」。
- **备选**：整队列替换写入（PUT 全量）——否决（并发多客户端会互相覆盖，且把「已经跑了」的判定挪到客户端）。

### D8：前端堆叠区 = 新组件 + 乐观对账 + 键盘可达

- **选择**：新增 `<sebas-pending-stack>` 渲染在 composer 上方，数据源为聚焦会话 payload 的 `pending`。拖拽用原生 HTML5 DnD（不引依赖），并额外提供键盘可达的「上移/下移」操作（仓库有 a11y 门禁，纯拖拽不可达）。操作后乐观更新，随即以服务端返回的 pending 列表对账；`AlreadyStarted` 视为「已开跑」，静默刷新而非弹错。
- **理由**：队列变化与 WS 推送、refetch 周期并存，乐观态必须有对账出口，否则会与服务端真相漂移。
- **备选**：引入 dnd 库——否决（新增依赖，而交互只有「组内上下移」）。

### D9：术语先改 glossary

- **选择**：`openspec/glossary.md` 新增 **pending submission（待生效提交）**，两种处置 **staging（并入首条消息）** 与 **queued turn（按序执行的待执行回合）**，并显式指出与 `SessionStatus::Queued`（子进程尚未产出，`sebas-webui/src/models.rs:17`）**不是一回事**。spec 用词以此为据。
- **理由**：仓库规矩是术语变化先改 glossary；`queued` 一词已被占用，不改会造成新的二义。

## Risks / Trade-offs

- [**删除/重排与 drain 竞态**：操作落在条目已经开始之后] → 全部判定与变更在 session map 单写锁内完成；`AlreadyStarted` 类型化返回，UI 静默对账，绝不回滚一个已经在跑的回合。
- [**`/btw` 优先项与拖拽语义冲突**：用户拖某条越过优先项] → `PriorityConflict` 拒绝；前端把优先项渲染为不可拖拽，并在拖拽落点非法时不做乐观更新（先判后动）。
- [**乐观态与服务端漂移**：快速连点删除/拖拽] → 每次操作以服务端返回为准重建列表；操作在途时禁用该条目的再次拖拽。
- [**事件风暴**：每次 pending 变化都触发前端 refetch 全量 detail] → pending 全量列表很小，复用既有 WS → refetch 节流即可；不做额外增量通道。
- [**staging 合并在观测上仍是一次「消失」**：多条 staging 合并成一条 prompt 后，堆叠区少掉 N 条、transcript 只多一条] → 合并是既定语义，UI 必须在合并前就用文案说清「将并入首条消息」，避免把合并读成丢失。
- [**队列满时 Feishu 与 WebUI 体验不对称**（一个收提示消息、一个收 4xx）] → 两者都「被明确告知」，形式按通道能力走；spec 只要求「在提交面可见地拒绝」。

## Migration Plan

1. **glossary 先行**：加 pending submission / staging / queued turn 词条（无代码依赖）。
2. **core 语义**：共享 `submit_turn` + prompt 落 transcript 时机（D3/D4）——此时 transcript 归属已经变正确，但队列仍不可见。
3. **观察/驱动面**：`PendingSubmissionView` 进 `SessionInfo` 与事件；`remove_pending`/`move_pending` 落 `SessionMap` 并暴露到 core channel（D2/D6/D7）。
4. **出口**：溢出拒绝 + `PendingDropped` + `close` 的 `discarded_pending`（D5）。
5. **webui API + 前端堆叠区**（D8）。
6. **e2e**：进程级先证 core 语义（忙中入队、开轮才落 transcript、remove/reorder、溢出拒绝、终结标注），再证浏览器侧堆叠区旅程。

**回滚**：本 change 不涉及持久化格式（队列在内存），回滚即回到旧二进制；`staging` 的合并与 `turn` 的逐条 drain 语义未变，唯一的对外行为变化是「满队列由静默丢改为可见拒绝」，回滚后恢复静默。

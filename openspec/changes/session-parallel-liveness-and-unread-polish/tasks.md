## 1. spawn 并发修复（session-lifecycle delta）

根因已在 design D1 中定调（出站泵同步 await 握手 `src/run.rs:231-245` + `src/dispatch.rs:41-60`），无须先跑诊断。

- [x] 1.1 `dispatch_out_without_feishu` 的 `Out::WebSpawn` / `Out::SpawnResume` 分支改为投递独立任务：用 `tokio::spawn` 新起一个 per-spawn 任务执行 `handle_web_spawn`→`acp_spawn_and_activate`（及复活路径），泵在本 key 的指令返回 `Ok(())` 后继续处理 mpsc 队列；任务内保持 spawn → activate/fail 串行顺序，失败经 `fail_spawn` 走既有事件通道，不引入新 wire
- [x] 1.2 双会话并行 e2e 用例（新）：`tests/testsuite_e2e_test.rs`，fake-claude 第一会话跑长回合，向第二 0-turn 占位会话发首条消息，断言第二子进程在 `startup_timeout` 内拉起并完成回合；修复前红 / 修复后绿
- [x] 1.3 spawn 失败 wire 透传：`SessionRow`/detail 携带失败态与原因（`SessionInfo` 既有字段透传，不新造状态机）；单测断言失败会话行可见原因
- [x] 1.4 `invoke testsuite-e2e` 全绿；出站泵/worker 所在 crate（core 层；按修复落点）与 `sebas-dispatch` 的单测断言「第一会话 WORKING 时第二 spawn 指令发出不被阻塞」
  - 备注（1.3/1.4 实施者）：单测落在根 crate `src/dispatch.rs::tests::web_spawn_instruction_is_not_blocked_by_a_stalled_handshake`（1.1 随 1.1 落地，本阶段实跑绿）；e2e（`tests/testsuite_e2e_test.rs::two_sessions_spawn_and_turn_concurrently`，--ignored）编译过、留 3c 阶段实跑。

## 2. 未读徽标接通（session-unread-badge + live-turn-stream delta）

- [x] 2.1 `sebas-webui/src/events.rs` WS 帧形状重构：`SessionUpdated` 改为 `{ session_id, status_slug, turn_engaged, msg_count, pending }`（**所有键每次帧必带，旧 `status` 字段删除，无兼容保留**，操作者拍板同 binary 发布不留旧 wire 用户）；`SessionCreated` 同形（初值 `status_slug:"starting" / turn_engaged:true / msg_count:0 / pending:[]`）；服务端填充+序列化单测（五键齐全，旧键不再出现）；`dashboard.ts:867-870` 的 `turn_engaged` 回退链（`?? status_slug === 'working'`）删除，只留「帧 / 详情」同形状真源
- [x] 2.2 `ws.ts` 类型重构与前端消费统一：`SessionUpdated`/`SessionCreated` wire 类型匹配 2.1 新形状（四键必带）；rail 行首圆点、徽标、composer 提交控件、QUEUED/STOP 形态全部从帧字段真读，不依赖 HTTP 详情轮询、不做任何字符串等值回退；回合开始 / 结束 / 泊车等每个 FSM flip 前端即时可见；`workbench-composer.test.ts`/rail 测试补帧驱动徽标断言与帧驱动排队形态断言
- [x] 2.3 锚统一：`unread-cursor` 存储简化为**单字段** `{anchor_count}`（u64，`seen_ts` 从读写路径删除）；存量 localStorage 含 `seen_ts` 的老 JSON 首次读取时按"无 anchor"对待（读为 fully-read、不迁移），随后被纯 `{anchor_count}` 覆写；`writeSeen` 全部调用点（transcript 读到底、流式贴底推进）显式传当前 `msg_count`；`unreadCount` 仅依赖 `anchor_count`；单测覆盖「流式推进 = 聚焦推进 = 手动读到底」三路写同一字段，且读含 `seen_ts` 老数据的兼容测试断言"按无 anchor 处理 + 首次写覆写"
- [x] 2.4 徽标高亮：未读行加行级强调（accent-soft 族 tint）+ 数字对比度微调；rail 快照/单测断言未读行与已读行可分辨
  - 备注（rebase 调和）：main 的走查打磨在 `transcript-view.onTurnAppend` 加了 `&& this.docVisible()` 守卫（后台 tab 不推进锚）与 `settleEmptyStreamAnchor()`（空流首交换建立锚），两者都写的是旧时间戳锚 API。rebase 后按段锚重写：`writeSeen()`（无参，写 `max(服务端段数, 本地已渲染段数)`）——行为意图不变（看得到的不算未读、后台不算、空流首交换建立水位），main 的 6 条时间戳锚用例同步改写到 `{anchor_count}` 语义。
- [x] 2.5 `invoke testsuite-acceptance` 全绿；`tests/acceptance/COVERAGE.md` 补并行会话与帧驱动徽标两行
  - 备注（3c 实跑）：`invoke testsuite-acceptance` 9 passed；COVERAGE.md 两行已补（簇①「会话并行 spawn 活性」+ 簇③「徽标与相位帧驱动」）。

## 3. Composer 与布局收敛（agent-workbench delta）

- [x] 3.1 mode 下拉紧凑化：max-content + 110px cap，选项 Title Case（Ask/Edit/Allow/Auto），change 仍发小写 wire 值；`workbench-composer.test.ts` 断言宽度约束与 wire 值
  - 备注（rebase 调和）：main 在本 change 之后合入了 `polish-workbench-walkthrough-ux` 4.1——下拉词汇收敛到共享 `MODE_OPTIONS`（创建弹窗与 composer 同源、带中文解释），并保留首项空值「默认（ask）」历史条目。4.1 更晚且有两侧同源测试钉死，故 rebase 后**词汇取 4.1**、Title Case 断言下线：3.1 的落点收敛为「下拉紧凑化（`--wa-form-control-width: max-content` + 110px cap）」+「change 仍发小写 wire 值」两项，D5b 的 `currentMode` 非空真源渲染由新增用例单独钉住。
- [x] 3.2 mode 缺省显式 ask（协议 + UI 真源 + SQLite 迁移，不做读侧投影）：
  - 备注（实施者）：**迁移挂点与 tasks 假设的偏差**——`desired_mode` 实际持久化在 state.json（`MappingDto`），不存在带 desired_mode 列的 SQLite sessions 表（`sebas.db` 的 session_map 只有 5 列）。一次性迁移落在 restore 反序列化点（`state.rs::deserialize_desired_mode`：旧文件 null/缺字段 → `'ask'`，幂等，restore 后 dump 不再写 null），这是 null 消失的唯一地点、先于任何投影；测试 `state.rs::legacy_null_desired_mode_migrates_to_ask_on_restore` 钉住。`desired_mode` Option→String 贯穿 Mapping/MappingDto/SessionInfo/SessionRow/detail/summary；节点链路 `RemoteSessionView`（冻结 wire 契约）保持 Option，远端投影点落缺省 ask。spawn 请求管道（`Out::WebSpawn.mode` 等 create-request 维度）保持 Option，webui API 层缺省已落 `'ask'` 恒以 Some 下发。
  - **`desired_mode` 在 wire / 内存模型中从 `Option<String>` 改必选 `String`**（`api.rs` `CreateSessionRequest.mode` / `SessionDetailResponse` / `SessionRow` 与 `session_backend.rs` 内部类型同步重构）；创建接口缺省时服务端落 `"ask"`；
  - **core 启动对 `sebas.db` 一次性迁移**：`UPDATE sessions SET desired_mode = 'ask' WHERE desired_mode IS NULL`，幂等；迁移挂点遵循既有 sqlite 迁移路径；
  - 创建对话框预填 `mode = "ask"`（提交字段无条件发送）；
  - composer `currentMode` 从详情真读（不再有 `?? null` 分支），不再有空态选择器；
  - 文档：`api.rs` / `models.rs` / `session_backend.rs` 相关注释统一改为「`'ask'|...|'auto'` 缺省 `'ask'`」，不留「None 兼容」残句；
  - 测试：`workbench-composer.test.ts` 断言「无空态、选项被正确标记」；`new-session-dialog.test.ts` 断言「打开选中 ask、wire 含 `mode: "ask"`」；API 测试断言「创建不带 mode 读数为 `"ask"`、迁移后存量 null 行变 `'ask'`」
- [x] 3.3 工具行共享基线：`.composer-bottom` 改 grid 或 baseline 对齐（按 D5 实测择稳者），不同本征高度控件同行居中；快照断言
- [x] 3.4 提交形态拆分：starting（子进程启动中、无在飞 turn）与 queued（在跑回合排队）两形态可分辨，starting 消费 `turn_engaged`/spawn 窗口事实；失败会话聚焦时就地呈现原因与重试入口；composer 测试补两形态与失败呈现断言
- [x] 3.5 间距收敛：app-shell `main`/nav margin `space-3`→`space-2`，`.stage-col`/`.composer-col` padding 统一 `space-2`，`--divider-width` 保持 6px；断言 CSS token 值与拖拽把手可达
- [x] 3.6 会话状态层级收敛（D6b）：
  - `dashboard.ts:1122` 删 session-head 卡片的 `<sebas-status-badge slug=… label=… glyph=…>` 挂载（这是"右上角 Queued"的直接源）；
  - `dashboard.ts` 删 `.session-head[data-status='starting|queued|working|done|failed|dormant']` 边框色规则及 `.session-head` 模板里 `data-status=${...}` 属性绑定；session-head 卡片保留 chat / node-tag / model / mode / actions，不做状态边框与状态徽标；
  - `dashboard.ts` 删 project-header 右上角 `X sessions` 计数与 `active/idle` 徽标块（含 `.active-dot` 样式与 `hasActive` 判定）；
  - `dashboard.ts:914`删 focused-link 内嵌的第二枚 `<sebas-status-badge>`（focused-link 保留 `chat_id` 锚点，不再复述状态 slug）;
  - **不新增** rail 行首圆点（`session-dot[data-status]` 已是唯一承载，7 态色 token 沿用）;
  - `<sebas-status-badge>` 组件本体保留（`sessions.ts` 等旧视图与 `a11y.test.ts` 仍在用），仅退出 dashboard 两处挂载；
  - 核对消息层"排队"只活在 pending-stack（composer 上方），rail 行 / session-head 卡 / project-header 任何一处都不再出现把"排队"格在会话身上的 UI 元素；
  - 测试：`dashboard.test.ts` 断言 session-head 卡片不再有 `sebas-status-badge`、不再有 `data-status` 属性；project-header 无 `sessions` 计数、无 `active|idle` 徽标、focused-link 内无 status-badge；rail 行首圆点仍在（快照/选择器断言）；`dashboard.test.ts:377` 现有断言行需换新表述（head 卡片里不再有 status-badge，改为断言 chat/model/mode/actions 仍在）
  - 备注（rebase 调和）：main 的走查打磨给 project-header 的计数/活跃度徽标做了中文翻译但未删除。本项按 D6b 收敛为**删除**（spec 是状态层级收敛的唯一出处），并连带清理只服务该块的死代码：`rowsForSelected()`、`allRows` 状态与其唯一的 `api.sessions()` 拉取（`refetch` 只剩 `projects.list` + `summary`，与 main 的 summary/detail 拆分口径一致）；session-head 侧 main 4.2 已把 `const ungated` 换成 `modeBadgeLabel`，该变量随之删除。

## 4. 收尾验证

- [x] 4.1 `rtk cargo test`（workspace）全绿；`pnpm vitest`（frontend）全绿
  - 备注（实施者）：受影响 crate（sebas-dispatch / sebas / sebas-webui / sebas-acp）两轮全绿；vitest 499/499 绿；`tsc --noEmit` 除 settings-modal.test.ts 两处**既有**断言签名报错（`toBe(x, msg)` 二参，本 change 未触碰该文件）外零错误。workspace 全量中发现两例与本 change 无关的既有问题：sebas-node `acp_body_e2e_test`（陈旧 fake-claude-cli 二进制掩盖的行为漂移，另一分支 135c383 已对齐用例）+ sebas-acp `permission_mode_gate` 假子进程时序偶发（隔离/连跑稳定绿）。
- [ ] 4.2 沙箱联调：`invoke testsuite-webui-sandbox` 起真实 UI，人工核验——双会话并行跑、未读徽标帧驱动更新与高亮、mode 下拉与工具行对齐、**新建会话 composer 即显示 Ask（无空态）、创建对话框预填 Ask、0-turn 占位行 composer 同样显示 Ask、存量 `desired_mode: null` 行经一次性迁移后读数为 Ask（在沙箱里构造 null 旧库验证迁移幂等）**、切 mode 到 native/ACP 会话时如实呈现「该执行体不支持 mode」而非假装生效、**WS 帧五键齐全（session_id/status_slug/turn_engaged/msg_count/pending）、无 status 字符串旧键、每个 FSM flip 都是离散的 WS 事件而不是轮询后出现**、**会话状态只在 rail 行首圆点一处表达（session-head 无 `sebas-status-badge` 文字徽标、无状态边框；project-header 无 sessions/active 徽标、focused-link 无 status-badge 副本）、"排队"只在 pending-stack 不出现在会话 chrome 上**、浮岛间距；浏览器套件 `invoke testsuite-webui-server` 旅程全绿
  - 备注（rebase 实测 + 旅程对齐）：浏览器套件首次实跑 73 例中 22 例红，逐条归因后**无一条来自双分支合并**；其中一处是本 change 自己的真实缺陷、其余是断言没跟上契约与上游文案，两者都已收口，套件回到全绿——
    - **修掉真实缺陷（本 change 自己引入）**：dashboard 的 composer 挂载点把 HTML 注释写进了标签的属性列表。浏览器在注释处即结束标签，其后 `.childStarting/.failureReason/.currentMode/.modeEditable/.hasTurns/.coreReachability/@composer-sent` 全部退化成文本子节点（页面上可见一串绑定文本）——mode 开关不渲染、提交后 dashboard 不刷新详情、starting/失败形态不显。注释已移出标签并留下警示。happy-dom/vitest 对同段标记宽容，只有真实浏览器能暴露——这正是把 4.2 留着不勾的价值。
    - **旅程对齐 D6b（状态只挂 rail 圆点）**：`FocusedSession.statusBadge` 从 session-head 徽标改为 rail 当前行的 `.session-dot[data-status]`，26 处断言收敛成 `expectStatus('done')`（内含项目行展开——会话行在项目折叠体里，不展开不在 DOM）；`createSession` 缺省把会话绑到沙箱场景项目（无项目会话不进 rail，没有状态面可言，也不符操作者真实用法）。两侧 `StatusSlug` 补 `spawn-failed`（1.3 行 slug）。
    - **旅程对齐折叠（懒渲染）**：`processFold/processItems` 改到 `div.process-fold/.process-item` + `aria-expanded`；工具结果文本的断言改走 `expectFoldedText()`——折叠体收起时不在 DOM，且定稿重分组会把条目折回收起态，所以「展开 → 看不到再展开」轮询到出现为止（一次展开不够：detached 形态下曾因此假红）。conversation 的过程折叠用例按新标记重写，permission 三例 + approval-detached 两例走轮询展开。
    - **旅程对齐上游文案与流程**：归档行点击 = 只读视图 + 显式恢复 + 确认弹窗（不再是即点即恢复）；Skills 删除弹窗正文中文；创建弹窗 native 不可用文案；空流提示中文「未聚焦任何会话」；mode 标签中文「放行」；`unread-badge` 锚断言改单字段 `anchor_count`。
    - **边界（如实记录）**：spawn 失败会话的项目路径未落定、行归不到任何项目，rail 里没有它的行——这类会话的可见面是工作台内显横幅，该用例改为在这一层面钉失败态（`spawn-failed`/`failed` 二者之一，不假装在跑）。
    - **顺带收紧断言侧竞速**：`bubbles()` 排除折叠体（`.fold-body`/`.item-body` 只在展开时渲染，混进来会让「气泡数」随折叠开合漂移）；dialog 空提交用例与 slash-commands 透传用例先等首轮气泡落盘再取基线——相位断言要展开项目行，而展开 = 同时选中该项目（工作台随 rail-select 重绘一次），基线抓在这一瞬会读成 0。该副作用已写进 helper 文档。
    - 人工沙箱核验（`invoke testsuite-webui-sandbox` 的双会话并行、迁移幂等、间距等观感项）仍待操作者实跑。
- [x] 4.3 `openspec validate` 通过；确认 `fix-webui-streaming-liveness` 与 `fix-pending-queue-liveness` 的 spec 行未被本 change delta 覆盖
  - 备注（实施者）：`openspec validate session-parallel-liveness-and-unread-polish` + `--all` 51/51 通过；本 change 只落盘自己的 4 个 delta 与 tasks.md，其他 change 工件零触碰。

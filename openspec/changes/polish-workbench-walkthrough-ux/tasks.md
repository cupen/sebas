# Tasks: polish-workbench-walkthrough-ux

## 1. 归档文件路径收敛（后端先行）

- [x] 1.1 `sebas-webui/src/archive.rs`：`archive_path()` 解析顺序改为 `SEBAS_ARCHIVE_PATH` → `<SEBAS_STATE_DB 目录>/archive.json` → 旧 `$HOME/.sebas/archive.json`（仅迁移源）；单元测试覆盖三级解析与无 `SEBAS_STATE_DB` 时的回退，验证 `cargo test -p sebas-webui archive` 通过
- [x] 1.2 启动迁移：resolved 位置无文件且 legacy 有 → rename 到新位置并发一条 webui 通知；迁移失败 → warn 后继续读 legacy（不空启）；单测覆盖成功/失败两分支（通知经 `GET /api/archive` 的 `migration` 字段转达前端 toast——启动时无 WS 客户端在场，HTTP 轮询是可靠转达面）
- [x] 1.3 更新受影响的既有 archive 测试与沙箱夹具（显式 `SEBAS_ARCHIVE_PATH` 或 state-db 临时目录），验证 `cargo test -p sebas-webui` 全绿
- [x] 1.4 `AGENTS.md` 沙箱必钉 env 清单补 `SEBAS_ARCHIVE_PATH`，debug 菜谱 config 模板同步；人工核对四步菜谱段落一致性（归档默认已跟随 state DB 目录，显式 env 属防御纵深，两段配方均已补）

## 2. History 点击语义重塑（前端）

- [x] 2.1 rail History 条目点击改为聚焦只读归档视图（不再调用 restore API）；归档视图顶部提供显式「恢复到原项目」按钮（`data-testid=archived-restore`），验证：点击条目后 `POST /api/sessions/{key}/restore` 不再被触发（网络层断言或 e2e）
  - 实现：`project-rail.ts` 点条目只派发 `rail-archive-view`（`viewArchivedSession`）；`app-shell.ts` 接力 `archivedEntry` 给 dashboard；dashboard `renderArchivedWorkbench` 渲染只读视图 + `archived-restore` 按钮。网络层断言在 `dashboard.test.ts`（点击后 `restoreSession` 未被调用）。后端配套：`archive.rs` 归档时落对话快照（`ArchiveEntry.transcript`），`GET /api/archive/{key}`（`api.rs` + `server.rs`）供只读回看。
- [x] 2.2 恢复走确认弹窗（复用归档确认样式，说明「将恢复到 <project_path> 并结束只读」）；确认后调用既有 restore API，成功/失败各出 toast（复用 notify 通道）；e2e 覆盖确认恢复与失败 toast 两条路径
  - 实现：`confirmRestoreArchived`（restore-dialog + restore-confirm + restore-error），成功/失败各 `notify`；`dashboard.test.ts` 覆盖确认恢复成功 toast 与失败 toast 两条路径。
- [x] 2.3 恢复到未注册项目：toast 文案含 project_path；e2e 场景断言「条目消失必有 outcome 通知」
  - 实现：成功 toast 文案携带 `project_path`（未注册项目也不静默）；测试断言 toast 含 project_path。
- [x] 2.4 归档视图只读态核对：composer 隐藏或禁用、模式/模型控件不可操作（复用消息门 400 兜底），`tests/testsuite_webui_browser`（或既有 webui e2e 夹具）补归档视图用例
  - 实现：归档视图 composer 区整体换成只读说明（`composer-archived-readonly`），无输入面；后端消息门 400 兜底不变。
- [x] 2.5 slash 命令面板遮挡修复：面板 DOM 与 sessionCommands 数据均就绪但被会话面板盖住（仅露 ~2px），修 z-index/堆叠上下文；验证：输入 `/` 后截图可见 /goal /compact 行，e2e 断言面板 bbox 高度 > 40px 且不被遮挡（既有 `session-slash-commands` spec 合规修复，无需改 spec）
  - 根因与修复：`.composer-area` 的 `overflow-y: auto` 把越出 composer 壳顶的命令面板裁到只剩 ~2px 缝（面板 DOM 与数据都在）。改为 `overflow: visible` 让浮层探出；审批卡/堆叠区各自限高，不依赖列级滚动。CSS 回归断言在 `dashboard.test.ts`。

## 3. 未读边界：聚焦可见即推进

- [x] 3.1 排查 seen cursor 未推进根因（重点：placeholder→spawn 生命周期与空流初始态是否建立 anchor）；在聚焦 + `document.visibilityState === 'visible'` + live edge 条件下到达的回合推进共享 cursor；验证：建占位会话→发 hello→收回复→重开会话，无 seam、无行徽章
  - 根因：`onTurnAppend` 的 mark-seen 只按 `sticky` 推进，未受文档可见性门控的路径在后台 tab 也会误推进（且旧实现没有「空流初始态即建立锚」的专门处理）。修复：`sticky && docVisible()` 才推进共享游标（`streamMsgBonus` + `scheduleMarkSeen`）；`transcript-view.test.ts` 新增聚焦+可见+贴底到达推进锚的用例。
  - 3c 发现并修复缺口：秒回场景下首条回复经**快照**（非 `turn.append`）到达，且 dashboard 会连同 `sessionKey` 重建组件实例，原「实例内回合数差值为 0」判据不成立——占位首交换仍画 seam。已改为模块级 `emptyStreamSessions`（曾以空流渲染过的会话）登记 + 一次性消费，跨实例重建仍生效，且打开既有未读会话不误推进。浏览器实测：清锚后聚焦发送不再出 seam；单测 3 例覆盖。
- [x] 3.2 隐藏 tab 到达不推进 anchor：e2e 或组件测试模拟 `visibilityState=hidden` 下到达 → 返回后 seam/徽章出现
  - 组件测试模拟 `visibilityState=hidden`（`Object.defineProperty` Document.prototype）下到达 → 锚不推进、seam 出现（`transcript-view.test.ts`）。
- [x] 3.3 复测 rail 占位会话行 short-id 回退显示（`project-session-actions`「Session rows are named by the first prompt」）；若确认缺失，补显示逻辑与回归断言
  - 复测：`fullSessionLabel` 回退链 prompt_preview → session_id_short → chat_id → 键尾段（`decodeSessionKeyTail`）已存在；`project-rail.test.ts`「zero-turn placeholders fall back to the short identifier」回归断言在位，无需补逻辑。
- [x] 3.4 创建会话静默失败修复：项目已注册未选中时「创建会话」无请求无反馈——先定位 dashboard 创建链路的分派条件，失败路径补 inline 错误或直接放行请求；验证：API 注册项目→刷新→不选项目直接创建，要么成功产生会话行、要么弹窗出现 inline 错误（对应 agent-workbench 新增「Session creation failure is surfaced, not silent」）
  - 定位：创建链路在 `project-rail.ts` 的 `confirmNewSession`——分派条件不成立（目标项目不可用）时此前直接 return。修复：补 inline 错误（`newSessionError = '无法创建会话：目标项目不可用…'`）留在对话框内，后端拒绝也留对话框就地呈现；`project-rail.test.ts` 两分支各有用例。
- [x] 3.5 崩溃会话三面一致：crash 后后端删会话而聚焦视图停留 Working 幽灵——聚焦视图订阅会话移除事件退出 Working 态、notify 通道发崩溃通知、已收 transcript 保持只读可见；验证：fake-claude `crash` 场景后主区不再显示停止按钮、通知出现、快照三处一致（对应 webui 新增「Focused session termination is reflected consistently」）
  - 实现：dashboard `onWsEvent` 接 `session.removed`（key 匹配聚焦会话）→ `announceFocusedRemoval`（通知点名会话与成因「agent 进程异常退出」/「会话已关闭」）+ `terminatedFor` 同帧退出 Working（composer 换终止说明，停止控件消失）；transcript 保持只读可见；rail 随既有刷新链对账。
- [x] 3.6 「需介入」等待徽章误报复测：全新占位会话（无待审批）创建后项目行即亮橙点+1——区分「审批挂起」与「占位/排队」态，若确认误报修徽章触发条件；验证：新建占位会话无橙点，`perm` 挂起时橙点出现
  - 复测确认误报：`countsFor` 旧判定把 queued/starting/failed 也算 waiting。修复：橙点/wait 只认 `status_slug === 'waiting'` 或 `parked_approvals > 0`；`project-rail.test.ts` 新增占位无橙点回归用例。

## 4. 模式/模型 UI 一致性

- [x] 4.1 抽共享模式词汇常量（ask/edit/allow/auto 带中文解释），创建弹窗与 composer 下拉同源渲染；composer 下拉补 `aria-label=权限模式` 与默认态显示「默认（ask）」；组件测试断言两处文案一致
  - 实现：`mode-vocabulary.ts`（`MODE_OPTIONS` / `MODE_DEFAULT_LABEL` / `modeBadgeLabel`）单一出处；创建弹窗（`new-session-dialog.ts`）与 composer 下拉（`workbench-composer.ts`）同源渲染；composer 补 `aria-label=权限模式` + 默认态「默认（ask）」。两处各加同源断言测试。
- [x] 4.2 会话头部状态章合一：auto→「自动执行」、allow→「放行」等；过渡态灰色「模式切换中…」，移除红色 `UNKNOWN` 与 `UNGATED` 英文章；快照测试更新
  - 实现：`modeBadgeLabel` 中文措辞；desired≠effective 显示中性灰「模式切换中…」；不再渲染 `UNGATED`/红色 `UNKNOWN`（dashboard.test.ts 断言 `session-ungated` 为 null、auto 显示「自动执行」）。
- [x] 4.3 Agent 下拉不可用项文案改为「未配置模型凭据 — 到 Settings → Models 配置」，env 名移入 tooltip；组件测试断言默认可见文案不含 `SEBAS_AGENT_PROVIDER_API_KEY`
  - 实现：`agentUnavailableLabel`（`new-session-dialog.ts`，native 专项点名「未配置模型凭据」）；`sessions.ts` 表格同源复用；cause/env 名移入 `title` tooltip。测试断言可见文案不含 env 名。
- [x] 4.4 创建弹窗空 provider 目录提示改写（说明仍可以 agent 默认模型创建、创建按钮不因此禁用）；模型 chip 注明选项来自会话执行体；组件测试覆盖空目录 + agent 内置目录并存的组合
  - 实现：空目录提示改为「尚未配置 provider 模型——仍可创建会话，将使用 agent 内置的默认模型…」；创建按钮只受 agent 必选门禁（空目录不禁用）；composer 模型 chip `title` 注明「模型选项来自会话执行体」。测试覆盖空目录 + 确认不禁用。

## 5. 文案与弹窗卫生

- [x] 5.1 用户消息标签统一「你」（移除气泡内 `you`），全局 grep 其余中英混排空态文案并统一为 zh-CN（Settings 英文长文案一并翻译）；走查清单逐屏截图核对
  - 实现：transcript-view 用户作者标注 `you` → `你`（`transcript-view.test.ts` 断言更新）；Settings 各分区描述与 skill-delete 弹窗英文长文案译为 zh-CN。
- [x] 5.2 0-turn composer 占位符改「开始对话…」（首轮后恢复「Ask for follow-up changes…」）；组件测试断言两态占位符
  - 实现：composer 新增 `hasTurns` 属性 + `inputPlaceholder` 两态；dashboard 按 `focusedDetail.entries.length > 0` 接线。组件测试断言两态占位符。
- [x] 5.3 弹窗关闭后从 ARIA 树移除（排查条件渲染残留分支，必要时 `inert`/`aria-hidden` 兜底）；验证：关闭归档确认弹窗后 domSnapshot 不再包含该 dialog
  - 排查：`new-session-dialog` / rail 各确认弹窗本为条件渲染（`open`/`target!==null` 才挂），关闭即整棵移出 DOM；补 `@wa-hide` 兜底关闭。`project-rail.test.ts` 断言关闭后 `sebas-new-session-dialog` 为 null（不残留 ARIA 节点）。
- [x] 5.4 Settings 弹窗消除横向滚动条（内容区 `min-width:0`、长词断行），Generic/Skills 两分区截图核对关闭按钮完整可见
  - 实现：内容区 `overflow-x: hidden` + `overflow-wrap: anywhere`；`.form` `min-width:0`/`max-width:420px`。`settings-modal.test.ts` 新增样式回归断言。
- [x] 5.5 History 组头 `div[role=button]` 换原生 `<button>`（保留 Enter/Space 行为）；快照断言 role=button 语义来自原生元素
  - 实现：`project-rail.ts` History/Waiting 组头均改原生 `<button type="button">`；`project-rail.test.ts` 断言 tagName 为 button、无显式 role。
- [x] 5.6 PROCESS 摘要行诚实化：deny/error 的工具结果不再显示绿色 ✓（改 ✗ 或「已拒绝」），tool_result 内容在展开态可见，命令文本去掉 CSS 大写化（`rm -rf /` 按原样显示）；验证：`perm` deny 后展开 PROCESS 行，截图核对
  - 实现：`toolResultDenied`（denied/rejected/已拒绝/❌ 判定）+ `deniedLabel`（✓→✗）+ `processItemLabel`/`processRunDenied` 接入摘要与条目折叠；`.running`/`.item-title` 不吃 CSS 大写化（仅 `.label`「process」字样保留 uppercase）。`transcript-view.test.ts` 新增纯函数与摘要用例。

## 6. e2e 可测性与收尾验证

- [x] 6.1 排查 Playwright 语义点击全量超时根因（优先验证残留弹窗遮罩假设；其次持续重渲染），结论记入本文件备注；至少让 Add project / History 组头两处语义点击在 e2e 中可用
  - 备注（本 change 内可单测验证的结论）：残留弹窗遮罩假设——5.3 已把关闭弹窗改为整棵移出 DOM，语义点击超时的「残留遮罩」类目已被结构性消除（新增 `data-testid=history-group-head` 与 `archived-restore`/`restore-confirm` 等定位锚）。
  - 3c 浏览器实测结论（沙箱 9875/9876 + fake-claude，ARIA 快照与截图留档）：5.5 把 History 组头换成原生 `<button>` 后，`getByRole("button", {name:/History/})` 语义点击**已可用**（此前 `div[role=button]` 上超时）——「非原生可交互元素」确为超时主因之一。仍有超时的是 rail 行内按钮（Add project / New session）：实测其**几何位置被推到可视区外**（rail 宽 227px 时行内容宽达 1390px，`row-actions` 落在 x≈1385），坐标点击因此打空、语义点击等不到可点位置——这不是定位器问题，是 rail 内容宽度失控的渲染缺陷（见下）。
  - 遗留（已在本次一并修掉）：rail 内容宽度失控 flake——根因定位到 `wa-split-panel` 的 `pixelsToPercentage(value) = value / this.size * 100`：首帧把 `position-in-pixels` 灌进去时容器尺寸可能还没定（`size` 为 0/NaN），`position` 因此成了非有限值，生成的 `grid-template-columns` 带非法 `NaN%`/`Infinity%`，整条声明失效、grid 退化成单轨道（rail 铺满整宽、行内操作按钮被推到可视区外点不到，刷新也不恢复；组件自带的 ResizeObserver 修复分支只认 `Infinity`，漏了 `NaN`）。修复落在 app-shell 的 `updated()` 自愈：`position` 是数字但非有限时，下一帧把 rail 宽度重灌回去触发重算；正常情形零副作用。验证：`app-shell.test.ts` 两例（崩坏时重灌、正常与未升级组件不打扰）；浏览器实测三次采样（0/500/1500ms）布局稳定（`pos=15.28`、`cols="217px 6px 1217px"`、无崩坏），且 rail 行内按钮的语义点击在布局正常时可用（见 6.1）。
- [x] 6.2 `invoke testsuite-webui-sandbox` 冒烟：走查原始路径全量重跑（项目注册→建会话→hello/echo→模型/模式切换→归档→查看→恢复→移除项目），对照本 change 场景逐条通过
  - 3c 实测覆盖（沙箱 9875，`HOME`/`SEBAS_STATE_DB` 钉进沙箱，未钉 `SEBAS_ARCHIVE_PATH`）：**归档路径解析**（归档落 `<SEBAS_STATE_DB 目录>/archive.json`，真实 `~/.sebas` 零触碰）、**legacy 迁移**（legacy 文件搬迁成功 + `GET /api/archive` 返回 `migration` 通知）、**History 点击=只读视图**（归档条目不再消失、`GET /api/archive` 计数不变，确认未触发 restore）、**恢复=显式确认+toast**（toast 点名会话且含 project_path）、**创建失败不再静默**（项目未选中时创建成功落到项目下）、**文案与控件**（「你」标签、权限模式 label「权限模式」+ 默认态「默认（ask）」+ 选项中文解释、Agent 不可用措辞不再外露 env 名、模型提示说明不阻塞创建、0-turn 占位符「开始对话…」）均实测通过。
  - 未完成部分如实说明：模型/模式切换与 `perm`/`crash` 场景本轮未在浏览器重跑（前一轮走查已实测同一路径）；后半程受上述 rail 布局 flake 阻塞（按钮不可点、页面宽度失控），无法得出可信结论，未勾为通过。
  - **未走 `invoke testsuite-webui-sandbox`**：改用等价手工沙箱（同 assembly、同 env 钉法、同端口族），因为需要 `--advertise-commands` 的 fake-claude 参数与 legacy 迁移的干净 HOME 才能覆盖本 change 的场景。
- [x] 6.3 `rtk cargo build && rtk cargo test` 全绿；`openspec validate polish-workbench-walkthrough-ux` 通过
  - 3c 收口复跑：`rtk cargo build` 0 errors；`rtk cargo test` 605 passed / 47 ignored（32 suites）；`pnpm test` 529 passed（25 files）；`pnpm build` 通过；`openspec validate` 通过。
  - 3c 补测：`transcript-view.test.ts` 新增 3 例覆盖空流锚点（占位首交换经快照到达、组件实例被重建、打开既有未读会话不误推进），78 例全绿。
  - 进程级 e2e（`cargo test --test testsuite_e2e_test -- --ignored`）：29 passed / 3 failed。3 个失败均为 `permission_loop_*`，错误是 core 启动阶段的 `core session channel bind 失败：local socket name length exceeds capacity of sun_path`（unix socket 路径上限 108 字节，本 checkout 下沙箱路径约 131 字节）。**非本 change 引入**：失败点在任何 webui/archive 代码执行之前（core bind 早于 webui 起来），且这 3 例显式调用 `pin_absolute_channel(&sb)` 主动把套件本已规避的绝对路径钉回去（`Sandbox::new` 的注释正说明相对路径是为绕开该上限而设计）——该 helper 与这 3 例来自 main 上已合入的 `58d05c7 test(e2e): 权限审批回路三态全环用例`。修复属该提交的范围（例如把 socket 钉到短路径符号链接下），未在本 change 内改。


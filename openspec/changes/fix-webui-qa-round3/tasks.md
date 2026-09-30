# Tasks — fix-webui-qa-round3

## 1. P2 行为修复

- [x] 1.1 D1 thinking 折叠成员与可区分性：run 分派修至「text 条目永不入过程 run」；thinking 二级折叠标题标明 thinking；正文段零过程芯片。验证：transcript-view 单测（thinking 场景快照）+ GUI 手测 thinking agent 回合截图。（QA W2 #8）
  - 落点：`transcript-view.ts`。分派层本就把 markdown 归文本 run（新增交替回合计时钉住）；误标根因在**呈现层**——收起行是裸文本、紧贴正文段，被读成「正文挂 PROCESS thinking 芯片」。修复：折叠收起行加 surface-2 小胶囊（无边框/无阴影，4.4 轻量合同不破）；run 成员词 `processRunKind`（thinking/tool/mixed）驱动 `data-kind` + thinking 专用 glyph（icons.ts 新增 `thinking`）；二级 thinking 条目同挂 glyph。单测 6 条（D1 describe）。
- [x] 1.2 D2 mode 下拉重复展开：选中后指针再次点击可重新展开，箭头与显隐一致（composer 工具条 + 创建对话框两处）。验证：前端单测 + GUI 手测。（QA W2 缺陷 3）
  - 落点：`wa-select-rescue.ts`（composer 与创建对话框共用的文档级包装层，design D2「展开态布尔与实际渲染一致性修复」）。新增正向不变量 `isOpenButUnrendered`（open=true 而 wa-popup 未 active 或 listbox 仍 hidden）+ `forceRenderOpenSelect`（0ms 定时器内拉齐渲染面，绝不落在指针序列内）；pointerdown/change 的定时器落地时重读 open——收起走既有残留清理、展开未渲染就地修好。单测 4 条；既有源码契约（零 hidePopover 调用、0ms 定时器）保持。
- [x] 1.3 D3 `/sessions` 栅格 1280 收纳：换行或横向滚动，卡片操作可达。验证：1280 视口布局断言（scrollWidth ≤ clientWidth 或存在滚动容器）+ GUI 截图。（QA W1 D1）
  - 根因：`.outlet.padded` 的 `width:100%` + 左右 padding 在 shadow DOM 里按 content-box 解析（文档级通配 border-box 进不来），恰好多出 2×space-8=64px 溢出视口、被宿主 overflow hidden 裁掉——换行栅格本就存在（auto-fill/minmax），只是容器溢出。修复：`.outlet.padded { box-sizing: border-box }`（app-shell.ts）。择定**换行**方案（design 倾向项）。样式合同单测 1 条。
- [x] 1.4 D8 usage 统计卡随窗口联动：卡片与图表同窗口重算。验证：前端单测 + GUI 手测切粒度前后截图。（QA W3 观察 2）
  - 落点：`usage.ts`。卡片数字改为从与图表**同一份 buckets** 现场累加（`windowTotals` 纯函数），空态判别同源——卡片口径按构造与图表窗口一致，切换粒度即随新响应重算；顶层 totals 漂移不再上屏。单测 4 条（纯函数 2 + 视图 2，含切粒度重算与 totals 漂移防线）。

## 2. P3 呈现与功能补全

- [x] 2.1 D4 usage 控件收纳（1280 无溢出）。（QA W3 D1）
  - 根因与 D3 同源（同一 padded 出口溢出把刷新按钮推出视口）；`.controls` 的 flex-wrap 本就存在。修复随 1.3 的 border-box；app-shell 样式合同单测覆盖（D3/D4 同一条）。
- [x] 2.2 D5 provider 探测错误文本完整可读（换行/展开）。（QA W3 D2）
  - 落点：`settings-modal.ts`。`.entries-head` 加 flex-wrap + `.fetch-error` 改 `flex: 1 1 100%` 独占整行、`overflow-wrap: anywhere`——长 cause 落到标题行下方整行铺开换行，不再被表单右缘单行裁切。样式合同单测 1 条。
- [x] 2.3 D6 重命名后详情头部即时同步；侧栏与头部同一标题源。（QA W3 D3）
  - 落点：`project-rail.ts`（改名成功回调派发 `SESSION_LABEL_CHANGED_EVENT`）+ `dashboard.ts`（监听并就地补丁 summary 的 recent_sessions 行）。头部命名链 `fullSessionLabel` 与 rail 行本就共享同一份行数据（无第二处缓存）；事件让同一份源提前一拍更新（不等 WS 相位帧/节流刷新），「侧栏已新、头部仍旧」的窗口关死。单测 2 条（改名跟随 + 清名回退 + 未知键 no-op）。
- [x] 2.4 D7 History 条目标签取归档时刻现用标签（先查 archive wire 是否带 label，按 design 择前端取数或后端快照补齐）。（QA W1 D3）
  - wire 已带：后端 ArchiveEntry 自 round4 3.1 落档 `operator_label`（归档时刻操作者现用标签，QA 沙箱 archive.json 实证：label=旧自动标题、operator_label=改名值）。择**纯前端取数**（design 的最小改动支）：client.ts 类型补 `operator_label?`，project-rail 导出 `archivedEntryLabel`（operator_label 非空优先、空白按未设置），History 行、归档只读视图头、恢复通知三处同源取数。单测 2 条。
- [x] 2.5 D9 项目栏重排序入口（上移/下移或拖拽），走既有 `/api/projects/reorder`，持久化。（QA W1 D2）
  - 拖拽本已存在但 QA 实锤真实指针序列里滑成文本选择（可发现性/成功率低）。落点：`project-rail.ts` 项目「…」菜单补**上移/下移**（确定性入口，边界位禁用），与拖拽共用同一条乐观更新 + `api.projects.reorder` 落盘链（`applyProjectOrder`，单一持久化路径）。单测 4 条（菜单存在/边界禁用/上移下移持久化）。
- [x] 2.6 D10 会话选择写 URL（pushState `/sessions/{key}`，清焦回落 `/`，刷新自持焦点，未知键降级不白屏）。（QA W3 D4）
  - 落点：`project-rail.ts`（openSession/confirmNewSession 导航到 `/sessions/{key}`）、`app-shell.ts`（工作台三态收敛到**同一个** dashboard 模板字面量——Lit 按模板身份复用 DOM，聚焦切换不再拆毁重建实例；deepLinkKey 属性驱动焦点）、`dashboard.ts`（`reflectFocusUrl`：rail 聚焦事件投影 URL、聚焦移除回落 `/`；pathname 一致幂等；只在显式聚焦事件上调用——summary 收敛不反推，浏览器前进/后退与项目行点击回 `/` 不被抢写）。未知键降级（focusedUnavailable「会话不可得」）与刷新自持（深链路由）为既有行为。单测：rail 1 条 + dashboard 2 条（写 URL/回落）+ app-shell 样式合同 1 条；旧「不写深链」源码契约按新 spec 反转更新。
- [x] 2.7 D11 登录前 `/ws` 静默/长退避，消除 401 console 噪音。（QA W4 D4）
  - 落点：`ws.ts`（双重防线：从未成功打开过的失败走长退避 `unopenedBackoffMs` 缺省 30s——401 升级拒绝的典型形状，短梯只服务曾连上后掉线的重连；`setAuthGated` 闸——未认证态不连、不排重连，撤闸即 reconnectNow）+ `app-shell.ts`（checkAuth 未认证/showLogin 会话失效 → 上闸；markAuthReady → 撤闸）。模块装载的急连至多产生一条 401（spec「至多做一次静默尝试」）。单测：ws 4 条（长退避/短梯回归/闸静默/撤闸重连）+ app-shell 接线 2 条。

## 3. 回归与验收

- [ ] 3.1 全量 `cargo test` + 前端单测 + 既有 Playwright 套件（conversation / parallel-permissions / unread-badge / settings 等）不回归。
  - 状态：全量前端单测（vitest 790 过）、`cargo build`、全量 `cargo test` 已跑——唯一失败是既有已申报的 `full_e2e_test.rs::slow_stream` 并行负载时序抖动，串行复跑通过（HEAD 既有属性）。另：全新 worktree 首次 `cargo build` 不落 `fake-acp-agent.exe`（sebas-acp 的测试桩 bin），需 `cargo build -p sebas-acp` 补齐后 acp_session_mapping 才可跑（已补，已过）。Playwright 套件留给 review 阶段（本阶段按分工只配普通单元测试，不起 e2e 装配）。
- [ ] 3.2 GUI 逐项手测（1280×720 沙箱）：上表 11 项逐项复核截图留档（对照 qa-evidence/w1、w3 的缺陷截图）。
  - 状态：review 阶段执行（本阶段按分工不写集成/e2e、不起 GUI 沙箱）。
- [ ] 3.3 thinking / drip / flood / perm / parallel 五个 fake 场景各跑一回合，确认核心链路无回归。
  - 状态：review 阶段执行（同上；`tests/testsuite_e2e_test.rs` 进程级 journey 未改动，属既有装配）。

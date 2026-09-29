## ADDED Requirements

### Requirement: Probe errors stay legible

provider 编辑面内的探测/抓取失败反馈 SHALL 完整可读：长错误文本 SHALL 换行或在可展开区域呈现，SHALL NOT 被表单右缘单行裁切至不可读。错误反馈不得要求操作者借助 devtools 才能看到失败原因。

#### Scenario: long probe failure is fully readable

- **WHEN** 操作者对一个指向不可达地址的 provider 执行探测且失败返回长错误文本
- **THEN** 错误文本完整呈现（换行或展开），不因容器宽度被截断

## MODIFIED Requirements

### Requirement: 会话 mode 在 dashboard 可见可切

会话 dashboard SHALL 展示当前会话的 mode（含远端会话已有的 desired/effective 呈现），切换入口 SHALL 位于输入框底沿左端，与模型芯片、提交按钮同一工具条，提交后走中途切换端点。会话头部 SHALL NOT 渲染 mode 切换控件。mode 显示对远端节点会话沿用 effective/desired 差异化呈现（effective 缺失时只显 desired）。

mode 切换下拉（含创建对话框的 mode 下拉）SHALL 支持连续指针交互：选择一项后，再次以鼠标点击 SHALL 重新展开选项列表（组件的展开态视觉与实际行为 SHALL 一致），无需刷新页面或改用键盘。

#### Scenario: composer 创建表单的 mode 选择

- **WHEN** 操作者在创建对话框展开表单
- **THEN** 表单提供 mode 下拉，缺省项为「agent 默认」（不发送 mode 字段）

#### Scenario: 会话头部切换 mode

- **WHEN** 操作者在输入框底沿的 mode 下拉选择另一个 mode
- **THEN** 前端提交 `POST /api/sessions/{key}/mode`；成功后下拉显示的 mode 更新，失败显示非致命错误且保持原显示

#### Scenario: 下拉可再次展开

- **WHEN** 操作者从 mode 下拉选择一项后，再次以鼠标点击该下拉
- **THEN** 选项列表重新展开；且下拉的展开指示（箭头方向）与列表实际显隐一致

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded key, operator label or first-message preview, session id, status slug, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Selecting a session in the rail, opening its `/sessions/{key}` deep link, or posting `/switch` SHALL focus that session in place — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. There SHALL be no separate per-session detail surface: the workbench renders the focused session. Switching the displayed project SHALL NOT alter the focused session pointer. The rail's current-session marker SHALL be derived from the focused-session pointer, not from the browser location.

会话选择 SHALL 反映到 URL：聚焦任一会话时地址 SHALL 更新为其 `/sessions/{key}` 形式（history pushState，不引发页面跳转）；无焦点（新建态）时地址回落 `/`。带会话地址刷新/重开 SHALL 恢复对该会话的焦点（会话已不存在时按未知键如实降级）。rail 标记仍由焦点指针派生，不因 URL 直接改写。

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

`GET /api/summary` SHALL NOT embed the focused session's full transcript; conversation content SHALL be fetched via the per-session detail endpoint with an incremental cursor. The dashboard SHALL rate-limit and dispatch refetches: lightweight events (session metadata, presence) refresh lists, turn content events update only the focused conversation via its cursor — an operator's browser SHALL NOT issue a full refetch storm (multi-request × whole-transcript responses) per streamed frame.

前端 SHALL 在会话重建或 core 重启后作废本地增量游标：当快照响应携带的世代/起始位置与本地游标矛盾（本地游标大于服务端当前日志长度，或会话标识世代变化）时，客户端 SHALL 丢弃本地游标与缓冲、重取全量快照，不得因陈旧游标永久拒收增量。

对话视图 SHALL 在回合进行中自动跟随流式输出：当操作者已聚焦该会话并处于贴底跟随状态时，新到内容 SHALL 持续滚动可见；未读缝的显隐 SHALL NOT 重建整个对话 DOM（既有条目的展开态 SHALL 保留）。流式期间渲染 SHALL NOT 对未变化的历史条目做整块重解析——正文增量 SHALL 以增量方式合并进当前条目。

会话列表页（`/sessions`）在 1280px 宽视口 SHALL 完整可用：卡片栅格 SHALL 换行收纳或提供横向滚动，任何卡片的操作入口（如 Focus/Close）SHALL 可达，SHALL NOT 出现被视口裁剪且不可滚动回收的列。

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the core's own session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: rail selection focuses in place

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes focused, the workbench renders its conversation in place, and the operator is not navigated to a separate detail page

#### Scenario: selection writes the session URL

- **WHEN** the operator selects a session in the rail（或经任意聚焦路径）
- **THEN** 地址栏更新为该会话的 `/sessions/{key}`，页面不发生跳转重载

#### Scenario: reload keeps focus from URL

- **WHEN** the operator reloads the page while the address carries `/sessions/{key}`
- **THEN** 该会话恢复为焦点会话；地址指向未知会话时工作台如实降级（空焦新建态或未知提示），不白屏

#### Scenario: project switch leaves focus alone

- **WHEN** the operator switches the displayed project
- **THEN** the focused session pointer is unchanged

#### Scenario: archive hides session from list

- **WHEN** a session is archived
- **THEN** it is no longer returned by `GET /api/sessions` and appears only in the `GET /api/archive` response

#### Scenario: focusing enables immediate follow-up

- **WHEN** a session becomes focused through any supported path
- **THEN** the workbench composer's next submission is delivered to that session as a follow-up message

#### Scenario: summary stays small while transcript is large

- **WHEN** the focused session has a multi-megabyte transcript and a turn is streaming
- **THEN** `GET /api/summary` responses remain in the kilobyte range; conversation content flows only through the session detail endpoint with the cursor

#### Scenario: streamed frame does not trigger full refetch

- **WHEN** a `turn.append` frame arrives
- **THEN** the dashboard updates the focused conversation from the frame (or a cursor-limited detail fetch), without re-fetching nodes, projects, sessions, and the full summary

#### Scenario: stale cursor after core restart converges

- **WHEN** the core restarts and the rebuilt session log assigns positions from zero while the browser holds an old high-water cursor
- **THEN** the browser detects the contradiction, resets its cursor, refetches the full snapshot, and subsequent increments apply normally

#### Scenario: auto-scroll follows streaming at the bottom

- **WHEN** the operator is focused and pinned to the bottom while output streams
- **THEN** new content stays in view without manual scrolling

#### Scenario: seam toggle preserves DOM state

- **WHEN** the unread seam appears or disappears during streaming
- **THEN** previously rendered entries are not rebuilt from scratch and expanded thinking/tool items keep their open state

#### Scenario: sessions grid stays usable at 1280

- **WHEN** 1280px 宽视口打开会话列表页且卡片不少于三张
- **THEN** 栅格换行或可横向滚动，任一卡片的 Focus/Close 等操作完整可达，无被裁剪且不可达的列

## MODIFIED Requirements

### Requirement: 全局核心可达性横幅

app-shell SHALL 提供全局"核心不可达"横幅，其状态由 WS 推送驱动而非轮询：客户端在 `/ws` 连接建立或重连后 SHALL 主动请求当前可达性（`core.reachability.get`），此后随 `core.reachability` 翻转通知即时更新。当上报的 `reachability.ok` 为 false 时横幅 SHALL 展示，内容含 cause（如 `socket absent`），呈现层级与现有"与服务器的连接已断开"横幅一致（全局、role=alert）；可达性恢复通知到达后横幅 SHALL 即时消失，无需刷新页面。横幅 SHALL 不阻塞页面其余部分的浏览（与"webui 在 core 不可达时继续服务"的既有语义一致）。WS 断线窗口内错过的翻转 SHALL 由重连后的 get 响应收敛，无需额外机制。

#### Scenario: core 不可达时横幅出现

- **WHEN** core 进程停止或通道不可达，浏览器停留在任意页面
- **THEN** 翻转通知到达即出现全局横幅，文本包含 `reachability` 上报的 cause，不等待任何轮询周期

#### Scenario: core 恢复后横幅消失

- **WHEN** core 恢复服务且通道重新握手成功
- **THEN** 恢复通知到达后横幅即时消失，无需刷新页面

#### Scenario: 连接建立即获当前态

- **WHEN** 浏览器建立（或断线重连后重建）`/ws` 连接
- **THEN** 客户端发起可达性 get 请求并以响应初始化横幅状态，不依赖轮询

#### Scenario: 断线窗口的翻转由重连收敛

- **WHEN** `/ws` 断线期间 core 经历不可达→可达翻转
- **THEN** 重连后的 get 响应携带当前真实状态，横幅据此收敛

### Requirement: 降级与错误表现

The WebUI frontend SHALL surface a visible global indicator when its live
connection to the server (`/ws`) is lost, and SHALL clear that indicator and
refresh visible view data automatically when the connection is restored
(the existing refetch hook). Data requests that fail at the network level
(server process down, DNS/connection failure) SHALL be distinguishable from
server-side business errors (4xx/5xx with a backend error body) so views can
react appropriately. List-style views (dashboard, project rail, sessions)
SHALL render an inline failure state with a retry affordance instead of a
blank panel when their initial data load fails. The workbench composer's
submit gate SHALL consume the same WS-pushed core reachability state as the
global banner (initial get + flip notifications): a reported
`reachability.ok = false` SHALL disable the composer until a recovery
notification or a fresh get response reports reachable, and `/api/summary`
SHALL remain a pure on-demand read endpoint whose availability no longer
feeds the submit gate.

#### Scenario: ws 断线显示全局横幅

- **WHEN** the `/ws` connection drops
- **THEN** a global connection banner is visible in the app shell, and the
  existing exponential-backoff reconnect keeps running

#### Scenario: 重连恢复后横幅消失并刷新数据

- **WHEN** the `/ws` connection is re-established
- **THEN** the banner disappears and the visible views refetch their data
  (the existing `sebas:refetch` behavior)

#### Scenario: 网络级失败可区分于业务错误

- **WHEN** a data request fails without an HTTP response (server process is
  down, connection refused)
- **THEN** the frontend error is a recognizable network-unreachable error
  rather than a backend-`ApiError` with business semantics

#### Scenario: 列表加载失败显示内联重试态

- **WHEN** the initial data load of the dashboard, project rail, or sessions
  view fails
- **THEN** the view renders an inline failure state with a retry affordance
  instead of an empty or silently stale panel

#### Scenario: summary 轮询失败等同 core 不可达

- **WHEN** the 5s polling is retired and the composer's reachability source
  is WS push only, while `/api/summary` remains available as an on-demand
  read endpoint
- **THEN** the submit gate follows only pushed `core.reachability` state
  (initial get + flip notifications); an `/api/summary` failure on the
  on-demand path SHALL no longer disable or enable the submit gate

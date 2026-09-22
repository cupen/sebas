# webui Delta

## MODIFIED Requirements

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded key, chat and thread ids, session id, status, phase, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Selecting a session in the rail, opening its `/sessions/{key}` deep link, or posting `/switch` SHALL focus that session in place — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. There SHALL be no separate per-session detail surface: the workbench renders the focused session. Switching the displayed project SHALL NOT alter the focused session pointer. The rail's current-session marker SHALL be derived from the focused-session pointer, not from the browser location.

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

`GET /api/summary` SHALL NOT embed the focused session's full transcript; conversation content SHALL be fetched via the per-session detail endpoint with an incremental cursor. The dashboard SHALL rate-limit and dispatch refetches: lightweight events (session metadata, presence) refresh lists, turn content events update only the focused conversation via its cursor — an operator's browser SHALL NOT issue a full refetch storm (multi-request × whole-transcript responses) per streamed frame.

前端 SHALL 在会话重建或 core 重启后作废本地增量游标：当快照响应携带的世代/起始位置与本地游标矛盾（本地游标大于服务端当前日志长度，或会话标识世代变化）时，客户端 SHALL 丢弃本地游标与缓冲、重取全量快照，不得因陈旧游标永久拒收增量。

对话视图 SHALL 在回合进行中自动跟随流式输出：当操作者已聚焦该会话并处于贴底跟随状态时，新到内容 SHALL 持续滚动可见；未读缝的显隐 SHALL NOT 重建整个对话 DOM（既有条目的展开态 SHALL 保留）。流式期间渲染 SHALL NOT 对未变化的历史条目做整块重解析——正文增量 SHALL 以增量方式合并进当前条目。

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the router's own session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: rail selection focuses in place

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes focused, the workbench renders its conversation in place, and the operator is not navigated to a separate detail page

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

## Purpose

Define the channel between the sebas core — the sole owner of session state and
the sole spawner of ACP children — and out-of-process clients that need to
observe and drive sessions, so that a detached WebUI can be genuinely live and
genuinely drivable without becoming a second writer of session state.

Session identity on the wire is channel-neutral: the protocol carries a `ChannelKey` (channel name plus an opaque per-channel reference) rather than any concrete channel id shape (decouple-feishu-channel).

## Requirements

### Requirement: Core is the single session authority

The core process SHALL remain the only owner of session mapping state and the
only spawner of ACP child processes. A channel client SHALL NOT construct its
own `RouterHandle`, mutate mapping state locally, or spawn children; every
mutation a client wants SHALL be requested over the channel and applied by the
core. Client-side data SHALL be treated as a cache of core state with no
independent authority.

#### Scenario: client mutation is applied by the core

- **WHEN** a channel client requests that a session be created
- **THEN** the core creates it, applies the mapping change in its own state, and
  the client learns the result from the core's response and event stream rather
  than from a local mutation

#### Scenario: client holds no independent state

- **WHEN** a client and the core disagree about a session's status
- **THEN** the core's value is authoritative and the client replaces its cached
  value on the next snapshot or event

### Requirement: Channel transport and authentication
The core SHALL expose the channel on a Unix domain socket created with owner-only permissions (0600) at a configurable path defaulting to `$XDG_RUNTIME_DIR/sebas/core.sock` (falling back to a per-uid temporary directory when `XDG_RUNTIME_DIR` is unset). Every connection SHALL be authenticated by both peer credentials — the connecting uid MUST equal the core's own uid — and a shared secret supplied out of band, in the same posture as the watchdog control RPC. A connection failing either check SHALL be rejected and closed without processing any request. The channel SHALL NOT be exposed over TCP. **补充**：peer-uid 与 secret 鉴权 SHALL 经由真实跨 uid 进程（不仅同 uid 单测）验证——本期单测覆盖 `cross_uid_rejected` 路径。

#### Scenario: foreign uid rejected

- **WHEN** a process running as a different uid connects to the socket
- **THEN** the connection is rejected and closed, and no request on it is processed

#### Scenario: missing or wrong secret rejected

- **WHEN** a connection does not supply the agreed secret, or supplies a different one
- **THEN** the connection is closed without a response

#### Scenario: cross_uid_rejected_live_process

- **WHEN** 跨 uid 进程（fork 子进程后 `setuid` 到不同账户——不是同进程改 uid，而是真实跨进程凭证）尝试连接 core socket 并发请求
- **THEN** 连接被拒绝；服务端 SHALL 在日志写 peer-uid 不匹配记录（`warn!("core channel: peer uid mismatch; closing")`）；不进入 request 处理

#### Scenario: stale socket file is reclaimed

- **WHEN** the core starts and a socket file already exists at the path with no
  live listener behind it
- **THEN** the core removes the stale file and binds a fresh socket

### Requirement: Session observation methods

The channel SHALL provide a snapshot method returning every known session with
the fields the WebUI renders — channel key (including the channel name and
per-channel reference), session id, status, phase, last-active — plus the
session's execution body and its current model when one is set, and a
subscription method that streams session events for the life of the
connection. A subscriber SHALL receive a snapshot first, then events, so that
no event is missed between the two.

#### Scenario: snapshot precedes the stream

- **WHEN** a client subscribes
- **THEN** it receives the current session set before any subsequent event, and
  applying the events in order to that set reproduces the core's current state

#### Scenario: execution body and model are visible

- **WHEN** a client snapshots a session set containing both an ACP session and
  a native-kernel session with a model selected
- **THEN** each entry states its execution body, and the native entry states
  its current model

#### Scenario: session change reaches subscribers

- **WHEN** a session is created, changes status or phase, or is removed in the
  core
- **THEN** every live subscriber receives a corresponding event

### Requirement: Session drive methods

The channel SHALL provide methods to create a session with a prompt and an
optional project directory, send a message to an existing session, close a
session, and cancel a session's in-flight turn. Each SHALL return either the
resulting `ChannelKey` or a typed rejection naming the reason. A create request
carrying a project directory SHALL have that path canonicalized and verified to
be an existing directory before any child is spawned, and SHALL be rejected
otherwise. Spawned sessions SHALL be registered under the requesting client's
channel (e.g. `web` for the WebUI).

The channel SHALL additionally provide IM-frontend semantics for chat-shaped
clients: a message request MAY carry ensure semantics under which the core
creates a session for an unknown key (lazily respawning a dormant session)
instead of rejecting it, preserving each channel's historical auto-spawn
behavior without requiring the client to hold mapping state.

A message request (plain or ensure) MAY carry attachments — locally resolvable
file references (path + mime type + file name), typically images — delivered
alongside the text. The core SHALL verify each attachment's path exists before
delivery; a request with a missing attachment SHALL be rejected with a typed
reason and no message delivered.

#### Scenario: create spawns a real session

- **WHEN** a client requests session creation with a prompt
- **THEN** the core spawns a real ACP session under the client's channel, returns
  its `ChannelKey`, and subscribers observe the new session

#### Scenario: unusable project directory rejected

- **WHEN** a create request names a path that is not an existing directory
- **THEN** the request is rejected with a reason and no child is spawned

#### Scenario: IM 前端对未知 key 的文本自动建会话

- **WHEN** an IM frontend sends a text message with ensure semantics for a key
  the core does not know
- **THEN** the core creates the session (as an inbound text historically did) and
  delivers the message to it

#### Scenario: IM 前端对 dormant 会话的文本复活会话

- **WHEN** an IM frontend sends a text message with ensure semantics for a
  dormant session key
- **THEN** the core lazily respawns the session and delivers the message

#### Scenario: 消息携带图片附件投给执行体

- **WHEN** an IM frontend sends a message whose attachments include an
  existing local image file
- **THEN** the core delivers the text and the image to the session's
  execution body as model-visible content, and the request's response reports
  acceptance

#### Scenario: 附件路径不存在被拒绝

- **WHEN** a message request names an attachment path that does not exist
- **THEN** the response is a typed rejection and nothing is delivered to the
  session

#### Scenario: cancel terminates the in-flight turn

- **WHEN** a client requests cancellation for a session with a turn in flight
- **THEN** the core cancels that turn and the session stays usable for
  subsequent messages

### Requirement: Turn content retrieval

The channel SHALL provide a method returning the rendered turn content the core
holds for a given session, so a client can display an agent conversation it did
not itself receive. The response SHALL carry a monotonic position so a client can
request only what it has not yet seen.

#### Scenario: incremental turn fetch

- **WHEN** a client requests turn content for a session with a position it has
  already seen
- **THEN** the response contains only content after that position, with a new
  position to use next time

### Requirement: Honest degradation when the core is unreachable
When the channel cannot be reached — socket absent, connection refused, secret rejected, or the connection dropped — a client SHALL surface that condition with its cause and SHALL NOT present stale data as current, report a mutation as succeeded, or offer a control whose request cannot be delivered. A client SHALL reconnect on its own and resume with a fresh snapshot when the core returns. **补充**：cause 富化沿用已落地的无条件 enrich（`reachability()` 全失败分支并入闩锁摘要，不限于 ENOENT 分支——闩锁 ready 自清除，stale 读已防住）；三类不可达的机器区分走 `kind` 字段（startup_failed / auth_rejected / disconnected），cause 保持人类可读全串。

#### Scenario: core down is stated, not hidden

- **WHEN** the core is not running and a client renders a session view
- **THEN** the view states that the core is not connected and why, rather than
  rendering an empty or stale list as though it were current

#### Scenario: no unsendable controls

- **WHEN** the channel is unreachable
- **THEN** controls that would require a channel request are unavailable and
  labeled with the reason, and no such request reports success

#### Scenario: reconnect resumes from a snapshot

- **WHEN** the core restarts while a client is connected
- **THEN** the client reconnects, takes a fresh snapshot, and its view converges
  on the core's state without a manual reload

#### Scenario: startup-failure banner distinguished from runtime disconnect

- **WHEN** core 在 ready 之前 fatal 并以 75 退出（socket 不存在、`SEBAS_STARTUP_ERROR_FILE` 存在含 `startup-failure: <原因>`）
- **THEN** webui degradation banner SHALL 显示 `core startup failed: <原因>` 全串（cause 即该全串，前端原文渲染不二次拼接）；`/api/summary.reachability.ok` 为 false、`kind` SHALL 为 `startup_failed`；runtime disconnect 走 `kind: disconnected` + banner "core is not connected" 区分路径

#### Scenario: core startup failure surfaces in banner

- **WHEN** core 在 ready 之前 fatal 并以 75 退出
- **THEN** webui degradation banner SHALL 显示 "core startup failed: <原因>"；`/api/summary.reachability.ok` 为 false 且 cause 字段包含 startup failure 的可读摘要

#### Scenario: core healthy after retry does not retain banner

- **WHEN** watchdog 重启 core 后 core 进入 ready（属运行期崩溃退避场景，不是 startup failure）
- **THEN** degradation banner 消失、reachability 恢复 true——本规约不影响运行期恢复路径

### Requirement: Channel protocol uses the neutral session key

The channel SHALL identify sessions by the channel-neutral `ChannelKey` carried
as a structured field — the channel name plus a channel-specific opaque
reference — encoded for transport. The protocol SHALL NOT expose concrete
channel id shapes (e.g. Feishu `chat_id` / `thread_id`) as first-class fields.
A client SHALL address a session by its `ChannelKey` and SHALL treat the
channel-specific reference as opaque. This replaces the previously Feishu-shaped
`SessionKey` (chat id + thread id) on the wire.

#### Scenario: sessions are addressed by neutral key

- **WHEN** a session originates from channel `feishu` with reference `oc_x` and
  thread `t1`
- **THEN** the channel protocol carries it as `ChannelKey`, and a client can
  display, message, close, or subscribe to it via that key without knowing the
  Feishu id shapes
- **AND** a session originating from channel `web` with reference `w1` is
  carried by the same protocol with a different channel name

#### Scenario: opaque reference is preserved

- **WHEN** a client sends a message addressed to a `ChannelKey`
- **THEN** the core routes it to the session whose key matches, and the
  channel-specific reference is passed through uninterpreted by the core

### Requirement: Approval requests surface from every execution body

The subscription stream SHALL surface permission/approval requests from every
execution body — the native kernel's gated tool calls and the ACP bridge's
permission requests alike — as answerable frames carrying the request id, the
tool name, and the session's `ChannelKey`. A decision returned via the approval
answer method SHALL be routed by the core to the originating execution body by
request id alone. Requests with no reachable client SHALL fail closed at the
originating execution body. This extends the native-only approval surfacing
(wire-webui-sebas-agent-e2e) to ACP-bridge sessions so a detached IM frontend
can render the same permission cards it rendered when it shared the core's
process.

#### Scenario: ACP 权限请求出现在订阅流

- **WHEN** an ACP-bridge session raises a tool permission request
- **THEN** a connected IM frontend receives an approval frame naming the
  request id and session key, and can render a permission card from it

#### Scenario: 决定按 request_id 回路由

- **WHEN** the IM frontend answers an approval frame with a decision
- **THEN** the core routes the decision to the ACP session by request id and the
  parked tool call resumes or is denied accordingly

#### Scenario: 无客户端连接时 fail-closed

- **WHEN** an approval request arises while no frontend is connected to answer it
- **THEN** the request fails closed at the execution body, matching the native
  kernel's existing posture

### Requirement: Spawn backend hint validation

Spawn 请求的 `backend` 执行体提示 SHALL 按以下语义处理：缺省（未携带字段）
默认路由到 ACP，保持旧客户端向后兼容；显式给出的值属于已知集合（`native`、
`acp`、`acp:<kind>` 前缀形式）时按既有路由语义分发；显式给出的值不属于已知
集合时 SHALL 返回 typed rejection 且 SHALL NOT 创建任何会话——未知执行体
SHALL NOT 静默回退为 ACP。

#### Scenario: 未知执行体提示被拒绝且不建会话

- **WHEN** 客户端以 `backend: "warp-drive"` 请求创建会话
- **THEN** 响应是 typed rejection（指名未知执行体），核心上不存在新会话

#### Scenario: 缺省提示仍默认 ACP

- **WHEN** 旧客户端发送不含 `backend` 字段的 Spawn 请求
- **THEN** 会话照常创建并路由到 ACP，行为与本变更前一致

#### Scenario: 显式 acp 与缺省等价

- **WHEN** 客户端以 `backend: "acp"` 请求创建会话
- **THEN** 会话创建并路由到 ACP，与缺省行为一致

### Requirement: Gated-call approval over the channel

The channel SHALL carry approval requests raised by native-kernel sessions —
gated tool calls awaiting an operator decision — to connected channel clients,
and SHALL carry the operator's decision (allow-once / allow-session / deny,
with an optional reason) back to the kernel's approver. A session whose
approval request has no reachable client SHALL fail closed: the call is not
executed. The approval surface SHALL be available in every process
configuration served through the channel, so a detached WebUI presents the
same review card as the in-process one.

#### Scenario: detached review card answers a gated call

- **WHEN** a native session requests approval for a gated tool call while a
  detached WebUI is connected to the channel
- **THEN** the review card is presented with the decision options, and an
  allow-once decision lets only that call through

#### Scenario: deny round-trips to the kernel

- **WHEN** the operator denies a channel-delivered approval with a reason
- **THEN** the kernel's approver receives the denial, the tool call is not
  executed, and the session reflects the rejection

#### Scenario: no reachable client fails closed

- **WHEN** a native session requests approval and no channel client is
  connected to receive it
- **THEN** the request is denied and the tool call is not executed

### Requirement: Session model selection over the channel

The channel SHALL provide a method to set the model of a known session,
returning either success or a typed rejection. Setting the model on a session
SHALL affect its subsequent turns regardless of the session's execution body.
A set-model request naming an unknown session, or a model the session's
execution body cannot serve, SHALL be rejected with a typed reason and nothing
SHALL change.

#### Scenario: model change reaches a native session

- **WHEN** a client sets a model on a native-kernel session over the channel
- **THEN** the session's current model updates and subsequent turns use it

#### Scenario: unknown session rejected

- **WHEN** a set-model request names a `ChannelKey` the core does not know
- **THEN** the response is a typed rejection and no session state changes

#### Scenario: unservable model rejected

- **WHEN** a client sets a model the session's execution body cannot serve
- **THEN** the response is a typed rejection naming the reason, and the
  session keeps its previous model

### Requirement: Channel auto-arm without injected secret

core SHALL 在 `SEBAS_CORE_SECRET` 缺失或为空时仍武装核心会话通道：现场生成随机 secret，并将其实时写入 secret 文件（路径从与 socket 一致的 config 解析，文件权限 0600）。env 显式提供 secret 时 SHALL 优先使用 env 值，且 secret 文件仍须写入（供迟启动的组件发现）。secret 文件写入 SHALL 原子替换（tmp + rename）。

#### Scenario: 无 env 启动时通道自动武装

- **WHEN** core 以不含 `SEBAS_CORE_SECRET` 的环境启动且 config 指向沙箱内 socket 路径
- **THEN** 通道 socket 在解析路径上出现，secret 文件在解析路径上出现且内容为本次启动的随机 secret，权限为 0600

#### Scenario: env 提供时 env 优先且文件仍落盘

- **WHEN** core 以非空 `SEBAS_CORE_SECRET` 启动
- **THEN** 通道以 env 值作握手 secret，secret 文件内容与 env 值一致

### Requirement: Secret file discovery for channel clients

通道客户端（webui、router 订阅、im）SHALL 按以下顺序解析握手 secret：`SEBAS_CORE_SECRET` env 优先；env 缺失时读取 secret 文件；两者皆缺省时以空 secret 连接并在启动时输出 warn（不静默）。当连接因 secret 不匹配失败且 env 未设时，客户端重连前 SHALL 重读 secret 文件——core 重启换钥后，已在运行的客户端 SHALL 在无人工干预下恢复连接。

#### Scenario: 独立 webui 无 env 经文件连上

- **WHEN** 独立 `sebas webui` 与 core 使用同一份 config，webui 环境无 `SEBAS_CORE_SECRET`
- **THEN** webui 读取 secret 文件完成握手，`/api/summary` 的 `reachability.ok` 为 true

#### Scenario: core 重启换钥后运行中的 webui 自愈

- **WHEN** core 被重启（生成新随机 secret 并覆写 secret 文件），webui 进程不重启
- **THEN** webui 在重连退避内恢复 `reachability.ok = true`，期间报过的 cause 如实反映 secret 不匹配或 socket 缺失

#### Scenario: env 与文件皆缺省时启动告警

- **WHEN** 通道客户端启动时 env 缺失且 secret 文件不存在
- **THEN** 客户端输出包含"核心通道 secret 未找到"语义的 warn 日志并继续以空 secret 尝试连接（不崩溃、不静默）

### Requirement: Channel bind failure is a hard startup failure

通道 socket bind 失败（路径被存活进程占用且拒绝回收等）SHALL 使 core 以 bind 失败退出码（与 webui 的 75 语义一致）退出，而不是继续运行一个无通道的"健康"进程。

#### Scenario: socket 被存活进程占用时启动失败

- **WHEN** 目标 socket 路径已被另一个存活 core 占用，新 core 启动
- **THEN** 新 core 以 bind 失败退出码退出，supervisor 可据此标记 Degraded 而非无限重启

### Requirement: State store channel surface
core session channel SHALL 暴露 state-store 引擎的 snapshot / mutation / subscribe 三类 RPC：`StateSnapshot { domain }` 返回该 domain 的当前快照，`StateMutation { domain, payload }` 应用变更，`StateSubscribe` 启动变更推送流；服务端 SHALL 把请求路由到 state-store engine 实例并以 `StateSnapshot / StateMutationOk` 帧回包。订阅流 SHALL 在首帧之后推送该 domain 的后续 mutation 帧，且 SHALL 在 mutation 失败时回 `Rejected { kind: ... }` typed rejection（不静默吞错）。三类 RPC SHALL 走与 session RPC 同样的 secret+peer-uid 鉴权。

#### Scenario: StateSnapshot returns current snapshot

- **WHEN** 客户端发 `StateSnapshot { domain: "agent-defaults" }`
- **THEN** 服务端以 `StateSnapshot { domain, payload }` 回包；payload 与 state-store engine 持有一致

#### Scenario: StateMutation applies change

- **WHEN** 客户端发 `StateMutation { domain: "agent-defaults", payload: {...} }`
- **THEN** 服务端应用变更、回 `StateMutationOk`；下一次 `StateSnapshot` 看到新值

#### Scenario: StateMutation rejected does not silently swallow

- **WHEN** `StateMutation` 携带非法 payload（如未知字段 / 类型错误）
- **THEN** 服务端回 `Rejected { kind: ... }` 含 typed rejection；engine 状态未变

#### Scenario: StateSubscribe delivers mutations after snapshot

- **WHEN** 客户端先发 `StateSubscribe { domain }`、收到首帧 snapshot；服务端在订阅期间应用 mutation
- **THEN** 客户端收到 mutation 帧（带 domain + payload）；滞后 SHALL 走 lag-disconnect 路径

### Requirement: Channel client reachability distinguishes startup failure
`reachability()`（channel client 侧）SHALL 区分三类不可达并经 `Reachability` 枚举显式表达：(a) **socket 不存在**（core 从未启动或启动失败退出，errno=ENOENT 形态）→ `Reachability::StartupFailed { cause }`；(b) **socket 在但握手被拒**（secret 错误）→ `Reachability::AuthRejected { cause }`；(c) **握手成功后断连** → `Reachability::Disconnected { cause }`。其中 `Reachability` 三变体 + `kind` 字段是本 change 新增（fail-fast 只落地了 cause 字符串富化，未做三态枚举——owner 归属见本 change design D3）。cause 富化沿用已落地的无条件 enrich（client.rs `enrich_with_startup_summary`）：闩锁文件存在时 cause 形如 `core startup failed: <原因>`（前缀由后端拼，前端原文渲染）；`kind` 字段是前端区分文案的机器可读依据。三类 SHALL NOT 互相混淆——webui 不允许统一显示 "core is not connected"。

#### Scenario: socket-not-found reports startup failure with SEBAS_STARTUP_ERROR_FILE

- **WHEN** channel socket 路径不存在、且 `SEBAS_STARTUP_ERROR_FILE` 指向的文件含 `startup-failure: <原因>` 一行
- **THEN** `Reachability::StartupFailed { cause }` 返回；cause SHALL 为 `core startup failed: <原因>` 全串（与已落地的 enrich 行为一致）；`kind` SHALL 为 `startup_failed`

#### Scenario: socket-not-found fallback cause

- **WHEN** channel socket 路径不存在、`SEBAS_STARTUP_ERROR_FILE` 未设置或文件不存在
- **THEN** `Reachability::StartupFailed { cause }` 返回；cause SHALL 为 `core session channel socket not found at <path>`；`kind` SHALL 为 `startup_failed`

#### Scenario: handshake auth failure reports AuthRejected

- **WHEN** channel socket 在、客户端发握手但 secret 错误
- **THEN** `Reachability::AuthRejected { cause: "core rejected channel handshake" }` 返回；`kind` SHALL 为 `auth_rejected`；客户端 SHALL NOT 用同一 secret 无限重试——env 未设时 SHALL 重读 secret 文件后至多再试一次（core 重启换钥自愈，见 Secret file discovery 规约），之后仍失败则保持 AuthRejected

#### Scenario: post-handshake disconnect reports Disconnected

- **WHEN** 客户端已握手成功、之后 socket read 返回 0 / EOF
- **THEN** `Reachability::Disconnected { cause }` 返回；`kind` SHALL 为 `disconnected`；客户端 SHALL 触发重连（按已有规约）

### Requirement: ensure_message IM 投递语义
`EnsureMessage` SHALL 走 IM 投递语义：未知 key 按入站文本历史语义自动建会话（dormant 占位或 active 新建由 core 内部策略决定）、dormant 会话懒复活；已知 active key 等价 `Message`。与 `Message` 的差别 SHALL 仅在服务端跳过存在性预检——webui 的"未知即拒绝"语义 SHALL NOT 受影响（webui 不应直接发 `EnsureMessage`）。channel 单测 SHALL 覆盖：(a) 未知 key 时 core 自动创建并返回；(b) dormant key 复活成功；(c) `Message` 在未知 key 上得到 typed rejection `UnknownSession`。

#### Scenario: unknown key auto-creates session via EnsureMessage

- **WHEN** 客户端发 `EnsureMessage { key: <未知>, message: "hi" }`
- **THEN** core 自动创建会话；客户端收到 `Ok`；后续 `Snapshot` 包含该会话

#### Scenario: dormant session resumes via EnsureMessage

- **WHEN** 客户端发 `EnsureMessage { key: <dormant>, message: "hi" }`
- **THEN** core 懒复活会话；客户端收到 `Ok`；订阅流推送 `Updated` 事件（无独立 `Revived` 帧）

#### Scenario: Message on unknown key is rejected

- **WHEN** 客户端发 `Message { key: <未知>, message: "hi" }`
- **THEN** core 回 `Rejected { kind: UnknownSession }`；不创建任何会话

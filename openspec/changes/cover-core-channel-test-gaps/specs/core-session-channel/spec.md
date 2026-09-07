## ADDED Requirements

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
`CoreChannelBackend::connect` SHALL 区分两类不可达情形：(a) **socket 不存在**（core 从未启动，errno=ENOENT 或 ENOENT 形态）→ 报告 `Reachability::StartupFailed { cause: "<可读摘要>" }`，cause 优先取 `SEBAS_STARTUP_ERROR_FILE`（若存在）首行；fallback 为 `core session channel socket not found`；(b) **socket 在但 secret 拒或 peer uid 不匹配** → 报告 `Reachability::AuthRejected { cause }`；(c) **socket 在、握手成功、之后断连** → 报告 `Reachability::Disconnected { cause }`。三类 SHALL NOT 互相混淆——webui 必须能在 banner 上分别显示 "core startup failed" / "core auth rejected" / "core disconnected"，不允许统一显示 "core is not connected"。

#### Scenario: socket-not-found reports startup failure with SEBAS_STARTUP_ERROR_FILE

- **WHEN** channel socket 路径不存在、且 `SEBAS_STARTUP_ERROR_FILE` 指向的文件含 `startup-failure: <原因>` 一行
- **THEN** `Reachability::StartupFailed { cause }` 返回；cause 内容 SHALL 等于该文件首行（去除前后空白）

#### Scenario: socket-not-found fallback cause

- **WHEN** channel socket 路径不存在、`SEBAS_STARTUP_ERROR_FILE` 未设置或文件不存在
- **THEN** `Reachability::StartupFailed { cause }` 返回；cause SHALL 为 `core session channel socket not found at <path>`

#### Scenario: handshake auth failure reports AuthRejected

- **WHEN** channel socket 在、客户端发握手但 secret 错误
- **THEN** `Reachability::AuthRejected { cause: "core rejected channel handshake" }` 返回；客户端 SHALL NOT 自动重试 secret 错误

#### Scenario: post-handshake disconnect reports Disconnected

- **WHEN** 客户端已握手成功、之后 socket read 返回 0 / EOF
- **THEN** `Reachability::Disconnected { cause }` 返回；客户端 SHALL 触发重连（按已有规约）

### Requirement: ensure_message IM 投递语义
`EnsureMessage` SHALL 走 IM 投递语义：未知 key 按入站文本历史语义自动建会话（dormant 占位或 active 新建由 core 内部策略决定）、dormant 会话懒复活；已知 active key 等价 `Message`。与 `Message` 的差别 SHALL 仅在服务端跳过存在性预检——webui 的"未知即拒绝"语义 SHALL NOT 受影响（webui 不应直接发 `EnsureMessage`）。channel 单测 SHALL 覆盖：(a) 未知 key 时 core 自动创建并返回；(b) dormant key 复活成功；(c) `Message` 在未知 key 上得到 typed rejection `UnknownSession`。

#### Scenario: unknown key auto-creates session via EnsureMessage

- **WHEN** 客户端发 `EnsureMessage { key: <未知>, message: "hi" }`
- **THEN** core 自动创建会话；客户端收到 `Ok`；后续 `Snapshot` 包含该会话

#### Scenario: dormant session resumes via EnsureMessage

- **WHEN** 客户端发 `EnsureMessage { key: <dormant>, message: "hi" }`
- **THEN** core 懒复活会话；客户端收到 `Ok`；订阅流推送 `Revived` 事件

#### Scenario: Message on unknown key is rejected

- **WHEN** 客户端发 `Message { key: <未知>, message: "hi" }`
- **THEN** core 回 `Rejected { kind: UnknownSession }`；不创建任何会话

## MODIFIED Requirements

### Requirement: Channel transport and authentication
The core SHALL expose the channel on a Unix domain socket created with owner-only permissions (0600) at a configurable path defaulting to `~/.sebas/core.sock`. Every connection SHALL be authenticated by both peer credentials — the connecting uid MUST equal the core's own uid — and a shared secret supplied out of band, in the same posture as the watchdog control RPC. A connection failing either check SHALL be rejected and closed without processing any request. The channel SHALL NOT be exposed over TCP. **补充**：peer-uid 与 secret 鉴权 SHALL 经由真实跨 uid 进程（不仅同 uid 单测）验证——本期单测覆盖 `cross_uid_rejected` 路径。

#### Scenario: foreign uid rejected

- **WHEN** a process running as a different uid connects to the socket
- **THEN** the connection is rejected and closed, and no request on it is processed

#### Scenario: missing or wrong secret rejected

- **WHEN** a connection does not supply the agreed secret, or supplies a different one
- **THEN** the connection is closed without a response

#### Scenario: cross_uid_rejected_live_process

- **WHEN** 跨 uid 进程（不是 `setuid` 模拟而是真正 fork + setuid 到不同账户）尝试连接 core socket 并发请求
- **THEN** 连接被拒绝；服务端 SHALL 在日志写 peer-uid 不匹配记录（`warn!("core channel: peer uid mismatch; closing")`）；不进入 request 处理

#### Scenario: stale socket file is reclaimed

- **WHEN** the core starts and a socket file already exists at the path with no
  live listener behind it
- **THEN** the core removes the stale file and binds a fresh socket

### Requirement: Honest degradation when the core is unreachable
When the channel cannot be reached — socket absent, connection refused, secret rejected, or the connection dropped — a client SHALL surface that condition with its cause and SHALL NOT present stale data as current, report a mutation as succeeded, or offer a control whose request cannot be delivered. A client SHALL reconnect on its own and resume with a fresh snapshot when the core returns. **补充**：当 socket 不存在且 `SEBAS_STARTUP_ERROR_FILE` 存在时，webui SHALL 读取该文件首行作为 cause，呈现 "core startup failed: <可读原因>" banner；不为空字符串或空 cause 时 SHALL 区分三类不可达（startup-failed / auth-rejected / disconnected）。

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
- **THEN** webui degradation banner SHALL 显示 "core startup failed: <原因>"；`/api/summary.reachability.ok` 为 false 且 cause 字段 SHALL 等于 `SEBAS_STARTUP_ERROR_FILE` 首行（不含「core startup failed」字面量，由前端拼接）；runtime disconnect 走 banner "core is not connected" 区分路径
## MODIFIED Requirements

### Requirement: Service lifecycle

The watchdog SHALL manage auxiliary services — the WebUI child, the router child, and the IM child — as supervised child processes spawned from the same binary (`sebas webui --config <path>` / `sebas router --config <path>` / `sebas im --config <path>`, each given the control secret). The WebUI child SHALL be spawned when `[service.webui] enabled = true`; the router child SHALL be spawned only when router management is explicitly enabled in the service config (default off — existing deployments see no new process until they opt in); the IM child SHALL be spawned when `[watchdog.im] enabled = true`, whose default SHALL follow the feishu enablement decision (`[feishu] enabled` or its implicit fallback) so that feishu deployments gain the IM service without a new config key. With `--debug`, the watchdog additionally spawns the debug router child (`sebas router --debug`) as today.

Auxiliary children SHALL survive core restarts (only the core child is respawned by an upgrade), and an auxiliary child that exits SHALL itself be restarted per the crash-backoff policy. `ServiceStatus` and `ServiceStatusFor` SHALL report each managed service's actual observed state (running / restarting / stopped / disabled) derived from process liveness and desired state — never a synthesized or hardcoded value.

`ServiceSet { service, desired, persist, force }` and `ServiceRestart { service }` SHALL execute for the auxiliary services (`webui`, `router`, `im`): `desired` ∈ {on, off} stops or starts the child; `persist: true` records the desired state so it survives a watchdog restart, `persist: false` scopes it to the current watchdog run; `force` (default `false`) is meaningful only for stopping the router (see below). `ServiceSet` or `ServiceRestart` naming the core service SHALL be rejected with an actionable error pointing at `RestartCore` (core restarts flow exclusively through the confirmed dangerous-action path).

Stopping the router child via `ServiceSet { service: "router", desired: "off" }` SHALL be refused while at least one session is actively routed — a non-terminal session whose effective provider mode is Router — with a rejection carrying the active-session count; `force: true` SHALL bypass this protection. The active-session truth SHALL come from the core state store over the session channel; when the core channel is unreachable the stop SHALL proceed (no core means no live routed streams). External CLI consumers dialing the router directly are not covered by this protection and SHALL be documented as such.

#### Scenario: webui survives core restart

- **WHEN** the core child crashes and is restarted by the watchdog
- **THEN** the WebUI child process is untouched

#### Scenario: webui enabled by default

- **WHEN** the config has no `[service.webui]` section
- **THEN** the watchdog spawns the WebUI child (webui 是 watchdog 唯一默认
启动的服务，enable-core-by-default 后 core 亦恒启）

#### Scenario: crashed webui is restarted

- **WHEN** the WebUI child process exits unexpectedly
- **THEN** the watchdog restarts it after the crash-backoff delay and its
reported status reflects the restart in progress

#### Scenario: status reflects reality

- **WHEN** the WebUI child is killed and `ServiceStatus` is queried before
the restart completes
- **THEN** the webui entry reports a non-running state rather than
"running"

#### Scenario: service set toggles auxiliary service

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "off", persist: false }`
- **THEN** the response is `Accepted` and the WebUI child stops; a
subsequent `ServiceStatus` reports webui as stopped with desired state off

#### Scenario: service set persisted across watchdog restart

- **WHEN** `ServiceSet { service: "router", desired: "on", persist: true }`
is accepted and the watchdog is later restarted
- **THEN** the watchdog spawns the router child again without a new
`ServiceSet`

#### Scenario: service commands on core are rejected

- **WHEN** a client sends `ServiceRestart { service: "core" }`
- **THEN** the response is `Rejected` with an actionable message pointing
to `RestartCore`

#### Scenario: router stop refused with active routed sessions

- **WHEN** `ServiceSet { service: "router", desired: "off" }` arrives while
one or more non-terminal sessions run with provider mode Router
- **THEN** the response is `Rejected` carrying the active-session count,
and the router child keeps running

#### Scenario: force bypasses router stop protection

- **WHEN** `ServiceSet { service: "router", desired: "off", force: true }`
arrives in the same situation
- **THEN** the response is `Accepted` and the router child stops

#### Scenario: core unreachable allows router stop

- **WHEN** the core channel is unreachable and router stop is requested
- **THEN** the stop proceeds (no core means no live routed streams)

#### Scenario: legacy watchdog config keys warned and ignored

- **WHEN** the config still carries a `[watchdog.core]`, `[watchdog.webui]`, or `[watchdog.router]` section
- **THEN** the config loader logs a deprecation warning naming the replacement key, ignores the section, and starts with defaults for those sub-tables


### Requirement: Managed service table
The watchdog SHALL supervise all child processes through one declarative table of managed services — core, webui, router, and im — where each entry declares its spawn specification (argv, env), its desired state (from config or `ServiceSet`), and its restart policy. **补充**：受管服务名在 RPC/REST 接口与 control RPC 中 SHALL 统一使用 `core` / `webui` / `router` / `im` 四个字符串（产品对外 IM 即「飞书」，但内部字符串 SHALL 为 `im` 以与 `ServiceName::Im` 枚举对齐）；webui 前端 SHALL 通过 `service_from_str("im")` 识别，禁止使用 `feishu` 作为内部名。监督循环 SHALL 维持对全部 entry 的均匀处理（spawn / 退出分类 / 重启决策），New-binary auto-rollback 仍是 core 专属。config 中 disabled 的服务 SHALL 无子进程被拉起、状态报告为 `disabled`。core 子进程的 pipe 协议 SHALL 仅承载 readiness 握手（单一 `Ready` 帧，无错误行——失败经退出码分类），控制操作不再经 pipe，control RPC socket 是唯一命令面。**补充（enable-core-by-default）**：core 为恒启 entry——无 `[service.core] enabled` 开关，watchdog SHALL 无条件拉起 core 并监督之；`ServiceSet`/`ServiceRestart` 命名 core SHALL 一律拒绝（指向 `RestartCore`），config 层与 `services.json` 覆盖层对 core SHALL 被忽略（历史 `core` 覆盖告警后弃用）。

#### Scenario: router managed when enabled

- **WHEN** the watchdog config enables router management
- **THEN** the watchdog spawns `sebas router --config <path>` as a supervised child and `ServiceStatus` includes a real router entry

#### Scenario: im managed when enabled

- **WHEN** the watchdog config enables IM management (explicitly or via the feishu-enablement default)
- **THEN** the watchdog spawns `sebas im --config <path>` with the control secret and `ServiceStatus` includes a real im entry

#### Scenario: core managed unconditionally

- **WHEN** the watchdog runs (any config, any services.json override)
- **THEN** the watchdog spawns and supervises the core child, and `ServiceStatus` includes a real core entry

#### Scenario: disabled service reports disabled

- **WHEN** `ServiceStatus` is queried while webui management is disabled in config
- **THEN** the webui entry reports state `disabled` and no child exists

#### Scenario: upgrade commands only via RPC

- **WHEN** the core child wants an upgrade or rollback executed
- **THEN** it sends the request over the control RPC socket; the pipe carries only the readiness handshake

#### Scenario: im 字符串命名约定

- **WHEN** webui 通过 `/api/admin/services` 列出受管进程
- **THEN** IM/飞书服务的 name 字段 SHALL 为 `im`（不是 `feishu`）；前端识别 SHALL 经 `service_from_str("im")`，使用 `feishu` 调用将得到 `UnknownService` 错误

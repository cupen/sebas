## MODIFIED Requirements

### Requirement: Service lifecycle

The watchdog SHALL manage auxiliary services — the WebUI child, the router
child, and the IM child — as supervised child processes spawned from the same
binary (`sebas webui --config <path>` / `sebas router --config <path>` /
`sebas im --config <path>`, each given the control secret). The WebUI child
SHALL be spawned when `[watchdog.webui] enabled = true`; the router child
SHALL be spawned only when router management is explicitly enabled in the
watchdog config (default off — existing deployments see no new process until
they opt in); the IM child SHALL be spawned when `[watchdog.im] enabled = true`,
whose default SHALL follow the feishu enablement decision (`[feishu] enabled`
or its implicit fallback) so that feishu deployments gain the IM service
without a new config key. With `--debug`, the watchdog additionally spawns the
debug router child (`sebas router --debug`) as today.

Auxiliary children SHALL survive core restarts (only the core child is
respawned by an upgrade), and an auxiliary child that exits SHALL itself be
restarted per the crash-backoff policy. `ServiceStatus` and
`ServiceStatusFor` SHALL report each managed service's actual observed
state (running / restarting / stopped / disabled) derived from process
liveness and desired state — never a synthesized or hardcoded value.

`ServiceSet { service, desired, persist, force }` and
`ServiceRestart { service }` SHALL execute for the auxiliary services
(`webui`, `router`, `im`): `desired` ∈ {on, off} stops or starts the child;
`persist: true` records the desired state so it survives a watchdog restart,
`persist: false` scopes it to the current watchdog run; `force` (default
`false`) is meaningful only for stopping the router (see below).
`ServiceSet` or `ServiceRestart` naming the core service SHALL be rejected
with an actionable error pointing at `RestartCore` (core restarts flow
exclusively through the confirmed dangerous-action path).

Stopping the router child via `ServiceSet { service: "router", desired:
"off" }` SHALL be refused while at least one session is actively routed —
a non-terminal session whose effective provider mode is Router — with a
rejection carrying the active-session count; `force: true` SHALL bypass
this protection. The active-session truth SHALL come from the core state
store over the session channel; when the core channel is unreachable the
stop SHALL proceed (no core means no live routed streams). External CLI
consumers dialing the router directly are not covered by this protection
and SHALL be documented as such.

#### Scenario: webui survives core restart

- **WHEN** the core child crashes and is restarted by the watchdog
- **THEN** the WebUI child process is untouched

#### Scenario: webui enabled by default

- **WHEN** the config has no `[watchdog.webui]` section
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

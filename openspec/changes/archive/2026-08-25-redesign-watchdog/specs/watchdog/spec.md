## MODIFIED Requirements

### Requirement: Service lifecycle

The watchdog SHALL manage auxiliary services — the WebUI child and the
gateway child — as supervised child processes spawned from the same binary
(`sebas webui --config <path>` / `sebas gateway --config <path>`, each given
the control secret). The WebUI child SHALL be spawned when
`[watchdog.webui] enabled = true`; the gateway child SHALL be spawned only
when gateway management is explicitly enabled in the watchdog config
(default off — existing deployments see no new process until they opt in).
With `--debug`, the watchdog additionally spawns the debug gateway child
(`sebas gateway --debug`) as today.

Auxiliary children SHALL survive core restarts (only the core child is
respawned by an upgrade), and an auxiliary child that exits SHALL itself be
restarted per the crash-backoff policy. `ServiceStatus` and
`ServiceStatusFor` SHALL report each managed service's actual observed
state (running / restarting / stopped / disabled) derived from process
liveness and desired state — never a synthesized or hardcoded value.

`ServiceSet { service, desired, persist }` and
`ServiceRestart { service }` SHALL execute for the auxiliary services:
`desired` ∈ {on, off} stops or starts the child; `persist: true` records
the desired state so it survives a watchdog restart, `persist: false`
scopes it to the current watchdog run. `ServiceSet` or `ServiceRestart`
naming the core service SHALL be rejected with an actionable error pointing
at `RestartCore` (core restarts flow exclusively through the confirmed
dangerous-action path).

#### Scenario: webui survives core restart

- **WHEN** the core child crashes and is restarted by the watchdog
- **THEN** the WebUI child process is untouched

#### Scenario: webui disabled by default

- **WHEN** the config has no `[watchdog.webui]` section
- **THEN** the watchdog spawns no WebUI child

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

- **WHEN** `ServiceSet { service: "gateway", desired: "on", persist: true }`
  is accepted and the watchdog is later restarted
- **THEN** the watchdog spawns the gateway child again without a new
  `ServiceSet`

#### Scenario: service commands on core are rejected

- **WHEN** a client sends `ServiceRestart { service: "core" }`
- **THEN** the response is `Rejected` with an actionable message pointing
  to `RestartCore`

### Requirement: Control request surface

The RPC SHALL serve: `Status`, `EventsSince`, `Update`, `Rollback`,
`RestartCore`, `ServiceStatus`, `ServiceStatusFor`, `ServiceSet`,
`ServiceRestart`, `Confirm`, and `Cancel`. `ServiceSet` and
`ServiceRestart` SHALL act on the auxiliary managed services (webui,
gateway) as specified in the Service lifecycle requirement; requests naming
the core service SHALL be rejected with an actionable error.
`Confirm` and `Cancel` SHALL be accepted only from a Feishu actor with a
`chat_id`; any other actor gets `unauthorized`.

#### Scenario: service set accepted

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "off" }`
- **THEN** the response is `Accepted` and the WebUI child stops

#### Scenario: service set rejected

- **WHEN** a client sends `ServiceSet { service: "core", desired: "off" }`
- **THEN** the response is `Rejected` with an actionable message pointing
  to `RestartCore` (core lifecycle is supervised, not user-toggled)

#### Scenario: cli cannot confirm

- **WHEN** a `Cli` actor sends `Confirm { token }`
- **THEN** the response is `Rejected { code: "unauthorized" }`

### Requirement: Crash backoff

The crash counter SHALL apply per managed service (core, webui, gateway),
each with an independent counter that resets when that service's previous
crash was more than 1 h ago. Restarts proceed while the service's crash
count is at or below the maximum (3); once exceeded, the watchdog sleeps
30 s, resets that service's counter, and keeps looping — the watchdog never
exits due to a crashing child.

#### Scenario: crash limit cycling

- **WHEN** the core crashes a 4th time within the window
- **THEN** the watchdog sleeps 30 s, resets the core's counter, and
  continues supervising

#### Scenario: independent per-service counters

- **WHEN** the webui child crashes twice and the core child crashes once
  within the window
- **THEN** the core's crash count is 1 (not 3), and both services are
  restarted per their own counters

## ADDED Requirements

### Requirement: Managed service table

The watchdog SHALL supervise all child processes through one declarative
table of managed services — core, webui, and gateway — where each entry
declares its spawn specification (argv, env), its desired state (from
config or `ServiceSet`), and its restart policy. The supervision loop SHALL
treat every entry uniformly for spawn, exit classification, and restart
decisions; the New-binary auto-rollback classification remains a core-only
refinement. A service disabled in config SHALL have no child spawned and
SHALL report state `disabled`.

The core child's pipe protocol SHALL consist of the readiness handshake
(and early fatal-error lines before readiness); control operations no
longer travel over the pipe — the control RPC socket is the sole command
surface.

#### Scenario: gateway managed when enabled

- **WHEN** the watchdog config enables gateway management
- **THEN** the watchdog spawns `sebas gateway --config <path>` as a
  supervised child and `ServiceStatus` includes a real gateway entry

#### Scenario: disabled service reports disabled

- **WHEN** `ServiceStatus` is queried while webui management is disabled
  in config
- **THEN** the webui entry reports state `disabled` and no child exists

#### Scenario: upgrade commands only via RPC

- **WHEN** the core child wants an upgrade or rollback executed
- **THEN** it sends the request over the control RPC socket; the pipe
  carries only the readiness handshake

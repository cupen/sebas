## MODIFIED Requirements

### Requirement: Control request surface

The RPC SHALL serve: `Status`, `EventsSince`, `Update`, `Rollback`,
`RestartCore`, `ServiceStatus`, `ServiceStatusFor`, `ServiceSet`,
`ServiceRestart`, `Confirm`, and `Cancel`. `ServiceSet` and
`ServiceRestart` SHALL act on the auxiliary managed services (webui,
router) as specified in the Service lifecycle requirement; requests naming
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

### Requirement: Managed service table

The watchdog SHALL supervise all child processes through one declarative
table of managed services — core, webui, and router — where each entry
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

#### Scenario: router managed when enabled

- **WHEN** the watchdog config enables router management
- **THEN** the watchdog spawns `sebas router --config <path>` as a
  supervised child and `ServiceStatus` includes a real router entry

#### Scenario: disabled service reports disabled

- **WHEN** `ServiceStatus` is queried while webui management is disabled
  in config
- **THEN** the webui entry reports state `disabled` and no child exists

#### Scenario: upgrade commands only via RPC

- **WHEN** the core child wants an upgrade or rollback executed
- **THEN** it sends the request over the control RPC socket; the pipe
  carries only the readiness handshake

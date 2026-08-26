## MODIFIED Requirements

### Requirement: Watchdog lifecycle ownership

The WebUI SHALL be spawned by the watchdog as a separate process (with the
control secret) by default — `[watchdog.webui] enabled` defaults to `true`
unless explicitly set to `false` — and SHALL survive core restarts. The
WebUI SHALL bind to `127.0.0.1:9797` by default; port conflict with a
legacy `run --webui` (or any other process) is resolved by kernel-level
bind atomicity — the first to bind wins, the second bind fails with a
distinct exit code.

#### Scenario: single owner

- **WHEN** the watchdog-spawned WebUI is running and a legacy
  `run --webui` is attempted
- **THEN** the second start is refused by the ownership guard (port
  already bound)

#### Scenario: default enablement

- **WHEN** the watchdog starts with a configuration that contains no
  `[watchdog.webui]` section
- **THEN** the watchdog spawns and supervises the WebUI child process

#### Scenario: explicit disable

- **WHEN** the configuration sets `[watchdog.webui] enabled = false`
- **THEN** the watchdog does not spawn the WebUI and reports it as a
  disabled service

## ADDED Requirements

### Requirement: WebUI bind failure exit code

The WebUI child process SHALL exit with a reserved exit code
(`EXIT_BIND_FAILED = 75`) when it fails to bind to the configured
address, so the watchdog supervisor can distinguish bind failures from
other crashes. The supervisor SHALL recognize this code, log a warning
naming the service, and mark the WebUI service as `Degraded` instead of
retrying.

#### Scenario: port already occupied

- **WHEN** the watchdog starts and `127.0.0.1:9797` is already bound by
  another process
- **THEN** the WebUI child exits with code 75, the supervisor logs a
  warning naming the WebUI service, and reports the state as `Degraded`

#### Scenario: recovery via restart

- **WHEN** the WebUI is `Degraded` due to a port conflict, the blocking
  process exits, and a control-plane request restarts the WebUI service
- **THEN** the WebUI binds successfully, the supervisor reports `Running`

#### Scenario: non-bind crash is not degraded

- **WHEN** the WebUI child exits with a code other than 75
- **THEN** the supervisor treats it as a normal crash and retries with
  backoff

### Requirement: Supervisor Degraded state

The `ServiceState` enum SHALL have a `Degraded` variant. When a service
enters `Degraded`, the supervisor SHALL stop spawning and wait for either
a `Restart` or `Stop` command. A `Restart` command SHALL reset the
service back to `Restarting` and attempt a new spawn.

#### Scenario: degraded service does not auto-retry

- **WHEN** a service is in `Degraded` state
- **THEN** the supervisor does not call `spawn()` again until a `Restart`
  command is received

#### Scenario: restart clears degraded

- **WHEN** a degraded service receives a `Restart` command
- **THEN** the supervisor sets the state to `Restarting` and calls
  `spawn()`

### Requirement: Bare run SHALL NOT start WebUI

The `sebas run` command SHALL NOT start the WebUI server unless the
`--webui` flag is passed. The `--no-webui` flag SHALL be accepted as an
explicit no-op for symmetry with the watchdog default, and SHALL be
mutually exclusive with `--webui`.

#### Scenario: bare run without flag

- **WHEN** `sebas run` is executed without `--webui`
- **THEN** no WebUI server starts

#### Scenario: --no-webui is accepted

- **WHEN** `sebas run --no-webui` is executed
- **THEN** no WebUI server starts, and the command proceeds normally

#### Scenario: --webui and --no-webui conflict

- **WHEN** both `--webui` and `--no-webui` are passed
- **THEN** the CLI parser rejects the command with an error
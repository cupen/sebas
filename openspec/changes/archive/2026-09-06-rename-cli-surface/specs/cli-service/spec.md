## RENAMED Requirements

- FROM: `### Requirement: Env-only bootstrap for run`
- TO: `### Requirement: Env-only bootstrap for core`

## MODIFIED Requirements

### Requirement: Subcommand tree

`sebas` SHALL expose the subcommands: `core` (long-lived core service),
`run` (the watchdog daemon), `router` (model router), `service`
(install/uninstall systemd unit), `replay` (offline event replay), `record`
(ACP stdio fixture capture), `webui` (dashboard server), `update` (one-shot
updater), `control` (control-plane client) — plus the aliases `status`
(= `control status`), `services` (= `control services`), `ctl` (= `control`),
and the hidden compatibility aliases `watchdog` (= `run`) and `gateway`
(= `router`). The hidden aliases keep pre-rename invocations — notably
already-installed systemd units whose `ExecStart` names `watchdog` — working
unchanged. Invoking bare `sebas` with no subcommand is a parse error; the
core never runs by default.

#### Scenario: bare invocation rejected

- **WHEN** the user runs `sebas` with no arguments
- **THEN** the CLI prints a usage error and exits nonzero without starting
  any service

#### Scenario: status alias

- **WHEN** the user runs `sebas status --secret ...`
- **THEN** the command behaves as `sebas control status`

#### Scenario: watchdog alias starts the daemon

- **WHEN** the user runs `sebas watchdog --config <config>` (an
  already-installed unit's `ExecStart` form)
- **THEN** the watchdog daemon starts exactly as `sebas run --config
  <config>` would

### Requirement: Service unit generation

`sebas service --install` SHALL write a systemd **system** unit to
`/etc/systemd/system/sebas.service` with: `After=`/`Wants=`
`network-online.target`, `Type=simple`, `User=`/`Group=` set to the
`--user` value (the service never runs as root). The unit's `ExecStart`
SHALL run the **run** entrypoint — the watchdog daemon
(`<fixed-binary> run --config <absolute config>`) — not the bare core. The
fixed binary path SHALL be `<data_dir>/bin/sebas`, where `data_dir` is
resolved from the config's `[watchdog.storage].data_dir` and falls back to
the `--user` home-derived data dir when unset. At install time the current
binary SHALL be seeded to `<data_dir>/bin/sebas`; update replaces that file
in place so a machine reboot runs the latest version. Installation requires
EUID 0. The unit SHALL include hardening directives `NoNewPrivileges`,
`ProtectSystem=full`, `ProtectHome=read-only`, and `PrivateTmp`; it SHALL
NOT set `ProtectSystem=strict` (self-upgrade must be able to replace the
binary). The rendered `ExecStart` paths SHALL be systemd-escaped so paths
containing whitespace or special characters remain valid. `RUST_LOG` SHALL
be taken from `--log-level` when given, otherwise inherited from the
installing environment (falling back to `info` when unset/empty). A
best-effort symlink at `/usr/local/bin/sebas` SHALL point to the fixed
binary path (failure to create it is not an error). `Restart=on-failure`,
`RestartSec=5`, `WantedBy=multi-user.target` are unchanged.

#### Scenario: unit content

- **WHEN** installing as root with `--user sebas`
- **THEN** the unit runs `<data_dir>/bin/sebas run --config <config>` under
  the `sebas` user, restarts on failure after 5 s, and boots at multi-user
  target

#### Scenario: privilege required

- **WHEN** `sebas service --install` runs as a non-root user
- **THEN** the command exits with code 4 without writing anything

#### Scenario: fixed binary seeded at install

- **WHEN** installing when `<data_dir>/bin/sebas` does not yet exist
- **THEN** the current binary is copied there and the unit's `ExecStart`
  references it

#### Scenario: paths with spaces escaped

- **WHEN** the config path contains a space
- **THEN** the rendered `ExecStart` is quoted/escaped and systemd parses it
  as a single argument

### Requirement: Config discovery

Every subcommand SHALL take an explicit `--config`/`-c` path defaulting to
`./config.toml` relative to the working directory; there is no multi-path
search and no `SEBAS_CONFIG` environment override.

#### Scenario: default path

- **WHEN** `sebas core` is invoked from a directory containing
  `config.toml`
- **THEN** that file is loaded without any flag

### Requirement: Env-only bootstrap for core

When the `core` config file is unreadable or absent, `sebas core` SHALL
fabricate a minimal config from `SEBAS_FEISHU_APP_ID` and
`SEBAS_FEISHU_APP_SECRET` (with a placeholder owner) instead of failing —
enabling container/env-driven deployments with no config file.

#### Scenario: no config file

- **WHEN** `sebas core --config ./missing.toml` runs with both Feishu env
  vars set
- **THEN** the core starts using the env-provided credentials

### Requirement: Config precedence and environment variables

Configuration precedence SHALL be: environment overrides over TOML over
built-in defaults. The env override set comprises `SEBAS_FEISHU_APP_ID`,
`SEBAS_FEISHU_APP_SECRET`, `SEBAS_LOG_LEVEL` (empty values ignored so a
blank variable never blanks a configured credential). Additionally
`SEBAS_CONTROL_SOCKET` and `SEBAS_CONTROL_SECRET` feed the control client,
`SEBAS_IPC` marks watchdog supervision, `SEBAS_ROUTER_PROVIDER_OVERLAY`
overrides the router's provider overlay, and `RUST_LOG` drives tracing for
the router/webui/watchdog entrypoints (the core filters on `[log] level`).
The pre-rename names `SEBAS_GATEWAY_PROVIDER_OVERLAY`,
`SEBAS_AGENT_GATEWAY_URL`, and `SEBAS_AGENT_GATEWAY_AUTH` SHALL remain
honored when their new names (`SEBAS_ROUTER_PROVIDER_OVERLAY`,
`SEBAS_AGENT_ROUTER_URL`, `SEBAS_AGENT_ROUTER_AUTH`) are unset — for one
release window, with a deprecation warning naming the replacement.

#### Scenario: env satisfies required field

- **WHEN** the TOML omits `feishu.app_secret` but
  `SEBAS_FEISHU_APP_SECRET` is set
- **THEN** parsing succeeds

#### Scenario: empty env ignored

- **WHEN** `SEBAS_FEISHU_APP_ID=""` is exported and the TOML has an app id
- **THEN** the TOML value is kept

#### Scenario: legacy router env still honored

- **WHEN** only `SEBAS_GATEWAY_PROVIDER_OVERLAY` is set
- **THEN** the router uses its value as the provider overlay and logs a
  deprecation warning pointing at `SEBAS_ROUTER_PROVIDER_OVERLAY`

## Purpose

Defines the command-line surface of the `sebas` binary: the subcommand
tree, the systemd service installation semantics (unit generation, privilege
drop, exit codes), configuration file discovery, environment-variable
overrides, and the control-plane client.

## ADDED Requirements

### Requirement: Subcommand tree

`sebas` SHALL expose the subcommands: `run` (long-lived core), `service`
(install/uninstall systemd unit), `replay` (offline event replay), `record`
(ACP stdio fixture capture), `gateway` (LLM gateway), `webui` (dashboard
server), `watchdog` (daemon), `update` (one-shot updater), `control`
(control-plane client) — plus the aliases `status` (= `control status`),
`services` (= `control services`), and `ctl` (= `control`). Invoking bare
`sebas` with no subcommand is a parse error; the core never runs by
default.

#### Scenario: bare invocation rejected

- **WHEN** the user runs `sebas` with no arguments
- **THEN** the CLI prints a usage error and exits nonzero without starting
  any service

#### Scenario: status alias

- **WHEN** the user runs `sebas status --secret ...`
- **THEN** the command behaves as `sebas control status`

### Requirement: Service unit generation

`sebas service --install` SHALL write a systemd **system** unit to
`/etc/systemd/system/sebas.service` with: `After=`/`Wants=`
`network-online.target`, `Type=simple`, `User=`/`Group=` set to the
`--user` value (the service never runs as root), `RUST_LOG` baked from the
installing environment, `ExecStart=<absolute binary> run --config <absolute
config>`, `Restart=on-failure`, `RestartSec=5`, `WantedBy=multi-user.target`.
Installation requires EUID 0.

#### Scenario: unit content

- **WHEN** installing as root with `--user sebas`
- **THEN** the unit runs the binary under the `sebas` user, restarts on
  failure after 5 s, and boots at multi-user target

#### Scenario: privilege required

- **WHEN** `sebas service --install` runs as a non-root user
- **THEN** the command exits with code 4 without writing anything

### Requirement: Service install validation and exit codes

The service installer SHALL validate and fail with distinct exit codes: 2
for argument conflicts (missing action, both install and uninstall); 3 when
the unit exists (without `--force`) or is absent on uninstall; 4 for
non-root, an empty or root `--user`, or a non-absolute binary path; 5 for a
missing or non-absolute `--config`; 6 on unsupported platforms
(macOS/Windows). `--force` overwrites an existing unit.

#### Scenario: user must not be root

- **WHEN** installing with `--user root`
- **THEN** the command exits 4

#### Scenario: existing unit

- **WHEN** the unit file exists and `--force` is not passed
- **THEN** the command exits 3 leaving the existing unit untouched

### Requirement: Service start and uninstall

With `--auto-start`, install SHALL run `systemctl daemon-reload` and
`systemctl enable --now sebas.service`; without it, install only writes the
unit and daemon-reloads, printing manual enable instructions. Uninstall
requires the unit to exist, then best-effort stops and disables it, removes
the unit file, and daemon-reloads.

#### Scenario: manual start path

- **WHEN** installing without `--auto-start`
- **THEN** the service is not enabled or started and the output tells the
  user how to do so

#### Scenario: uninstall absent unit

- **WHEN** uninstalling when no unit file exists
- **THEN** the command exits 3

### Requirement: Config discovery

Every subcommand SHALL take an explicit `--config`/`-c` path defaulting to
`./config.toml` relative to the working directory; there is no multi-path
search and no `SEBAS_CONFIG` environment override.

#### Scenario: default path

- **WHEN** `sebas run` is invoked from a directory containing
  `config.toml`
- **THEN** that file is loaded without any flag

### Requirement: Env-only bootstrap for run

When the `run` config file is unreadable or absent, `sebas run` SHALL
fabricate a minimal config from `SEBAS_FEISHU_APP_ID` and
`SEBAS_FEISHU_APP_SECRET` (with a placeholder owner) instead of failing —
enabling container/env-driven deployments with no config file.

#### Scenario: no config file

- **WHEN** `sebas run --config ./missing.toml` runs with both Feishu env
  vars set
- **THEN** the core starts using the env-provided credentials

### Requirement: Config precedence and environment variables

Configuration precedence SHALL be: environment overrides over TOML over
built-in defaults. The env override set comprises `SEBAS_FEISHU_APP_ID`,
`SEBAS_FEISHU_APP_SECRET`, `SEBAS_LOG_LEVEL` (empty values ignored so a
blank variable never blanks a configured credential). Additionally
`SEBAS_CONTROL_SOCKET` and `SEBAS_CONTROL_SECRET` feed the control client,
`SEBAS_IPC` marks watchdog supervision, and `RUST_LOG` drives tracing for
the gateway/webui/watchdog entrypoints (the core filters on `[log] level`).

#### Scenario: env satisfies required field

- **WHEN** the TOML omits `feishu.app_secret` but
  `SEBAS_FEISHU_APP_SECRET` is set
- **THEN** parsing succeeds

#### Scenario: empty env ignored

- **WHEN** `SEBAS_FEISHU_APP_ID=""` is exported and the TOML has an app id
- **THEN** the TOML value is kept

### Requirement: Control client

`sebas control` SHALL expose the sub-subcommands `status`, `events
--since`, `update --dev --dry-run`, `rollback --dry-run`, `restart-core`,
and `services`, with output `--format human|json` (default human). Socket
resolution: `--socket` over `SEBAS_CONTROL_SOCKET` over the default path;
secret: `--secret` over `SEBAS_CONTROL_SECRET` — with neither, the command
fails with an actionable hint (the secret is never persisted). A `Rejected`
response exits with code 2.

#### Scenario: secret missing

- **WHEN** `sebas control status` runs with no secret flag or env var
- **THEN** the command fails with a hint explaining how to supply the
  secret

#### Scenario: rejected exits 2

- **WHEN** the control plane rejects a request (e.g. unauthorized)
- **THEN** the CLI process exits with code 2

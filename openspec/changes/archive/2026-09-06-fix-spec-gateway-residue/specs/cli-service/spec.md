## MODIFIED Requirements

### Requirement: Subcommand tree

`sebas` SHALL expose the subcommands: `core` (long-lived core service),
`run` (the watchdog daemon), `router` (model router), `service`
(install/uninstall systemd unit), `replay` (offline event replay), `record`
(ACP stdio fixture capture), `webui` (dashboard server), `update` (one-shot
updater), `control` (control-plane client) — plus the aliases `status`
(= `control status`), `services` (= `control services`), and `ctl`
(= `control`). The pre-rename compatibility aliases `watchdog` and
`gateway` SHALL NOT be accepted: nothing was released under the old
surface, so old invocations fail as unknown subcommands. Invoking bare
`sebas` with no subcommand is a parse error; the core never runs by default.

#### Scenario: bare invocation rejected

- **WHEN** the user runs `sebas` with no arguments
- **THEN** the CLI prints a usage error and exits nonzero without starting
  any service

#### Scenario: status alias

- **WHEN** the user runs `sebas status --secret ...`
- **THEN** the command behaves as `sebas control status`

#### Scenario: pre-rename subcommand aliases rejected

- **WHEN** the user runs `sebas gateway ...` or `sebas watchdog ...`
- **THEN** the CLI reports an unknown subcommand and exits nonzero without
  starting any service

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
`SEBAS_AGENT_GATEWAY_URL`, and `SEBAS_AGENT_GATEWAY_AUTH` SHALL NOT be
honored — only the new names (`SEBAS_ROUTER_PROVIDER_OVERLAY`,
`SEBAS_AGENT_ROUTER_URL`, `SEBAS_AGENT_ROUTER_AUTH`) take effect.

#### Scenario: env satisfies required field

- **WHEN** the TOML omits `feishu.app_secret` but
  `SEBAS_FEISHU_APP_SECRET` is set
- **THEN** parsing succeeds

#### Scenario: empty env ignored

- **WHEN** `SEBAS_FEISHU_APP_ID=""` is exported and the TOML has an app id
- **THEN** the TOML value is kept

#### Scenario: pre-rename env names are not honored

- **WHEN** only `SEBAS_GATEWAY_PROVIDER_OVERLAY` is set
- **THEN** the router uses the provider overlay from its config (or none)
  and never reads the pre-rename variable

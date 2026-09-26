## MODIFIED Requirements

### Requirement: Config precedence and environment variables

Configuration precedence SHALL be: environment overrides over TOML over
built-in defaults. The env override set comprises `SEBAS_FEISHU_APP_ID`,
`SEBAS_FEISHU_APP_SECRET`, `SEBAS_LOG_LEVEL` (empty values ignored so a
blank variable never blanks a configured credential). Additionally
`SEBAS_CONTROL_SOCKET` and `SEBAS_CONTROL_SECRET` feed the control client,
`SEBAS_IPC` marks watchdog supervision, and `RUST_LOG` drives tracing for
the router/webui/watchdog entrypoints (the core filters on `[log] level`).
The pre-rename names `SEBAS_GATEWAY_PROVIDER_OVERLAY`,
`SEBAS_AGENT_GATEWAY_URL`, and `SEBAS_AGENT_GATEWAY_AUTH` SHALL NOT be
honored — only the new names (`SEBAS_AGENT_ROUTER_URL`,
`SEBAS_AGENT_ROUTER_AUTH`) take effect. The retired state-file and
provider-overlay variables SHALL NOT be honored either: they are no longer
part of the override set, and setting them SHALL have no effect on where
state is read or written.

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

#### Scenario: retired state-file variables are not honored

- **WHEN** the retired state-file or provider-overlay variable is exported
  alongside a configured state database
- **THEN** the configured state database is used
- **AND** the retired variable changes neither the read nor the write path

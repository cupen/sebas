## MODIFIED Requirements

### Requirement: Subcommand tree

`sebas` SHALL expose the subcommands: `core` (long-lived core service),
`run` (the watchdog daemon), `router` (model router), `service`
(install/uninstall systemd unit), `replay` (offline event replay), `record`
(ACP stdio fixture capture), `webui` (dashboard server), `webui-passwd`
(create or update the WebUI login account), `im` (standalone IM service —
the Feishu bot host, spawned by the watchdog when
`[watchdog.im] enabled = true`), `update` (one-shot updater), `control`
(control-plane client), and `agent-kinds list` (reachability report for
configured third-party agents) — plus the aliases `status`
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

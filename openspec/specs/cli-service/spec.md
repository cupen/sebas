# cli-service Specification

## Purpose
Defines the command-line surface of the `sebas` binary: the subcommand
tree, the systemd service installation semantics (unit generation, privilege
drop, exit codes), configuration file discovery, environment-variable
overrides, and the control-plane client.

## Requirements

### Requirement: Subcommand tree
The CLI SHALL provide subcommands: `service`, `service --install/--uninstall`, `run` (watchdog), `core`, `webui`, `router`, `im` (standalone IM service), `auth` (组命令：`auth add`/`auth passwd`/`auth list`，见 `auth-cli` 能力), `agent-kinds`, `ctl` (alias `control`; with `status` and `services` available as top-level shorthand commands for `control status` / `control services`), `update`, `record`, `replay`, `node-link` (execution-node link), `agent-bench`, `feishu` (一次性会话外直连飞书发送文本/图片的调试命令), `skills` (skill 仓管理: list/add/remove/sync), `fake-provider` (本地 Anthropic 线协议假上游，测试与演示用). **补充退出码语义**：每个子命令的进程退出码 SHALL 区分两类失败——启动失败（启动阶段未能达到 ready 或同等成功信号）与运行时崩溃（启动后正常服务中的崩溃）。启动失败 SHALL 以 EX_TEMPFAIL (75) 退出——systemd `Restart=on-failure` 看到 75 会走指数退避（默认 5 s 起跳），避免无限快速重启掩盖错误。运行时崩溃 SHALL 沿用既有退出码语义（典型为 1）。CLI 启动失败 SHALL 同时把失败摘要写入 stderr 的最后一行（"startup-failure: <可读原因>"）以便 CI / 操作员一眼定位。

#### Scenario: core startup failure exits 75

- **WHEN** `sebas core --config bad.toml` 因配置错误在 ready 之前 fatal
- **THEN** 进程以 EX_TEMPFAIL (75) 退出、stderr 末行为 `startup-failure: <原因>`、触发者（systemd / 操作员）即时可见失败

#### Scenario: webui startup failure exits 75

- **WHEN** `sebas webui --config <path>` 因 port bind 失败在 ready 之前 fatal（watchdog secret 缺失不是 fatal：warn 后 admin 控制面只读降级）
- **THEN** 进程以 EX_TEMPFAIL (75) 退出、stderr 末行为 `startup-failure: <原因>`

#### Scenario: runtime crash uses existing exit codes

- **WHEN** core 已经 ready 后因 panic 退出
- **THEN** 进程退出码沿用既有语义（典型为 1），与 startup failure 区分

#### Scenario: startup failure summary line

- **WHEN** 任何 sebas 子命令进入 `failed-startup` 终态
- **THEN** stderr 最后一行 SHALL 为 `startup-failure: <可读原因>`；`SEBAS_STARTUP_ERROR_FILE`（若设置） SHALL 同样包含该摘要

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

#### Scenario: fake-provider 启动假上游

- **WHEN** the user runs `sebas fake-provider --listen 127.0.0.1:0`
- **THEN** the CLI 在 127.0.0.1 上以系统分配端口启动 fake 上游并输出实际绑定地址，Anthropic `/v1/messages` 可被拨号应答

#### Scenario: retired webui-passwd rejected

- **WHEN** the user runs `sebas webui-passwd ...`
- **THEN** the CLI reports an unknown subcommand and exits nonzero；账户管理改用 `sebas auth`（`add`/`passwd`/`list`）

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

### Requirement: Service install validation and exit codes

The service installer SHALL validate and fail with distinct exit codes: 2
for argument conflicts (missing action, both install and uninstall); 3 when
the unit exists (without `--force`) or is absent on uninstall; 4 for
non-root, an empty or root `--user`, a nonexistent `--user` account, or a
non-absolute binary path; 5 for a missing or non-absolute `--config`; 6 on
unsupported platforms (macOS/Windows). `--force` overwrites an existing
unit. A `--log-level` flag SHALL be accepted (install only, ignored by
uninstall) and validated to be a non-empty string.

#### Scenario: user must not be root

- **WHEN** installing with `--user root`
- **THEN** the command exits 4

#### Scenario: existing unit

- **WHEN** the unit file exists and `--force` is not passed
- **THEN** the command exits 3 leaving the existing unit untouched

#### Scenario: nonexistent user

- **WHEN** installing with `--user nosuchuser`
- **THEN** the command exits 4 before writing anything

### Requirement: Service start and uninstall

With `--auto-start`, install SHALL run `systemctl daemon-reload` and
`systemctl enable --now sebas.service`; without it, install only writes the
unit and daemon-reloads, printing manual enable instructions. In both cases,
after writing the unit and daemon-reload, if the unit is currently active
(started), install SHALL explicitly `systemctl restart sebas.service` so a
repeated `install` is a deterministic "reload config and take effect"
operation. Uninstall requires the unit to exist, then best-effort stops and
disables it, removes the unit file, and daemon-reloads.

#### Scenario: manual start path

- **WHEN** installing without `--auto-start`
- **THEN** the service is not enabled or started and the output tells the
  user how to do so

#### Scenario: uninstall absent unit

- **WHEN** uninstalling when no unit file exists
- **THEN** the command exits 3

#### Scenario: reinstall restarts a running service

- **WHEN** installing over an existing, currently-active unit
- **THEN** the unit is rewritten, daemon-reloaded, and restarted so the new
  config takes effect

### Requirement: Config discovery

Every subcommand SHALL take an explicit `--config`/`-c` path defaulting to
`<SEBAS_HOME>/config.toml` (the configuration file inside the sebas home);
there is no multi-path search and no `SEBAS_CONFIG` environment override. An
explicit `--config`/`-c` flag SHALL always win over the default, and a path
that is unreadable or absent keeps the existing env-only bootstrap behavior.
The router's reload source (`SEBAS_ROUTER_CONFIG`) SHALL keep precedence over
that same default when it is unset.

#### Scenario: default path

- **WHEN** `sebas core` is invoked with no `-c` while `SEBAS_HOME` is unset
- **THEN** the configuration file resolves to `<home>/.sebas/config.toml`
  and is loaded when present

#### Scenario: explicit flag wins

- **WHEN** `-c ./config.toml` is passed while `SEBAS_HOME` points elsewhere
- **THEN** the working-directory file is loaded
- **AND** no configuration is read from the sebas home

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

### Requirement: Router runs standalone-only

The router SHALL run only as a standalone process: `sebas router --config
<path> [--debug]`. The `core` entrypoint SHALL NOT embed a router
server — the `--router` and `--debug` flags on `core` SHALL be removed
(BREAKING): passing them SHALL fail with an unknown-argument error rather
than starting any in-process router. `sebas run` retains a `--debug` flag as
a debugging bypass: it starts the standalone router in debug mode alongside
the managed services (the router child is force-enabled when `--debug` is
given even though its service switch defaults to off). Deployments that want the router SHALL
run `sebas router` as its own process — manually, as a compose sidecar, or
via the watchdog's managed router child (which spawns the same standalone
entrypoint). The debug `test` provider remains available via
`sebas router --debug`.

#### Scenario: embedded router flag is rejected

- **WHEN** `sebas core --config <path> --router` is invoked
- **THEN** it fails with an unknown-argument error and no router starts
  inside the core process

#### Scenario: standalone router with debug provider

- **WHEN** `sebas router --config <path> --debug` is invoked
- **THEN** the standalone router starts and the debug `test` model answers
  without dialing any upstream

#### Scenario: manual and managed forms share one entrypoint

- **WHEN** the watchdog spawns its managed router child and, separately, an
  operator runs `sebas router` manually
- **THEN** both are the same standalone `sebas router` entrypoint with the
  same HTTP surface

### Requirement: Sebas home derives every file location

The system SHALL resolve every sebas-owned file location — the layered state
databases, the WebUI user store, the archive registry, the project registry,
the node-link registry, the watchdog's service-override file, the
configuration file default, the channel secret file default, the channel
socket, the control socket, the media download cache, the sebas-node state
directory, and the watchdog's upgrade data directory — from a single sebas
home directory. `SEBAS_HOME` SHALL be the one environment variable that sets
it, and it SHALL otherwise default to `~/.sebas`. Derived paths SHALL be
fixed names inside it: databases and registry files flat in the home root,
sockets under `run/`, the media cache under `cache/downloads/`, node state
under `node/`, and upgrade data under `upgrade/`. The legacy alias
`SEBAS_STATE_DIR` SHALL still be honored when `SEBAS_HOME` is unset, with a
startup warning announcing the rename; when both are set `SEBAS_HOME` SHALL
win. A per-file environment variable or configuration key SHALL remain
honored as an explicit override of that one location, taking precedence over
the derived path; the channel secret file keeps its rule of defaulting
alongside the configuration file, which itself defaults inside the home. No
sebas-owned location SHALL default outside the sebas home. The retired
single-database variable SHALL NOT be honored, so that a leftover value
cannot silently point state at a file the system no longer uses.

#### Scenario: one variable relocates every state file

- **WHEN** only `SEBAS_HOME` is set
- **THEN** every sebas-owned file and socket — databases, registries,
  configuration default, sockets, caches, node and upgrade state — resolves
  inside that directory
- **AND** no sebas-owned file is created or read outside it

#### Scenario: a per-file override wins over the derived path

- **WHEN** `SEBAS_HOME` and one per-file variable are both set
- **THEN** that one file resolves to the per-file value
- **AND** every other file still resolves inside the sebas home

#### Scenario: defaults all live under the default state directory

- **WHEN** neither `SEBAS_HOME` nor any per-file variable is set
- **THEN** every sebas-owned path resolves inside the default sebas home
  `~/.sebas`
- **AND** no default points somewhere else, so there is exactly one rule
  rather than a table of exceptions

#### Scenario: no state write escapes the derived directory

- **WHEN** the sebas home is set and the process runs its normal lifecycle
  including first start, mutations, and shutdown
- **THEN** every sebas-owned file that is created or modified lies inside it
- **AND** a file outside it is a defect, reported by the mechanical check

#### Scenario: a file with no previous override becomes pinnable

- **WHEN** the sebas home is set and the service-override file, the
  node-link registry, a socket, the media cache, the node state directory,
  or the upgrade data directory is written
- **THEN** it is written inside the sebas home
- **AND** it can be relocated without editing any configuration file

#### Scenario: the retired database variable has no effect

- **WHEN** the retired single-database variable is exported while the sebas
  home is configured
- **THEN** no database is opened at the retired variable's path
- **AND** the layered databases resolve from the sebas home as normal

#### Scenario: the legacy alias still works but warns

- **WHEN** only `SEBAS_STATE_DIR` is set and the system starts
- **THEN** every file location resolves inside that directory exactly as if
  it were `SEBAS_HOME`
- **AND** the startup log carries a warning naming `SEBAS_HOME` as the
  canonical variable

#### Scenario: both variables set — home wins

- **WHEN** `SEBAS_HOME` and `SEBAS_STATE_DIR` are both set to different
  directories
- **THEN** every file location resolves inside `SEBAS_HOME`
- **AND** the startup log warns about the conflict

#### Scenario: a socket leaves XDG_RUNTIME_DIR

- **WHEN** `XDG_RUNTIME_DIR` is set or unset and the channel socket is
  created with no explicit override
- **THEN** the socket is created at `<sebas home>/run/core.sock`
- **AND** no path under `XDG_RUNTIME_DIR` or a per-uid temporary directory
  is consulted

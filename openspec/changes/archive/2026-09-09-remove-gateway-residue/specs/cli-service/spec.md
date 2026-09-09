## MODIFIED Requirements

### Requirement: Subcommand tree
The CLI SHALL provide subcommands: `service`, `service --install/--uninstall`, `run` (watchdog), `core`, `webui`, `router`, `webui-passwd`, `agent-kinds`, `ctl`, `update`, `record`, `replay`. **补充退出码语义**：每个子命令的进程退出码 SHALL 区分两类失败——启动失败（启动阶段未能达到 ready 或同等成功信号）与运行时崩溃（启动后正常服务中的崩溃）。启动失败 SHALL 以 EX_TEMPFAIL (75) 退出——systemd `Restart=on-failure` 看到 75 会走指数退避（默认 5 s 起跳），避免无限快速重启掩盖错误。运行时崩溃 SHALL 沿用既有退出码语义（典型为 1）。CLI 启动失败 SHALL 同时把失败摘要写入 stderr 的最后一行（"startup-failure: <可读原因>"）以便 CI / 操作员一眼定位。

#### Scenario: core startup failure exits 75

- **WHEN** `sebas core --config bad.toml` 因配置错误在 ready 之前 fatal
- **THEN** 进程以 EX_TEMPFAIL (75) 退出、stderr 末行为 `startup-failure: <原因>`、触发者（systemd / 操作员）即时可见失败

#### Scenario: webui startup failure exits 75

- **WHEN** `sebas webui --config <path>` 因 port bind 失败或 watchdog secret 缺失在 ready 之前 fatal
- **THEN** 进程以 EX_TEMPFAIL (75) 退出、stderr 末行为 `startup-failure: <原因>`

#### Scenario: runtime crash uses existing exit codes

- **WHEN** core 已经 ready 后因 panic 退出
- **THEN** 进程退出码沿用既有语义（典型为 1），与 startup failure 区分

#### Scenario: startup failure summary line

- **WHEN** 任何 sebas 子命令进入 `failed-startup` 终态
- **THEN** stderr 最后一行 SHALL 为 `startup-failure: <可读原因>`；`--log-file`/`SEBAS_STARTUP_ERROR_FILE`（若指定） SHALL 同样包含该摘要

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

#### Scenario: unknown subcommand rejected

- **WHEN** the user runs `sebas <command>` with a command that is not in the
  subcommand tree and not a documented shorthand alias (`ctl`/`status`/`services`)
- **THEN** the CLI reports an unknown subcommand and exits nonzero without
  starting any service

## REMOVED Requirements

### Requirement: pre-rename subcommand aliases rejected

**Reason**: rename-cli-surface 已从 CLI 树移除 `sebas gateway` 与 `sebas
watchdog`（`src/cli.rs` 的 `Cmd` 枚举无此变体），clap 对两者天然报未知子
命令——护栏守护的对象已不存在，本需求只是为框架默认行为背书。且把旧命令
词继续写进现行 spec 与"gateway 已回归自由词"的目标相悖。守护"未知子命令
报错退出"这一真实行为的职责由新增的泛化场景 `unknown subcommand rejected`
承接。

**Migration**: 无需迁移——被拒命令从未发布；验收其行为等价于新增的
`unknown subcommand rejected` 场景（clap 对任意未注册命令词一致处理）。

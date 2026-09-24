## MODIFIED Requirements

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

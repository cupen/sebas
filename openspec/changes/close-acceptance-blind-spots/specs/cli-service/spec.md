## ADDED Requirements

### Requirement: Inherited provider env posture is reported at startup

当 core 或 webui 进程启动时，若 OS 环境携带会引导 Claude 子进程的 `ANTHROPIC_*`
变量（`ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_MODEL`、
`ANTHROPIC_DEFAULT_OPUS_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL`、
`ANTHROPIC_DEFAULT_HAIKU_MODEL`、`ANTHROPIC_SMALL_FAST_MODEL`），而当前 provider
解析模式不会以 cover 语义强制覆盖其中任一变量，系统 SHALL 在启动日志 emit 一条
WARN，逐变量点名「继承自 shell、将在 Claude 子进程生效」。当 cover 语义覆盖全部
被检变量时 MUST NOT 告警。该检测 SHALL 只报告不篡改——MUST NOT 改变 env 的实际
传递行为。

#### Scenario: 未覆盖的继承 env 触发告警

- **WHEN** shell 导出 `ANTHROPIC_BASE_URL` 与 `ANTHROPIC_MODEL` 后，以不覆盖
  env 的 provider 模式启动 core
- **THEN** 启动日志出现一条 WARN，点名这两个变量继承自 shell 并将作用于
  Claude 子进程

#### Scenario: 全覆盖时保持安静

- **WHEN** provider 解析为 covering 模式，五个 cover 变量全部被强制覆盖
- **THEN** 启动日志不出现 env posture 告警

## Purpose

pi（earendil-works/pi coding agent）作为 sebas 第三个一等 agent 的驾驶契约：以 pi 自有的 headless RPC 协议（`pi --mode rpc`，stdio 严格 JSONL）驱动完整会话生命周期，覆盖回合流式、会话持久化与恢复、模型选择、无权限系统的如实呈现与凭据自管口径。

## ADDED Requirements

### Requirement: pi is drivable as a first-class agent

以 `driver = "pi"` 声明的 agent SHALL 由 PiDriver 驱动：spawn `pi --mode rpc` 子进程，经 stdin/stdout 的严格 JSONL（JSON-RPC 2.0 超集，LF 分帧）通信，对外只产出与其余 driver 相同的 `AcpEvent`/`AcpCommand` 词表，pi 协议类型不得泄漏出 driver 模块边界。启动握手 SHALL 在该 agent 配置的 startup timeout 内完成（超时失败口径与其它 driver 一致）。取消 SHALL 映射为 RPC `abort` 并以 `agent_settled(aborted=true)` 作为回合收尾；pi 进程意外退出 SHALL 上报带 terminal 标记的 `Error`。

#### Scenario: pi session streams the shared vocabulary

- **WHEN** 一个 pi 会话收到 prompt，pi 依次流出 `message_update`（text/thinking 增量）、`tool_execution_start/update/end`、`agent_settled`
- **THEN** 下游只观察到 `TextDelta`/`ThinkingDelta`/`ToolStart`/`ToolProgress`/`ToolEnd`/`Finished` 等 `AcpEvent` 变体
- **AND** 任何 pi 私有字段名都不出现在 driver 模块边界之外

#### Scenario: startup timeout is enforced

- **WHEN** pi 子进程在配置的 startup timeout 内未完成握手（如二进制存在但挂起）
- **THEN** 会话创建以超时失败，错误口径与其它 driver 的握手超时一致

#### Scenario: cancel settles the turn

- **WHEN** 会话收到取消指令
- **THEN** PiDriver 发送 RPC `abort`，并以 `agent_settled(aborted=true)` 之后才发 `Finished`
- **AND** pi 进程随后被按 idle-kill 生命周期管理，不因取消而泄漏

#### Scenario: unexpected pi exit is terminal

- **WHEN** pi 子进程在会话活跃期意外退出（非正常 shutdown）
- **THEN** 会话收到带 terminal 标记的 `Error`，路由移除映射并如实呈现

### Requirement: pi sessions persist and resume

pi 会话 SHALL 落在该 agent 配置的 sessions 目录（经 `--session-dir` 钉住）；握手完成时 SHALL 把 pi 会话 id 作为 agent 侧会话 id 上报（对齐通用 ACP driver 的会话 id 语义）。恢复 SHALL 以 `--session <id>` 重新挂接同一 pi 会话；恢复被拒（会话文件缺失或损坏）SHALL 诚实回落到新会话并以 resumed=false 呈现，不得伪造续接。

#### Scenario: round-trip resume

- **WHEN** 一个完成过回合的 pi 会话被恢复（sebas 重启或会话重挂）
- **THEN** PiDriver 以记录的 pi 会话 id spawn `--session <id>`，续接历史并续用同一 agent 侧会话 id

#### Scenario: rejected resume falls back to a fresh session

- **WHEN** 记录的 pi 会话文件已被删除，恢复请求到达
- **THEN** PiDriver 如实回落为新会话（resumed=false）并记录告警，不向上游伪装续接成功

### Requirement: pi reports turn completion and usage

回合边界 SHALL 以 `agent_settled` 为准：`agent_end` 之后仍可能有自动重试、compaction 恢复或排队消息跟进，`Finished` 只在 settled 后发出。用量 SHALL 从事件流携带的累计 usage 上报 `UsageUpdate`（pi 报多少是多少，缺项如实为零/缺省）。

#### Scenario: retries finish before the turn settles

- **WHEN** pi 在一次 prompt 后经历 `auto_retry`（`agent_end` 后跟重试再跑一个低层 run）
- **THEN** 会话在该低层 run 结束时不发 `Finished`，直到 `agent_settled` 才发

#### Scenario: usage reaches the accounting surface

- **WHEN** pi 的 `message_update` 携带累计 usage（input/output/cache 计数与费用）
- **THEN** 下游经 `UsageUpdate` 收到同值数据，供用量统计与呈现消费

### Requirement: pi model selection round-trips

切换模型 SHALL 映射为 RPC `set_model`，成功后 SHALL 发出 `ModelChanged`；可用模型面 SHALL 来自 `get_available_models` 的应答（不内置硬编码模型表）；`set_model` 失败 SHALL 如实上报为非终态错误，会话保持可用。

#### Scenario: switch model mid-session

- **WHEN** 操作员在活跃 pi 会话上切换模型
- **THEN** PiDriver 发送 `set_model`，成功后下游收到 `ModelChanged`，后续回合使用新模型

#### Scenario: invalid model is rejected honestly

- **WHEN** `set_model` 指向 pi 不认识的模型
- **THEN** 失败响应被上报为非终态错误，会话继续以原模型可用

### Requirement: pi has no permission system — honest semantics

pi 无内置权限系统，v1 的 pi 会话 SHALL 不产生 `PermissionRequest`（工具以子进程权限直跑）。`SetMode` SHALL 得到非终态的「不支持」应答，会话保持可用；审批卡与权限模式 UI 对 pi 会话如实呈现不可用，不得假装已生效。看门狗的周期性模式探针在 pi 会话上 SHALL 被容忍（应答不支持、不升级为故障）。

#### Scenario: set mode answered unsupported, session lives on

- **WHEN** 对 pi 会话下发 `SetMode`（含看门狗周期探针）
- **THEN** 会话收到非终态「不支持」类错误，会话与回合流不受影响

#### Scenario: no permission cards for pi

- **WHEN** pi 会话的工具调用直接执行
- **THEN** 下游不出现该会话的 `PermissionRequest`，UI 不为 pi 渲染审批卡

### Requirement: credentials are pi-managed

sebas SHALL NOT 翻译或代写 pi 的凭据与 provider 资产（`models.json`、auth 存储）：pi 子进程继承 sebas 进程环境（标准 API key env 表由 pi 自行读取），provider 配置由操作员在 pi 侧完成。pi 的 reachability SHALL 以二进制在场探测，且在二进制在场时辅以 `pi auth check`（ready/not_ready/invalid）如实区分「装了没登录」。

#### Scenario: inherited env reaches pi

- **WHEN** 操作员在启动 sebas 前导出了 pi 认识的 API key 环境变量并创建 pi 会话
- **THEN** pi 子进程凭继承的 env 完成上游认证，sebas 自身不读写任何 pi 凭据文件

#### Scenario: reachability distinguishes installed-but-unauthenticated

- **WHEN** pi 二进制在场但 `pi auth check` 返回 not_ready
- **THEN** agent 目录将 pi 报为可达（二进制在场）但带认证未就绪的说明，而不是笼统的不可达

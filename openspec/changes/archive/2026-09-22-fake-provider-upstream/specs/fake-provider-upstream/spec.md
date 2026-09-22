# fake-provider-upstream 增量

## Purpose

为验收与本地演示提供零 token 的 Anthropic 线协议假上游：以确定性、可编排的应答替代真实模型 API，让 agent 相关用例在进程级测试中完整跑通且可离线断言。

## ADDED Requirements

### Requirement: fake-provider 服务生命周期

`sebas fake-provider` SHALL 在 127.0.0.1 上启动 HTTP 服务并持续应答，实际监听地址（含 0 端口随机分配后的实际值）SHALL 通过 stdout/日志可见；收到终止信号后 SHALL 优雅退出。服务 SHALL 暴露 Anthropic `/v1/messages` 协议面。

#### Scenario: 随机端口启动可见

- **WHEN** `sebas fake-provider --listen 127.0.0.1:0` 启动
- **THEN** 进程进入就绪且 stdout/日志输出实际绑定的 127.0.0.1 地址与端口

#### Scenario: 终止信号优雅退出

- **WHEN** 向运行中的 fake-provider 发送 SIGTERM
- **THEN** 进程优雅退出且端口释放，不留下监听残留

### Requirement: Anthropic /v1/messages 应答

fake 上游 SHALL 对 `/v1/messages` 返回符合 Anthropic 线协议的应答：非流式为 200 JSON message（含 content 块、stop_reason、usage）；`stream=true` 为 `text/event-stream` 的完整事件序列（message_start → content_block delta → message_delta → message_stop）。auth header SHALL 不校验具体值。应答内容 SHALL 由行为规则决定（内置规则或 scenario）。

#### Scenario: 非流式文本应答

- **WHEN** 以无 `tools` 的普通文本请求（stream=false）POST `/v1/messages`
- **THEN** 返回 200 JSON：assistant 文本应答、stop_reason=end_turn、usage 为确定性非零值

#### Scenario: 流式事件序列完整

- **WHEN** 同一请求以 stream=true 发送
- **THEN** 返回 200 text/event-stream，事件序列完整且文本内容与非流式一致

### Requirement: 内置 agent-loop 确定性规则

未配置 scenario 时，fake 上游 SHALL 按确定性规则应答以自动驱动标准工具环：请求含非空 `tools` 且消息历史中尚无 `tool_result` → 返回首个 tool 的 `tool_use` 块（stop_reason=tool_use，input 为确定性对象）；消息历史中已含 `tool_result` → 返回终文本应答（stop_reason=end_turn）；无 `tools` 的请求返回纯文本应答。相同请求 SHALL 产生相同应答。

#### Scenario: 首轮返回 tool_use

- **WHEN** 请求带非空 tools 且消息历史无 tool_result
- **THEN** 应答 content 含指向首个 tool 的 tool_use 块且 stop_reason=tool_use

#### Scenario: tool_result 后终局

- **WHEN** 请求消息历史已含 tool_result
- **THEN** 返回终文本应答且 stop_reason=end_turn，agent 工具环自然收敛

### Requirement: 确定性 usage 计量

fake 上游 SHALL 为每个成功应答返回确定性非零 usage（input/output tokens）：默认值固定，scenario 可覆盖；同一场景下重复相同请求的 usage SHALL 完全一致，使 quota/metrics 断言无需真实模型。

#### Scenario: usage 确定可断言

- **WHEN** 同一场景下两次相同请求
- **THEN** 两次应答的 usage 数值完全一致且非零

### Requirement: scenario 文件编排

`--scenario <file>` SHALL 加载 JSON 剧本并按序消费预置条目：文本/工具块应答，或错误注入（指定状态码如 429/500 与 retry-after 头）；剧本耗尽后 SHALL 回落内置规则；未提供 scenario 时 SHALL 直接使用内置规则。剧本加载失败（文件不存在或非法 JSON）SHALL 视为启动失败并按 CLI 启动失败语义退出。

#### Scenario: 剧本按序消费

- **WHEN** scenario 预置三条应答且依次发起三个请求
- **THEN** 三个应答严格按预置顺序返回

#### Scenario: 错误注入透传

- **WHEN** scenario 条目为 429 加 retry-after
- **THEN** 该请求收到 429 与 retry-after 头，后续请求不受影响

#### Scenario: 剧本耗尽回落内置规则

- **WHEN** 剧本条目已全部消费后继续请求
- **THEN** 后续请求由内置规则应答，服务不失败

### Requirement: 请求留痕

fake 上游 SHALL 将收到的每个请求（方法、路径、header、body）以 NDJSON 逐行追加到 `--journal <file>` 指定的文件，供套件离线断言透传行为（上游 key 注入、下游 key 不泄漏、model rename）。

#### Scenario: 透传断言可离线完成

- **WHEN** router 以自定义 provider 接入 fake 并转发一条请求
- **THEN** journal 新增该请求记录，其 auth header 为 provider 上游 key 且不含下游 key

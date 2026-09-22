# testsuite-process-e2e 增量

## ADDED Requirements

### Requirement: provider 透传 journey

套件 SHALL spawn fake 上游并在沙箱 config 声明自定义 provider（`base_url_anthropic` 指向 fake、上游 key 为哑值），经路由 `/v1/messages` 验证进程级透传链路：上游 key 注入且下游 key 不泄漏（fake journal 离线断言）、SSE 逐事件透传、usage 结算落 router-usage.jsonl。

#### Scenario: 非流式透传与 usage 落账

- **WHEN** 向 router 发起 model 路由到 fake provider 的非流式请求
- **THEN** 应答为 fake 的确定性内容，router-usage.jsonl 新增记录含非零 input/output tokens 与该 provider 名

#### Scenario: 流式透传完整

- **WHEN** 发起 stream=true 的同类请求
- **THEN** 客户端收到完整 SSE 事件序列，usage 结算同样落账

#### Scenario: 下游 key 不泄漏到上游

- **WHEN** 检查 fake 的请求 journal
- **THEN** 转发请求的 auth header 为 provider 上游 key，且不含下游 key 与 hop-by-hop 头

### Requirement: agent-loop journey（零 token）

在 claude-code 二进制可用的环境，套件 SHALL 以真实 claude-code 为 ACP 执行体、会话模型路由到 fake 上游，验证完整 agent 工具环：消息 → tool_use → 工具执行 → tool_result → 终文本，会话到达 Done；二进制缺席时该用例 SHALL 跳过而非失败。

#### Scenario: 工具环到 Done

- **WHEN** 创建模型路由到 fake 的 ACP 会话并提交一条触发工具的任务
- **THEN** 会话状态到达 Done，回合内容含终文本与工具执行痕迹，全程无真实上游外呼

#### Scenario: claude-code 缺席诚实跳过

- **WHEN** 测试环境不存在 claude-code 二进制
- **THEN** 该用例跳过并输出原因，套件整体不判失败

### Requirement: 确定性限流/用量用例

套件 SHALL 以 fake 上游验证 per-key 令牌桶限流与用量记录的确定性：fake 秒回应答消除真实上游网络延迟抖动，使限流断言可精确复现，且全程不向真实上游发起网络请求；usage 计量断言使用 fake 的确定性非零 usage。

#### Scenario: 令牌桶耗尽 429 可复现

- **WHEN** 以超过桶容量的速率向 router 连发请求（fake 秒回）
- **THEN** 越界请求收到 429 rate_limit_error，且该用例重复运行结果一致、无网络外呼

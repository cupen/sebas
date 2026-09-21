## ADDED Requirements

### Requirement: Fake agent stub simulates full model behavior

fake-claude 桩 SHALL 支持场景参数化地模拟真实模型回合的全部行为面，供全部测试层
零真模型调用复用：

- 正文增量流式输出（多段文本 delta）
- thinking 增量，及 thinking→正文交替
- 工具环：tool_use → 测试侧 tool_result → 后续正文
- 空响应回合（正常结束、零输出）
- 慢响应（延迟可配，用于停滞与反馈时限用例）
- 错误响应（上游错误形态）

场景经启动参数（键值形式）选择，未指定时保持既有缺省行为；桩 SHALL 在 journal
记录所用场景供断言。进程级 e2e 与浏览器级旅程的新增用例 SHALL 优先用这些场景
覆盖对应能力，MUST NOT 为扩大覆盖面新增真模型依赖。

#### Scenario: 工具环场景

- **WHEN** 以工具环场景启动 fake-claude 会话并发送一条消息
- **THEN** 会话投影依次出现 tool_use、工具执行与后续正文，回合到达 Done

#### Scenario: 空响应场景

- **WHEN** 以空响应场景启动 fake-claude 会话并发送一条消息
- **THEN** 回合正常结束，会话投影含零输出合成提示条目

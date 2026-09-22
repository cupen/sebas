## ADDED Requirements

### Requirement: 旅程证明标准——绿即工作台可用且稳定

test 模型 journey 的价值在于其**证明力**，套件 SHALL 按以下标准设计与断言，使 journey 全绿即构成「agent 工作台正常、稳定、可用」的证据：

- **用户面保真**：journey SHALL 经工作台的用户面驱动——webui HTTP API 提交操作、WS/SSE 接收事件——不得为图省事绕过用户面直调内部接口；断言的对象 SHALL 是操作员可见的行为（转录呈现、会话状态、通知），不是内部数据结构。
- **路径完整**：journey 集 SHALL 合计覆盖操作员完整路径——项目注册/选择 → 创建会话 → 提交消息 → 流式呈现 → 权限决策 → 结果确认 → 关闭/归档；任一环节无 journey 命中即为缺口。
- **可重复即稳定**：场景的确定性 SHALL 被 journey 消费——同一 journey 重复执行 SHALL 产生相同的转录形状与会话终态，套件 SHALL 至少对一条代表性 journey 断言重复一致性；单次绿不构成稳定性证明。
- **失败也诚实**：错误路径 journey SHALL 同时断言「失败如实呈现」与「失败后工作台仍可操作」（其余会话可继续创建与使用）。

#### Scenario: 完整操作员路径在用户面走通

- **WHEN** 以 webui HTTP API 注册项目、创建 native 会话（模型 `test/tool-use`）、经 WS 提交消息并完成一次权限批准
- **THEN** 流式事件、权限请求、终文本均出现在用户面
- **AND** 会话经用户面关闭，全程零真实上游外呼

#### Scenario: 重复执行结果一致

- **WHEN** 代表性 journey（工具环）连续执行两次（独立会话）
- **THEN** 两次的转录块形状、权限交互次数与会话终态一致
- **AND** 差异仅允许出现在时间戳与 id 类字段

#### Scenario: 会话取消在流式中生效

- **WHEN** 以 `test/long` 驱动长流会话，流式期间经用户面发起取消
- **THEN** 流式停止、取消被如实呈现（非伪装完成）
- **AND** 会话此后仍可发起新回合或被关闭

#### Scenario: 模型切换在下一回合生效

- **WHEN** 会话先以 `test/text` 完成一回合，再经用户面把模型切换为 `test/tool-use` 并提交含工具的任务
- **THEN** 下一回合的 LLM 行为符合新场景（产生 tool_use 与权限请求）
- **AND** 切换前后的回合各自保留其呈现形状

#### Scenario: 失败后工作台仍可操作

- **WHEN** 一个会话因 `test/error` 回合失败后
- **THEN** 操作员仍可创建新会话并完成一次正常回合（如 `test/text`）
- **AND** 工作台不因单个会话的失败进入不可用状态

### Requirement: test 模型场景 journey

套件 SHALL 以 debug `test` provider 的场景模型驱动进程级验收 journey，覆盖核心五簇中**LLM 形状相关**的工作台能力，全程零真实凭据、零上游外呼、零额外进程（router 内自答，区别于 fake 上游的拨号 journey）。场景与被覆盖能力的对应关系 SHALL 按 `extend-test-model-scenarios` 的覆盖矩阵执行：

- **native kernel 工具环 journey**（`test/tool-use`）：tool_use → 权限请求 → 批准 → 工具执行 → tool_result → 终文本 → Done；
- **并行权限 journey**（`test/tools-parallel`）：一回合多个 tool_use 各自产生独立权限请求，逐一批准/拒绝后回合继续；
- **thinking 呈现 journey**（`test/thinking`）：thinking 进转录且与正文呈现形态可区分；
- **混排 journey**（`test/full`）：thinking / 正文 / tool_use 同回合按序呈现；
- **零输出通知 journey**（`test/empty`）：回合完成但无可见输出时追加通知；
- **长文流式 journey**（`test/long`）：流式背压路径与增量呈现，拼接结果与非流式一致；
- **错误呈现 journey**（`test/error`）：LLM 失败如实呈现（错误进回合/会话状态），不伪装成功；
- 可选：真实 claude-code 场景 journey（缺席诚实跳过，与 agent-loop journey 同法）。

对话连续性 SHALL 由 echo 句式显式证明：`test/text` 的应答回显最后一条用户消息，journey SHALL 断言该回显内容与会话历史一致，使「完整对话历史送达 LLM」成为被断言的事实而非隐含假设。

各 journey 的断言 SHALL 使用场景的确定性形状（块顺序、stop_reason、固定非零 usage），且既有的 bare `test` echo 断言 SHALL 不改动而通过。

#### Scenario: native kernel 工具环经权限流到 Done

- **WHEN** 以 native 后端创建模型为 `test/tool-use` 的会话并提交一条任务
- **THEN** 首回合产生 tool_use 并触发权限请求，批准后工具执行、tool_result 回传
- **AND** 次回合返回终文本，会话到达 Done，usage 记录为场景的确定性非零值，全程无真实上游外呼

#### Scenario: 并行工具调用各自获得独立权限请求

- **WHEN** 以 `test/tools-parallel` 驱动一个声明了多个工具的会话
- **THEN** 同一回合的每个 tool_use 各自产生独立的权限请求（互不串扰）
- **AND** 逐一决策后回合继续推进

#### Scenario: thinking 进入转录且与正文可区分

- **WHEN** 创建模型为 `test/thinking` 的会话并提交一条任务
- **THEN** 回合转录含 thinking 内容块，其呈现形态与正文块可区分
- **AND** 会话正常到达 Done

#### Scenario: echo 回显证明对话历史完整送达

- **WHEN** 会话经历多轮消息后以 `test/text` 完成一回合
- **THEN** 应答中回显的正是本轮最后一条用户消息的原文
- **AND** 更早轮次的消息不污染回显（回显取的是最后一条）

#### Scenario: 零输出回合追加通知

- **WHEN** 创建模型为 `test/empty` 的会话并提交一条任务
- **THEN** 回合完成且转录无可见输出
- **AND** 通知路径被触发（零输出通知出现在会话面）

#### Scenario: 长文流式保持拼接一致

- **WHEN** 创建模型为 `test/long` 的会话并提交一条任务
- **THEN** 流式呈现期间无帧丢失，最终拼接文本与非流式 body 一致
- **AND** 会话正常到达 Done

#### Scenario: LLM 错误如实呈现

- **WHEN** 创建模型为 `test/error` 的会话并提交一条任务
- **THEN** 失败以错误状态如实呈现（回合/会话面可见错误，不伪装成功）
- **AND** 会话状态迁移遵循既有终局错误语义

#### Scenario: 既有 echo 断言不改而通过

- **WHEN** 套件中既有的 bare `test` debug 应答用例运行
- **THEN** 断言不加改动而通过
- **AND** 场景模型的引入未改变 bare `test` 的响应

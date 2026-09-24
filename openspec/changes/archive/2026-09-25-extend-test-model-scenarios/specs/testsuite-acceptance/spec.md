## ADDED Requirements

### Requirement: 工作台验收载体 = 内置 test 模型

涉及 agent 回合的工作台行为验收（旅程级、进程级与浏览器级）SHALL 以 router 内置 debug `test` 模型的场景模型为标准 LLM 载体：正文、thinking、工具环、权限流、零输出、长文流式、错误呈现等形状一律由 `test/<scenario>` 驱动，而非真实凭据或独立 fake 上游进程。fake 上游与真实上游 journey SHALL 保留，但其职责限定为**拨号透传路径**的验收（header 过滤、key 注入、SSE 透传、限流/用量结算），不承担工作台行为验收。`fake-claude` 桩在浏览器级继续作为其驱动器专属契约（ACP 驱动行为）的权威，其 LLM 形状类用例按账本节奏向 test 模型迁移。载体切换 SHALL NOT 降低覆盖口径：核心五簇账本（`tests/acceptance/COVERAGE.md`）保持 100% 硬指标——证据可以随载体更换重指，requirement 分母与命中率口径不变；账本 SHALL 记录本次载体定向作为变更说明。

#### Scenario: 工作台旅程由 test 模型驱动

- **WHEN** 一条涉及 agent 回合的工作台验收旅程被编写或改写
- **THEN** 其 LLM 响应来自 `test/<scenario>` 场景模型
- **AND** 旅程不依赖真实凭据、真实上游或额外 fake 进程

#### Scenario: 拨号路径 journey 职责不变

- **WHEN** 验收对象是 router→上游的透传链路（header 过滤、SSE 透传、限流）
- **THEN** 该 journey 继续使用 fake 上游（或 fake-provider）
- **AND** 不被本要求改写为 test 模型

#### Scenario: 载体切换不降低覆盖口径

- **WHEN** 一条既有 journey 的 LLM 载体从 fake 上游或真实凭据换为 test 模型
- **THEN** 其命中的 requirement 在账本中的证据被重指到新 journey
- **AND** 核心五簇的 requirement 分母与 100% 命中率口径不变

#### Scenario: 账本记录载体定向

- **WHEN** 载体定向落地
- **THEN** `COVERAGE.md` 的账本规则段记录「工作台行为验收 = test 模型」及其日期
- **AND** 核心集增删说明段落载本次定向

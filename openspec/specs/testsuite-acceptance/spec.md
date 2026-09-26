# testsuite-acceptance Specification

## Purpose

以能力×旅程矩阵为账本的全功能验收套件：对仓库主 specs 的全部能力维护"关键功能是否被验收命中"的全局账（能力数与 requirement 数以 `tests/acceptance/COVERAGE.md` 的复核记录为准，规格正文不固定具体基数）；核心功能集（五簇：agent workbench 相关、项目管理、会话管理、models 管理、通道与监督）命中 ≥90% 为唯一硬指标，其余能力矩阵可见不设门槛，测不了的面以豁免清单明示，并提供一键复跑入口。

## Requirements

### Requirement: 验收矩阵账本

套件 SHALL 维护 `tests/acceptance/COVERAGE.md` 验收矩阵：仓库主 specs 的每个能力一行，行内关键 requirement SHALL 标注命中证据——验收用例 id、既有测试（单元/集成）引用，或豁免标记（必须注明 cause）。未命中且未豁免的条目即构成缺口清单。矩阵 MUST 与能力变更同步更新：新增/修改能力的变更落地时，同一变更 SHALL 更新对应矩阵行。

#### Scenario: 每个能力都有账面行

- **WHEN** 审阅 `tests/acceptance/COVERAGE.md`
- **THEN** 主 specs 下每个能力目录都有一行，行内条目均带命中证据或豁免 cause，不存在空白条目

#### Scenario: 缺口可见

- **WHEN** 存在未命中且未豁免的能力条目
- **THEN** 矩阵的缺口清单段列出这些条目，直到补用例或转豁免为止

### Requirement: 覆盖通过标准

验收覆盖采用两级度量。**核心功能集**（界定见"核心功能集界定"）的命中 SHALL ≥90%，为套件通过的唯一硬指标；其余能力在矩阵中 SHALL 全量可见（命中证据、豁免 cause 或缺口标注），不设数字门槛。覆盖面按 requirement/旅程级命中计：一条 requirement 被任一测试层（验收用例、集成、单元测试）完整命中即计入，以代码行覆盖率为度量被明确排除。核心功能集的每一簇 SHALL 至少有一条套件内旅程用例命中，不得全靠引用既有单元测试凑数。豁免面（飞书传输、浏览器级 UI、真实模型语义）不计入分母，豁免 MUST 注明 cause 与替代验证手段。达标复核 SHALL 显式执行并记录数字。

#### Scenario: 核心集达标复核

- **WHEN** 收尾复核运行
- **THEN** 核心五簇分别给出 requirement 总数、命中数与百分比，全部 ≥90% 且每簇至少一条套件内旅程用例，记录通过

#### Scenario: 非核心能力不挡通过

- **WHEN** 非核心能力存在未命中且未豁免的缺口
- **THEN** 套件仍可通过，该缺口保留在矩阵缺口清单中待补

#### Scenario: 豁免面明示

- **WHEN** 某能力条目被标为豁免
- **THEN** 矩阵注明 cause（如"需真实凭据"）及现有替代验证（如进程内集成测试引用）

### Requirement: 核心功能集界定

套件 SHALL 在矩阵中显式标注核心功能集，共五簇：agent workbench 相关（agent-workbench、permission-flow）、项目管理（project-session-actions 及 state-store/projects、webui projects 面）、会话管理（session-lifecycle、session-persistence、acp-session-mapping）、models 管理（acp-model-selection、router-model-aliases、provider-management）、通道与监督（core-session-channel、watchdog）。核心集的增删 MUST 是显式变更：矩阵标注与变更说明同步更新，不得静默调整。

#### Scenario: 核心行有标注

- **WHEN** 审阅验收矩阵
- **THEN** 五簇对应的条目带"核心"标注，90% 复核仅统计这些条目

#### Scenario: 边界调整留痕

- **WHEN** 某能力条目被加入或移出核心集
- **THEN** 矩阵标注更新且变更说明记录了这次调整

### Requirement: 旅程用例形态

验收用例 SHALL 是多步旅程级场景：以真实二进制（沙箱拓扑）经进程间真实边界（HTTP、核心通道、文件系统）串联多个能力，断言面向操作员可见结果。套件 MUST NOT 在进程级孤立重测单元层已覆盖的单点契约；无旅程命中的能力簇才新增用例。沙箱隔离、平台门控、显式超时、失败保留现场 MUST 遵循 `testsuite-process-e2e` 能力的同一约定。

#### Scenario: 一条旅程跨多个能力

- **WHEN** 一条验收用例执行
- **THEN** 它串联至少两个能力的用户可见行为（如 provider 治理：overlay 修改 → admin API 校验 → 网关路由生效），并在矩阵中标注其命中的全部条目

#### Scenario: 不重测单点契约

- **WHEN** 某 requirement 已有单元/集成测试完整覆盖且无旅程级缺口
- **THEN** 矩阵行引用既有测试作为命中证据，不新增验收用例

### Requirement: 一键入口与诊断

套件 SHALL 提供单条命令入口 `invoke testsuite-acceptance`：构建工作区二进制后运行全部验收用例，退出码如实反映结果；`--case <用例名>` SHALL 透传为过滤器以手动单跑。用例失败时 MUST 保留沙箱目录与日志并打印路径（同 `testsuite-process-e2e` 约定）。

#### Scenario: 一键全量验收

- **WHEN** 操作员执行 `invoke testsuite-acceptance`
- **THEN** 构建完成后全部验收用例运行，任一失败则非零退出，全部通过则零退出

#### Scenario: 手动单跑与现场保留

- **WHEN** 开发者执行 `invoke testsuite-acceptance --case <用例名>`，且该用例失败
- **THEN** 仅该用例运行，其沙箱目录与 core/webui 日志被保留并打印路径

### Requirement: native 链路验收策略

native 内核旅程 SHALL 以 in-process 沙箱形态（`SEBAS_AGENT_ROUTER_URL → debug router` 通路）纳入套件（`wire-webui-sebas-agent-e2e` 已落地，native 旅程用例已入账；detached 形态的 native 验收以既有豁免/缺口标注管理，不为本 requirement 的前置）。矩阵 SHALL 记录该用例 id 与其命中的 native 相关条目；后续新增 native 旅程（如 detached 形态）按「验收矩阵账本」同步更新。

#### Scenario: native 旅程入账

- **WHEN** 审阅验收矩阵
- **THEN** native 相关条目标注为"已纳入"并引用套件内旅程用例 id，不再以 spike 过程叙事作为规约

### Requirement: 套件运行预算

全套验收用例 MUST 以显式超时为界，单用例与全套总时长 SHOULD 有预算上限（单用例 ≤30s、全套 ≤5 分钟量级）；用例 MUST 以 `#[ignore]` 标注不进默认 `cargo test`，平台门控遵循 `testsuite-process-e2e` 约定。

#### Scenario: 默认路径不受扰

- **WHEN** 开发者运行默认 `cargo test`
- **THEN** 验收用例不执行，既有测试全绿不受影响

### Requirement: 远端节点 mode 旅程

验收套件 SHALL 包含远端节点会话的 mode 旅程用例（真 sebas-node 二进制、EchoBody 桩，零真模型调用）：

- **创建带 mode**：在远端节点上以 mode=allow 创建会话，断言投影的 desired_mode 为 allow 且节点回报一致；门控动作（`run:` 输入）不再进入 waiting 而直接执行（与既有旅程的 waiting 断言形成对照）。
- **中途切换**：对运行中的远端会话切换 mode（如 allow→ask），断言 SetMode 经链路生效、后续门控动作重新进入 waiting、投影 mode 更新。
- **离线拒绝**：节点离线时创建带 mode 会话按既有规则如实拒绝（点名节点），不建占位。

#### Scenario: 远端 allow 会话免门控

- **WHEN** 在远端节点以 mode=allow 创建会话并发送 `run: ls -la`
- **THEN** 动作直接执行（不产生 waiting 审批），会话回合完成

#### Scenario: 远端会话中途切回 ask 恢复门控

- **WHEN** 对该会话切换 mode 为 ask 后再发送 `run:` 输入
- **THEN** 动作被门控为 waiting 等待控制面决定，投影 mode 显示 ask

#### Scenario: 零 token 断言

- **WHEN** 该旅程全程运行
- **THEN** 不发生任何真实模型调用（节点 agent 为 EchoBody 桩）

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

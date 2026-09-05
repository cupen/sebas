# acceptance-suite Specification

## Purpose

以能力×旅程矩阵为账本的全功能验收套件：对全部能力（30 个、247 条 requirement）维护"关键功能是否被验收命中"的全局账；核心功能集（agent workbench 相关、项目管理、会话管理、models 管理）命中 ≥80% 为唯一硬指标，其余能力矩阵可见不设门槛，测不了的面以豁免清单明示，并提供一键复跑入口。

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

验收覆盖采用两级度量。**核心功能集**（界定见"核心功能集界定"）的命中 SHALL ≥80%，为套件通过的唯一硬指标；其余能力在矩阵中 SHALL 全量可见（命中证据、豁免 cause 或缺口标注），不设数字门槛。覆盖面按 requirement/旅程级命中计：一条 requirement 被任一测试层（验收用例、集成、单元测试）完整命中即计入，以代码行覆盖率为度量被明确排除。核心功能集的每一簇 SHALL 至少有一条套件内旅程用例命中，不得全靠引用既有单元测试凑数。豁免面（飞书传输、浏览器级 UI、真实模型语义）不计入分母，豁免 MUST 注明 cause 与替代验证手段。达标复核 SHALL 显式执行并记录数字。

#### Scenario: 核心集达标复核

- **WHEN** 收尾复核运行
- **THEN** 核心四簇分别给出 requirement 总数、命中数与百分比，全部 ≥80% 且每簇至少一条套件内旅程用例，记录通过

#### Scenario: 非核心能力不挡通过

- **WHEN** 非核心能力存在未命中且未豁免的缺口
- **THEN** 套件仍可通过，该缺口保留在矩阵缺口清单中待补

#### Scenario: 豁免面明示

- **WHEN** 某能力条目被标为豁免
- **THEN** 矩阵注明 cause（如"需真实凭据"）及现有替代验证（如进程内集成测试引用）

### Requirement: 核心功能集界定

套件 SHALL 在矩阵中显式标注核心功能集，共四簇：agent workbench 相关（agent-workbench、permission-flow）、项目管理（project-session-actions 及 state-store/projects、webui projects 面）、会话管理（session-lifecycle、session-persistence、acp-session-mapping）、models 管理（acp-model-selection、gateway-model-aliases、provider-management）。核心集的增删 MUST 是显式变更：矩阵标注与变更说明同步更新，不得静默调整。

#### Scenario: 核心行有标注

- **WHEN** 审阅验收矩阵
- **THEN** 四簇对应的条目带"核心"标注，80% 复核仅统计这些条目

#### Scenario: 边界调整留痕

- **WHEN** 某能力条目被加入或移出核心集
- **THEN** 矩阵标注更新且变更说明记录了这次调整

### Requirement: 旅程用例形态

验收用例 SHALL 是多步旅程级场景：以真实二进制（沙箱拓扑）经进程间真实边界（HTTP、核心通道、文件系统）串联多个能力，断言面向操作员可见结果。套件 MUST NOT 在进程级孤立重测单元层已覆盖的单点契约；无旅程命中的能力簇才新增用例。沙箱隔离、平台门控、显式超时、失败保留现场 MUST 遵循 `process-e2e-suite` 能力的同一约定。

#### Scenario: 一条旅程跨多个能力

- **WHEN** 一条验收用例执行
- **THEN** 它串联至少两个能力的用户可见行为（如 provider 治理：overlay 修改 → admin API 校验 → 网关路由生效），并在矩阵中标注其命中的全部条目

#### Scenario: 不重测单点契约

- **WHEN** 某 requirement 已有单元/集成测试完整覆盖且无旅程级缺口
- **THEN** 矩阵行引用既有测试作为命中证据，不新增验收用例

### Requirement: 一键入口与诊断

套件 SHALL 提供单条命令入口 `invoke accept`：构建工作区二进制后运行全部验收用例，退出码如实反映结果；`--case <用例名>` SHALL 透传为过滤器以手动单跑。用例失败时 MUST 保留沙箱目录与日志并打印路径（同 `process-e2e-suite` 约定）。

#### Scenario: 一键全量验收

- **WHEN** 操作员执行 `invoke accept`
- **THEN** 构建完成后全部验收用例运行，任一失败则非零退出，全部通过则零退出

#### Scenario: 手动单跑与现场保留

- **WHEN** 开发者执行 `invoke accept --case <用例名>`，且该用例失败
- **THEN** 仅该用例运行，其沙箱目录与 core/webui 日志被保留并打印路径

### Requirement: native 链路验收策略

native 内核链路 SHALL 先经 spike 验证沙箱内 `SEBAS_AGENT_GATEWAY_URL → debug gateway` 通路（in-process 形态）：可通则 native 旅程用例纳入套件；不可通则该面转豁免并把 cause 与证据记入矩阵。detached 形态的 native 验收 MUST 等 `wire-webui-sebas-agent-e2e` 落地后补入，此前不计缺口。

#### Scenario: spike 结论入账

- **WHEN** native 通路 spike 完成
- **THEN** 矩阵中 native 相关条目标注为"已纳入（用例 id）"或"豁免（cause=通路不可通，证据…）"

### Requirement: 套件运行预算

全套验收用例 MUST 以显式超时为界，单用例与全套总时长 SHOULD 有预算上限（单用例 ≤30s、全套 ≤5 分钟量级）；用例 MUST 以 `#[ignore]` 标注不进默认 `cargo test`，平台门控遵循 `process-e2e-suite` 约定。

#### Scenario: 默认路径不受扰

- **WHEN** 开发者运行默认 `cargo test`
- **THEN** 验收用例不执行，既有测试全绿不受影响

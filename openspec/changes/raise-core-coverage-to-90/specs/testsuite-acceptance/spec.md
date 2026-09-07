# testsuite-acceptance (delta)

## MODIFIED Requirements

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

## MODIFIED Requirements

### Requirement: 核心功能集界定

套件 SHALL 在矩阵中显式标注核心功能集，共四簇：agent workbench 相关（agent-workbench、permission-flow）、项目管理（project-session-actions 及 state-store/projects、webui projects 面）、会话管理（session-lifecycle、session-persistence、acp-session-mapping）、models 管理（acp-model-selection、router-model-aliases、provider-management）。核心集的增删 MUST 是显式变更：矩阵标注与变更说明同步更新，不得静默调整。

#### Scenario: 核心行有标注

- **WHEN** 审阅验收矩阵
- **THEN** 四簇对应的条目带"核心"标注，80% 复核仅统计这些条目

#### Scenario: 边界调整留痕

- **WHEN** 某能力条目被加入或移出核心集
- **THEN** 矩阵标注更新且变更说明记录了这次调整

### Requirement: native 链路验收策略

native 内核链路 SHALL 先经 spike 验证沙箱内 `SEBAS_AGENT_ROUTER_URL → debug router` 通路（in-process 形态）：可通则 native 旅程用例纳入套件；不可通则该面转豁免并把 cause 与证据记入矩阵。detached 形态的 native 验收 MUST 等 `wire-webui-sebas-agent-e2e` 落地后补入，此前不计缺口。

#### Scenario: spike 结论入账

- **WHEN** native 通路 spike 完成
- **THEN** 矩阵中 native 相关条目标注为"已纳入（用例 id）"或"豁免（cause=通路不可通，证据…）"

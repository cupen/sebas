## ADDED Requirements

### Requirement: Transcript ingest stays responsive under burst

单回合摄入突发转写条目（数百条量级）SHALL 不长时间阻塞主线程——不出现秒级以上的交互/快照冻结；渲染层可采用摘要或虚拟化，但摄入完成后完整转写 SHALL 保持可滚动、可读。

#### Scenario: flood turn stays interactive

- **WHEN** 一个回合连续投递约 1200 个条目
- **THEN** 摄入期间页面保持可交互（无多秒级冻结），摄入完成后转写完整且可滚动

### Requirement: Settings surface detail corrections

设置面的细节行 SHALL 如实、可读地呈现：About 的 Rust toolchain 行 SHALL 呈现探测值；配置了最低版本界限时 SHALL 一并呈现，未配置时 SHALL NOT 渲染悬空的界限标签（不得出现有标签无值的行）；Env Vars 表格在常规宽度下 SHALL 保持每列可读（不得一词一行挤压、长值不得贴面板边缘）；Add-project 弹窗的目录列表、分隔符与相邻区块 SHALL 在任意滚动位置保持清晰间距。

#### Scenario: toolchain row shows its bound

- **WHEN** 操作员打开 Settings → About
- **THEN** Rust toolchain 行显示探测到的工具链版本；若配置了要求的最低版本则同 show 其界限，未配置则不出现悬空界限标签

#### Scenario: env vars table readable

- **WHEN** Env Vars 表格渲染含长值的行
- **THEN** 各列保持可读，无一词一行换行、无边缘贴碰

#### Scenario: add-project modal spacing

- **WHEN** 目录列表滚动到最后一行
- **THEN** 分隔符与相邻区块之间仍保持清晰间距，不与列表底边拥挤

### Requirement: Workbench operational polish

工作台运营细节 SHALL 保持可信：会话行的 last active 计时 SHALL 持续更新（不冻结在创建值）；多条 toast SHALL 依次堆叠而不相互重叠遮盖；同名/同 id 的 agent 或场景重复创建 SHALL 有可见提示；工作台模型 chip SHALL 如实呈现会话生效模型及其来源（fake/测试模型不冒充真实模型名）。

#### Scenario: last active keeps ticking

- **WHEN** 会话列表展示超过一分钟
- **THEN** 行内 last active 计时随时间更新，不冻结

#### Scenario: toasts stack visibly

- **WHEN** 短时间内连续触发多条操作反馈
- **THEN** toast 依次堆叠呈现，任一条不被其它条遮挡致不可读

#### Scenario: duplicate creation warns

- **WHEN** 操作员以已存在的 id 创建 agent
- **THEN** UI 给出可见的重名提示，不静默产生歧义条目

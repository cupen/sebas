## MODIFIED Requirements


### Requirement: Agent form covers spawn-critical fields

The agent create/edit form SHALL cover the spawn-critical fields (agent id, display name, driver/形态, binary path, sessions dir, work dir, launch args) and SHALL validate before save: a missing agent id (or a reserved id) SHALL surface a field-level error and block the save.

The form's validation and save SHALL read the values currently present in the fields: after the operator types a valid agent id (by keyboard or any trusted input path), a save attempt SHALL NOT report the field as empty while the rendered field displays a non-empty value. Saving a valid form SHALL create the agent in the store without a restart, and the form SHALL be re-openable afterwards: closing the form (save, cancel, or ✕) SHALL leave the settings modal able to open a fresh, empty create form again. A form that renders while its input fields fail to render, or that cannot be opened on a subsequent attempt after a prior open, is a violation.

#### Scenario: GUI-created agent is spawn-ready

- **WHEN** 操作员经设置表单以完整字段（含 sessions 目录、work 目录、参数）创建 agent 并用其开session
- **THEN** 会话以表单填写的目录与参数 spawn 成功并完成回合

#### Scenario: edit prefills display name

- **WHEN** 操作员打开已存有 display name 的 agent 的编辑表单
- **THEN** display name 字段回填存储值而非留空

#### Scenario: 保存读取表单实况值

- **WHEN** the operator opens the create-agent form, types a valid unique agent id plus a binary path, and clicks save
- **THEN** the agent is created (appears in the directory list, spawnable without restart) and no "agent id 必填" error is shown while the id field displays a value

#### Scenario: 表单可重复打开

- **WHEN** the operator closes the create-agent form (cancel or save) and clicks ＋新建 agent again in the same settings session
- **THEN** a fresh empty form opens with its input fields rendered and writable

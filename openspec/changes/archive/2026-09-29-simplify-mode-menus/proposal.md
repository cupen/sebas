# Proposal: simplify-mode-menus

## Why

主 spec `agent-workbench`（权限模式切换，源自 session-parallel-liveness-and-unread-polish）早已要求：mode 下拉选项标签 Title Case（Ask/Edit/Allow/Auto）、composer 不得渲染空/占位 mode 态、创建弹窗只携带 `ask|edit|allow|auto` 四词且预选 `ask`。实现侧只落地了 Title Case（add-agent-settings-and-session-titles 7.2），两个菜单（创建弹窗与 composer）仍各自渲染第五个空值占位项「默认（Ask）」——既是残留中文注解，也是对既有 spec 的违背，还是一条潜在故障路径：弹窗选中「默认」会把 `mode: ""` 原样上 wire，被后端词汇校验 400 拒绝（`valid_session_mode("")` 为假）。

## What Changes

- 创建弹窗（`new-session-dialog.ts`）删除空值首项 `<wa-option value="">默认（Ask）</wa-option>`：mode 选择恰为 Ask/Edit/Allow/Auto 四词，预选 `ask` 不变，wire 不再有可发的空值。
- composer 权限模式下拉（`workbench-composer.ts`）删除同一个空值兜底项：不再渲染任何空/占位 mode 态（对齐 spec「SHALL NOT render an empty or placeholder mode state」）。
- **模式描述以非内联通道保留**（评审修正：desc 不进菜单项标签，但要换方式提示）：`ModeOption` 增加 `description` 字段（恢复 7.2 移除的解释语义，中文措辞收敛进词汇源文件）；两个菜单的 `wa-option` 挂 `title` 原生悬浮提示；创建弹窗的 `wa-select` 加 `hint` 行，动态跟随当前选中模式；composer 因紧凑工具栏约束只走 title 通道。
- 删除随之失去全部消费点的常量 `MODE_DEFAULT_LABEL`（`mode-vocabulary.ts`）及其断言；`MODE_OPTIONS` 保持唯一词汇出处不变。
- 更新断言旧文案的单测与 Playwright 用例（`new-session-dialog.test.ts`、`workbench-composer.test.ts`、`tests/testsuite-webui/tests/mode.spec.ts`），并补描述通道（title/hint）断言。
- 不改 wire 词汇（`ask|edit|allow|auto` 不变）、不改后端校验、不动 dashboard 徽章的中文措辞（那是中文 UI 正文文案，非菜单注解）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

（无——本 change 是对主 spec `agent-workbench` 既有要求的**实现收敛**，不引入、不修改任何需求；spec 级行为零变化，故按仓库先例设 `skip_specs: true`。）

## Impact

- `sebas-webui/frontend/src/views/mode-vocabulary.ts`：删 `MODE_DEFAULT_LABEL`；`ModeOption` 增加 `description` 字段。
- `sebas-webui/frontend/src/views/new-session-dialog.ts` / `workbench-composer.ts`：删空值 option 与对应 import，更新注释；`wa-option` 挂 `title`，弹窗加动态 `hint` 行。
- 测试：`new-session-dialog.test.ts`、`workbench-composer.test.ts`、`tests/testsuite-webui/tests/mode.spec.ts`。
- 风险极低：纯前端展示面收敛；后端 `create_session` / `switch_session` 与状态库 schema 零改动。

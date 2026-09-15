## MODIFIED Requirements

### Requirement: 会话 mode 在 dashboard 可见可切

会话 dashboard SHALL 展示当前会话的 mode（含远端会话已有的 desired/effective 呈现），切换入口 SHALL 位于输入框底沿左端，与模型芯片、提交按钮同一工具条，提交后走中途切换端点。会话头部 SHALL NOT 渲染 mode 切换控件。mode 显示对远端节点会话沿用 effective/desired 差异化呈现（effective 缺失时只显 desired）。

#### Scenario: composer 创建表单的 mode 选择

- **WHEN** 操作者在创建对话框展开表单
- **THEN** 表单提供 mode 下拉，缺省项为「agent 默认」（不发送 mode 字段）

#### Scenario: 会话头部切换 mode

- **WHEN** 操作者在输入框底沿的 mode 下拉选择另一个 mode
- **THEN** 前端提交 `POST /api/sessions/{key}/mode`；成功后下拉显示的 mode 更新，失败显示非致命错误且保持原显示

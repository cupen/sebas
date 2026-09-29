## ADDED Requirements

### Requirement: 界面文案语言一致性

工作台界面以中文为基准语言。用户可见文案——确认框与对话框（标题、正文、按钮）、Settings 分区名与标题、表格表头、输入占位符、空态文案、统计行、title/tooltips、通知——SHALL 使用中文；SHALL NOT 出现同位面的中英混排（如全中文确认框夹一段英文正文、中文界面切占位符后变英文）。品牌名、产品 id（agent/模型/命令/协议字段名）与技术术语的英文形态不在此列。前端拼装的错误前缀 SHALL 与后端消息语言一致（不添加英文前缀）。

浏览器原生表单校验气泡 SHALL 以中文呈现校验消息（novalidate + 自定义消息或等价机制），SHALL NOT 弹出浏览器缺省英文气泡。

#### Scenario: 确认框整段中文

- **WHEN** 操作者触发任一确认类操作（Close 会话、删除 provider/skill、归档、移除项目等）
- **THEN** 确认框的标题、正文、按钮全部为中文，无英文段落

#### Scenario: 常驻界面文案中文

- **WHEN** 操作者浏览侧栏、Settings 各分区、Env Vars 表、usage 视图与新建会话对话框
- **THEN** 分区名、标题、表头、占位符、统计行均为中文（品牌/id/技术术语除外）

#### Scenario: 原生校验气泡中文

- **WHEN** 操作者在 setup 或其它表单提交缺填字段
- **THEN** 校验提示以中文呈现，不出现 "Please fill out this field." 类浏览器缺省英文气泡

#### Scenario: 错误前缀不混排

- **WHEN** 前端展示一条后端错误消息（如权限不足）
- **THEN** 呈现文本不含英文 "Error:" 类前缀，与后端消息语言一致

#### Scenario: 文案快照防回归

- **WHEN** 文案改动破坏 zh 基准一致性（快照断言失败）
- **THEN** 前端单测在 CI 报红，阻止混排回归

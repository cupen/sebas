## ADDED Requirements

### Requirement: Project path input fidelity

项目注册的路径输入 SHALL 保真：操作员键入的路径在提交与校验前不得被静默删除或改写任何字符（含 Windows 路径分隔符 `\`）。校验反馈 SHALL 对键入突发做去抖——不得每次键击都发起注定被拒的网络请求，也不得在提交前闪烁与真实原因不符的错误文案；拒绝时 SHALL 陈述真实原因。

#### Scenario: typed Windows path reaches validation intact

- **WHEN** 操作员在路径输入框键入含反斜杠的路径并提交
- **THEN** 提交的路径与键入字符逐一一致，校验按真实结果通过或给出真实原因的失败提示

#### Scenario: no per-keystroke rejection storm

- **WHEN** 操作员正在键入一个尚不完整的路径
- **THEN** UI 不逐键击发起被 400 拒绝的请求，也不显示误导性错误（如「路径不存在」）直到提交或去抖窗口结束

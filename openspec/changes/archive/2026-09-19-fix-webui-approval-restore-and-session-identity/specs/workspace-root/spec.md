## MODIFIED Requirements

### Requirement: 范围判定是规范化且 fail-closed 的

范围判定 SHALL 把候选路径与 workspace root 都解析到真实路径（canonicalize）后做逐分量前缀比较。候选路径无法解析时 SHALL 判为越界；workspace root 自身无法解析时 SHALL 判定一切候选路径越界（fail-closed）。符号链接逃逸（根内路径经链接指向根外）SHALL 判为越界。判定拒绝 SHALL 面向操作者呈现具体的拒绝原因（如「路径在 workspace root 之外」「路径不存在」），注册界面 SHALL 在给出原因的同时保持提交控件禁用，不允许只禁用不解释。

#### Scenario: 符号链接逃逸被拒绝

- **WHEN** workspace root 内的某个目录是指向根外目标的符号链接
- **AND** 以该目录作为项目路径发起注册或打开
- **THEN** 判定为越界并被拒绝

#### Scenario: 相对路径穿越被拒绝

- **WHEN** 候选路径携带 `..` 分量且解析后落在 workspace root 之外
- **THEN** 判定为越界

#### Scenario: 无法解析即越界

- **WHEN** 候选路径不存在或无法 canonicalize
- **THEN** 判定为越界（与确定的越界同罪，不区分文案）

#### Scenario: 手填越界路径给出禁用原因

- **WHEN** 操作者在注册界面手填一个 workspace root 之外（或不存在）的路径
- **THEN** 界面在路径输入下方显示具体禁用原因，且提交控件保持禁用

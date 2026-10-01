## ADDED Requirements

### Requirement: 目录选择器可新建子目录

添加项目（及会话对话框等复用 folder-picker 的入口）的目录浏览 SHALL 提供「新建文件夹」能力：在当前浏览的目录内创建一个子目录。创建 SHALL 受与 browse-dirs 相同的 workspace root 边界约束（复用既有 fail-closed 路径校验单点），且 SHALL 仅支持单层创建——父目录不存在、名称为空、含路径分隔符或 `..` 等非法名的请求 SHALL 被类型化拒绝。创建成功后 SHALL 无需手工刷新即可在树中看到新目录并选中进入。

#### Scenario: 边界内建目录成功

- **WHEN** 操作者在 workspace root 内的某个已存在目录下新建名为合法名的文件夹
- **THEN** 目录被创建，树中即时出现并可进入
- **AND** 该目录可用于项目注册

#### Scenario: 越界与非法名被拒

- **WHEN** 创建请求的父目录越出 workspace root、父目录不存在、或名称含分隔符/`..`/为空
- **THEN** 请求被类型化拒绝并明确中文提示，文件系统无副作用

## ADDED Requirements

### Requirement: Spawn backend hint validation

Spawn 请求的 `backend` 执行体提示 SHALL 按以下语义处理：缺省（未携带字段）
默认路由到 ACP，保持旧客户端向后兼容；显式给出的值属于已知集合（`native`、
`acp`、`acp:<kind>` 前缀形式）时按既有路由语义分发；显式给出的值不属于已知
集合时 SHALL 返回 typed rejection 且 SHALL NOT 创建任何会话——未知执行体
SHALL NOT 静默回退为 ACP。

#### Scenario: 未知执行体提示被拒绝且不建会话

- **WHEN** 客户端以 `backend: "warp-drive"` 请求创建会话
- **THEN** 响应是 typed rejection（指名未知执行体），核心上不存在新会话

#### Scenario: 缺省提示仍默认 ACP

- **WHEN** 旧客户端发送不含 `backend` 字段的 Spawn 请求
- **THEN** 会话照常创建并路由到 ACP，行为与本变更前一致

#### Scenario: 显式 acp 与缺省等价

- **WHEN** 客户端以 `backend: "acp"` 请求创建会话
- **THEN** 会话创建并路由到 ACP，与缺省行为一致

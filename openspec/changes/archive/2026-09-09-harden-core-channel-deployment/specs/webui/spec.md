# Delta — webui

## ADDED Requirements

### Requirement: 全局核心可达性横幅

app-shell SHALL 提供全局"核心不可达"横幅：当 `/api/summary` 的 `reachability.ok` 为 false 时展示，内容含 cause（如 `socket absent`），呈现层级与现有"与服务器的连接已断开"横幅一致（全局、role=alert）；可达性恢复后横幅 SHALL 消失。横幅 SHALL 不阻塞页面其余部分的浏览（与"webui 在 core 不可达时继续服务"的既有语义一致）。

#### Scenario: core 不可达时横幅出现

- **WHEN** core 进程停止或通道不可达，浏览器停留在任意页面
- **THEN** 全局横幅出现且文本包含 `reachability` 上报的 cause

#### Scenario: core 恢复后横幅消失

- **WHEN** core 恢复服务且通道重新握手成功
- **THEN** 无需刷新页面，横幅在下一次可达性轮询后消失

### Requirement: 项目注册降级如实提示

`POST /api/projects` 在状态库（核心通道）不可用而落到本地文件注册表时，响应 SHALL 携带降级标记（含 cause 语义），前端 SHALL 就地提示"核心不可达，已写入本地注册表"一类的如实文案；状态库可用时响应不含该标记，前端无降级提示。两者皆失败时保持既有 503 行为。

#### Scenario: 核心不可达时加项目获得降级提示

- **WHEN** 核心通道不可达时通过 UI 注册一个合法项目目录
- **THEN** 注册成功（201）且响应携带降级标记，项目栏出现该项目并伴随降级提示，用户不再直到新建会话才得知核心不可达

#### Scenario: 核心正常时无降级提示

- **WHEN** 核心通道正常时注册项目
- **THEN** 响应无降级标记，UI 仅呈现常规成功路径

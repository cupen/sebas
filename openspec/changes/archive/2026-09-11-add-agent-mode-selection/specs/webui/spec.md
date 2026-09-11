## ADDED Requirements

### Requirement: 会话创建携带 mode

`POST /api/sessions` SHALL 接受可选 `mode` 字段，词汇为 `ask / edit / allow / auto`（与节点链路 `SessionMode` 一致）。缺省或 null 表示"agent 默认行为"，wire 上不携带 mode。未知词汇 SHALL 返回 400 指出非法值，SHALL NOT 静默降级为默认。`mode` 与 `agent`/`model` 一样在创建时记入会话映射；0-turn 占位会话 SHALL 记住请求的 mode，并在首条消息触发 spawn 时应用。远端节点上的 0-turn 占位仍按既有规则如实拒绝。

#### Scenario: 创建会话带 mode=allow

- **WHEN** 客户端 `POST /api/sessions` 携带 `{"agent":"claude","prompt":"...","mode":"allow"}`
- **THEN** 会话以 mode=allow 建立，后续快照可见请求的 mode

#### Scenario: 未知 mode 如实拒绝

- **WHEN** 客户端 `POST /api/sessions` 携带 `"mode":"plan"`
- **THEN** 返回 400 并指出 mode 非法，不创建会话

#### Scenario: 占位会话记住 mode

- **WHEN** 不带 prompt 创建会话且携带 `mode=edit`
- **THEN** 建立 0-turn 占位，不 spawn 子进程；首条消息到达时以 edit 的语义 spawn

### Requirement: 会话中途切换 mode

WebUI SHALL 暴露 `POST /api/sessions/{key}/mode`（请求体 `{"mode": "<ask|edit|allow|auto>"}`，同创建词汇与校验）。切换 SHALL 沿会话放置路径送达执行体：本机 claude 会话经 driver 运行时权限模式切换，远端节点会话经节点链路 SetMode。执行体接受与否 SHALL 经事件流反馈：接受后会话快照的 mode 更新；拒绝或执行体做不到时 SHALL 产生非致命错误事件（UI 显示错误、mode 保持原值），SHALL NOT 终止会话。

#### Scenario: 本机会话切换成功

- **WHEN** 对运行中的本机 claude 会话 POST `/api/sessions/{key}/mode` with `{"mode":"allow"}`
- **THEN** driver 收到运行时权限模式切换并回 `ModeChanged`；快照 mode 更新为 allow

#### Scenario: 执行体拒绝切换不致命

- **WHEN** 切换请求被执行体拒绝（如 agent 不支持该模式）
- **THEN** 会话收到一条非致命错误事件（UI 可见），快照 mode 保持原值，会话继续可用

### Requirement: 会话 mode 在 dashboard 可见可切

会话 dashboard SHALL 展示当前会话的 mode（含远端会话已有的 desired/effective 呈现），并提供切换入口（下拉/菜单），提交后走中途切换端点。mode 显示对远端节点会话沿用 effective/desired 差异化呈现（effective 缺失时只显 desired）。

#### Scenario: composer 创建表单的 mode 选择

- **WHEN** 操作者在创建模式展开表单
- **THEN** 表单提供 mode 下拉，缺省项为"agent 默认"（不发送 mode 字段）

#### Scenario: 会话头部切换 mode

- **WHEN** 操作者在会话头部选择另一个 mode
- **THEN** 前端提交 `POST /api/sessions/{key}/mode`；成功后头部 mode 更新，失败显示非致命错误且保持原显示

### Requirement: SessionBackend seam 承载 mode

`SessionBackend` 的 spawn / placeholder / 切换方法 SHALL 携带 mode 维度（与 agent/model/node 同构）：进程内后端把 mode 交给本机执行体映射；分离部署经核心通道 Spawn 帧携带 mode；远端放置把 mode 交给节点投影。 seam 的默认实现对 mode 的降级 SHALL 如实（不支持 mode 的后端不假装生效）。

#### Scenario: 分离部署 wire 携带 mode

- **WHEN** webui 与 core 分离部署、创建会话带 mode
- **THEN** 核心通道 Spawn 帧携带该 mode，core 侧按同一映射语义放置

#### Scenario: 不支持 mode 的执行体如实回报

- **WHEN** mode 发给无法生效它的执行体（如 native 内核、未声明 mode 能力的通用 ACP agent）
- **THEN** 创建成功但该执行体不声称 mode 生效；能回报 effective 的位置如实回报空

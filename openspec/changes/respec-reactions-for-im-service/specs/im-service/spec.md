## MODIFIED Requirements

### Requirement: IM 交互状态机随迁

卡片状态机（每会话单卡片、流式合并、预算与轮换——中立契约遵循 `channels`「Neutral presentation content contract」，飞书渲染遵循 `feishu-cards`）、权限审批卡、acknowledgment/阶段 reactions、命令解析、provider/settings 表单 UI SHALL 全部由 im 服务持有与执行，其行为规格分别遵循 `channels`、`feishu-cards`、`permission-flow`、`dispatch-commands`、`provider-management` capability。阶段 reaction 的渲染 SHALL 由 im 前端基于从核心会话通道观察到的 `SessionInfo.phase` 变化驱动（seed→working→terminal 相位机、同 emoji 不重发、旧 reaction 尽力移除的 swap 语义，见 `feishu-reactions`）；core SHALL NOT 为 IM 通道渲染 reaction 或发送聊天向卡片。im SHALL 把交互蒸馏为核心通道请求（会话消息、审批决定、状态库变更），SHALL NOT 本地实现任何会话语义。

#### Scenario: 权限按钮点击走通道回传

- **WHEN** 用户点击 im 渲染的权限卡按钮
- **THEN** im 把决定经核心通道 `ApprovalAnswer` 回传，由核心路由给对应执行体

#### Scenario: 表单提交走状态库

- **WHEN** 用户提交 provider 表单
- **THEN** im 经通道状态库接口（StateMutation）持久化，反馈卡片如实回报成功或失败

#### Scenario: 相位变化触发 reaction

- **WHEN** im 前端观察到某会话的 `SessionInfo.phase` 由 seed 变为 working
- **THEN** im 在该会话的卡片消息上应用 `OnIt` reaction（同 emoji 不重复发 API）

## ADDED Requirements

### Requirement: IM 前端渲染会话级 reaction

IM 服务的前端 SHALL 从核心会话通道的 `SessionInfo.phase` 推导并渲染会话级 reaction，遵循 `feishu-reactions` 的相位机、swap 与目标选择契约。core 进程 SHALL NOT 持有或发射 IM 向的 reaction 指令。

#### Scenario: 重启后由快照恢复相位

- **WHEN** core 重启且 im 服务持续运行
- **THEN** im 从重连后的会话快照重新取得各会话相位，并据此对齐卡片 reaction

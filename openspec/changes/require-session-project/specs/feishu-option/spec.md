## MODIFIED Requirements

### Requirement: 双通道共享会话状态

当 feishu 与 webui 同时启用时，两者 SHALL 汇聚到同一会话权威：webui 会话（`web-*` 前缀）与飞书会话（`oc_*` / `ou_*` chat_id）在同一个快照中可见，任何一侧创建/变更/移除的会话 SHALL 通过共享状态对另一侧可见。共享会话状态 SHALL 通过通道抽象与 `ChannelKey` 表达；`web-*` 与 `oc_*` 分别是 `web` 与 `feishu` 两个通道的 key，不由核心特判前缀。飞书通道是**唯一**允许会话没有项目目录的通道：飞书会话由聊天发起、本就没有项目目录，它们只在飞书面呈现、不进 webui 的 rail；webui 建立的会话一律从属于项目。

#### Scenario: 飞书会话出现在 webui 列表

- **WHEN** 一条飞书消息创建了一个会话
- **THEN** 该会话在 webui 的 `GET /api/sessions` 中可见，且不进任何 rail 分组（无项目归属的飞书会话是这条通道语义的例外，不是「无项目会话」的通例）

#### Scenario: webui 会话对飞书不可操作

- **WHEN** webui 会话（`web-*` 或 `agent-*` 前缀）已创建
- **THEN** 飞书侧不对它执行卡片操作，其生命周期由 webui 面承载

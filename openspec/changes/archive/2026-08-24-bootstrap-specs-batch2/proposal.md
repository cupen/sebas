## Why

承接已归档的 `bootstrap-specs`（试点 2 个 capability，模板已定，bead sebas-61l）。继续按功能域回填 baseline spec，本批覆盖**接入与会话核心闭环**：飞书桥接、会话生命周期、命令路由、状态持久化。这四块是其余能力的地基，优先回填可让后续 change 的 delta 有锚点。

## What Changes

- 新增 4 份 capability spec（英文，格式同试点）：
  - `feishu-bridge`：WS 长连接、事件去重、chat_type 过滤、@bot 提及、thread 回复、瞬时错误重试
  - `session-lifecycle`：chat→session 映射、懒 spawn、dormant→resume、双重 spawn 竞态防护、会话死亡清理
  - `router-commands`：slash 命令解析与分发（本地命令 / 转发会话 / watchdog 控制三路）
  - `session-persistence`：state.json v2 存取、v1→v2 迁移、损坏容忍、repair_mode、providers.json overlay
- 纯文档，无代码改动

## Capabilities

### New Capabilities

- `feishu-bridge`: Feishu WebSocket 接入层——长连接生命周期、入站事件解析与去重、会话类型过滤、群聊 @bot 判定、话题(thread)路由、出站 API 调用与瞬时错误重试。
- `session-lifecycle`: 会话生命周期——SessionKey 映射、首条消息懒 spawn、Spawning 占位防竞态、dormant 恢复、终端错误后的映射/allowlist/卡片清理、重启懒恢复。
- `router-commands`: slash 命令面——解析规则（转义/透传）、各命令的路由去向（本地处理 / 转发活跃会话 / watchdog RPC）、无会话时的行为、/btw 插队语义。
- `session-persistence`: 持久化状态——state.json/providers.json 布局、版本迁移、损坏回退默认、原子写、repair_mode、default_selection 语义。

### Modified Capabilities

（无）

## Non-goals

- **不**覆盖卡片渲染细节（`feishu-cards`，下一批）
- **不**覆盖媒体下载细节（`feishu-media`，下一批）
- **不**覆盖 provider 解析与三模式（`provider-management`，下一批）
- **不**覆盖 watchdog 控制协议细节（`watchdog`，第四批）；本批只写到「命令转发给 watchdog」这一层
- **不**改代码；**不**为已废弃行为（ACP bridge）补 spec

## Impact

- 新增 `openspec/specs/{feishu-bridge,session-lifecycle,router-commands,session-persistence}/spec.md`
- 代码零改动；依赖无变化
- 归档后 `openspec/specs/` 共 6 个 capability

## Why

承接 sebas-61l 回填计划（batch2 已归档 4 份，累计 6 个 capability）。本批覆盖**用户体验层与 LLM 网关层**：卡片渲染、表情状态机、provider 管理、网关核心、网关鉴权限流。

## What Changes

- 新增 5 份 capability spec（英文，格式同前）：
  - `feishu-cards`：卡片结构与流式更新、长内容截断/折叠、元素上限、主题、帮助卡
  - `feishu-reactions`：emoji 反应状态机（确认→处理中→完成/失败）与换绑语义
  - `provider-management`：/provider 卡片 CRUD、三模式（Off/Direct/Gateway）spawn 时 env 解析、model 探测
  - `gateway-core`：双协议端点、模型路由表、透传引擎（SSE/非流式）、错误翻译
  - `gateway-auth-rate-limit`：下游 key 鉴权、RPM 令牌桶、usage 落盘、访问日志
- **范围调整**：原计划的 `feishu-media` 不单独成 spec——生产代码中入站媒体只传 file key（无下载），该行为已在 `feishu-bridge` 的 "Inbound media events pass file keys only" requirement 覆盖；单独成 spec 会是空壳
- 纯文档，无代码改动

## Capabilities

### New Capabilities

- `feishu-cards`: 交互卡片渲染与流式更新——每轮新卡、内容分区（标题/引用/思考折叠/工具调用/输出）、累积 patch 更新、长内容截断与折叠、元素数量上限、主题色、帮助卡分组与就地切换。
- `feishu-reactions`: 消息表情反应状态机——入站确认 emoji、阶段换绑（移旧加新）、终态 emoji、权限等待时的表现。
- `provider-management`: provider 全生命周期管理——/provider 主卡（模式按钮/默认下拉/列表/CRUD 表单）、model 探测、spawn 时三模式 env/args 解析（含 --model 优先级与 provider 错误中止）。
- `gateway-core`: LLM 网关核心——/v1/* 嗅探与 /anthropic//openai 显式挂载、model→provider 路由（精确/glob/命名空间/默认回退 + 改名）、字节级透传（SSE 与非流式）、超时/取消、双协议错误格式。
- `gateway-auth-rate-limit`: 网关安全面——下游 key 鉴权（Bearer/x-api-key）、per-key RPM 令牌桶、usage tee（SSE/JSON 增量解析 + jsonl 落盘）、访问日志。注：代码中不存在每日 token 配额（仅一条过时注释引用），不写入 spec。

### Modified Capabilities

（无）

## Non-goals

- **不**覆盖 watchdog 控制面与升级流程（batch 4 的 `watchdog`，原 `upgrade-command` 并入）
- **不**覆盖 webui / cli-service / replay-debug（batch 4）
- **不**覆盖 state.json 文件格式（已归档的 `session-persistence`）；本批 `provider-management` 只引用其存储语义
- **不**改代码；废弃设计（如 ACP bridge）不写入

## Impact

- 新增 `openspec/specs/{feishu-cards,feishu-reactions,provider-management,gateway-core,gateway-auth-rate-limit}/spec.md`
- 代码零改动
- 归档后 `openspec/specs/` 共 11 个 capability

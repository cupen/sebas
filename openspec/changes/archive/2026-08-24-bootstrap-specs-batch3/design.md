## Context

前两批已建立 6 个 capability（permission-flow、acp-driver、feishu-bridge、session-lifecycle、router-commands、session-persistence）。本批 5 个覆盖 UX 层与网关层。批内关系：

```
feishu-bridge（通道，已归档）
   │ 每条用户消息一张卡
   ▼
feishu-cards ◀──内容── router 命令/会话流（已归档）
feishu-reactions ◀──状态── 同上
   │ spawn 时 env
   ▼
provider-management ──(Gateway 模式)──▶ gateway-core + gateway-auth-rate-limit
```

## Goals / Non-Goals

**Goals:**

- UX 行为以「用户在飞书里看到什么」为契约：卡片何时建、何时更、终态长什么样
- 网关行为以「协议面」为契约：端点、路由、透传保真、错误形状
- provider 管理覆盖**卡片交互**与**spawn 解析**两个半边（存储格式已在 session-persistence）

**Non-Goals:**

- 不写卡片 JSON schema 逐字段细节（实现细节）；写结构与行为约束
- 不写网关内部性能参数的具体默认值除非它们是行为契约（如上限保护）

## Decisions

### D1: `feishu-media` 撤销独立 capability

研究确认生产路径只做 file-key 标记（`compose_media_prompt`），`download_file` 是不可达代码。feishu-bridge 已有 requirement 覆盖。空壳 spec 违背「只反映当前状态」。若未来接通下载，再以 change 增加 `feishu-media` capability。

### D2: 卡片与反应拆成两份 spec

反应（emoji 状态机）发生在**用户消息**上，卡片发生在**bot 消息**上——是两个独立可观察面，渲染引擎也不同（reaction API vs card API）。合并会让「卡片的 requirements」与「反应的 requirements」互相稀释。

### D3: provider-management 跨两个 crate（router + src/spawn_env）

与试点 D2 同理：卡片 CRUD 在 router，env 解析在 src。按行为环路切：用户在卡上做的每个动作 → 落到哪个存储 → spawn 时产生什么 env。`--model` 优先级链（default_selection.model > overlay default_model > 不加）是核心契约，单独成 requirement。

### D4: gateway 两份按「数据面 / 控制面」切

- `gateway-core`：请求进来 → 路由 → 透传出去（数据面）
- `gateway-auth-rate-limit`：你是谁 → 允不允许 → 记多少账（控制面 + 记账）

### D5: 网关契约测试即 spec 证据

gateway/tests 的 contract test（mock upstream + 端点矩阵全绿）是行为的机器可读版；spec 的 scenario 直接从测试断言提炼，双向可追溯。

## Risks / Trade-offs

- [网关行为研究依赖测试文件较多] → Mitigation: agent 被要求给 file:line；写 spec 时抽查关键路由/错误路径
- [卡片渲染细节多，spec 可能过长] → Mitigation: 只写行为约束（何时建卡/更卡/终态），不写布局细节
- [5 份一次归档 review 负担] → 每份独立可审

## Migration Plan

纯文档。validate --strict → archive。回滚 = 删对应 5 个目录。

## Open Questions

无。

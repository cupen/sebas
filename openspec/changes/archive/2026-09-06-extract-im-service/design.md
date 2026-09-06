# extract-im-service — Design

## Context

事实基础（详见 proposal.md 的 Why）：

- core（`sebas core`，rename-cli-surface 前为 `sebas run`）进程内：feishu 适配器（WS/client/渲染，`sebas-feishu`）→ `ChannelEvent` → `sebas-dispatch`（原 sebas-router，rename-cli-surface 更名）的 IM UX 层（card_state / card_events / cards_ui / commands / crud + `engine/`（inbound / acp_events / maps / events）+ msgid/perm/allowlist 映射）→ `Out` 枚举 → `src/dispatch.rs`（ACP 执行与飞书发送混装）。
- 核心会话通道已有：`Snapshot/Spawn/Message/Close/Turns/Subscribe/StateSnapshot/StateMutation/ApprovalAnswer`（NDJSON over Unix socket，watchdog 注入 `SEBAS_CORE_SECRET`）；detached webui 即其客户端。
- `SessionEvent` 只在映射变更时发布（Created/Updated/Removed/Resync）；turn 内容（`TurnEntry`）逐 delta 落 `turn_log` 但**不发事件**——通道客户端靠拉取 `Turns`（webui 即「SSE 事件触发 + 防抖重拉」）。
- webui 的拆分模式：`SessionBackend` trait（InProcess / CoreChannel 两实现），协议类型留在 root crate（`src/core_channel/`），`sebas-webui` 不依赖协议。
- 前置：`wire-webui-sebas-agent-e2e`（进行中，2/11）动同一片 `core_channel`/`run.rs`，需先落地。

## Goals / Non-Goals

**Goals:**
- im 成为独立进程与独立 crate，core 二进制不再依赖 `sebas-feishu`/`sebas-im`。
- 飞书侧用户可见行为保持不变（卡片、审批、命令、表单、reactions、线程回复）。
- 通道协议扩展保持 additive，webui 旧客户端不受影响。
- 每个里程碑可独立验证、可回退。

**Non-Goals:**
- 不实现第二个 IM 适配器；不改 webui 行为；不动 gateway。
- 不把 turn 内容改成服务端推送流（见 D3，必要时另立项）。
- 不做 im 多实例；不合并 provider 状态存储。

## Decisions

### D1 im 的代码来源与 crate 边界

新 crate `sebas-im`，迁入：

| 来源 | 内容 |
|---|---|
| `sebas-dispatch` | `card_state.rs`、`card_events.rs`、`cards.rs`（CardConfig）、`cards_ui.rs`、`commands.rs`、`crud.rs`、`engine/inbound.rs` 的 IM 交互面、msgid/perm_cards/allowlist 映射、`engine/acp_events.rs` 的卡片累积面 |
| root crate | `src/reactions.rs`、`dispatch.rs` 的飞书呈现半边（send/update card、reaction、ack、topic 感知发送）、`run.rs` 的飞书装配（token/hello/test-msg/ws-dump）、`webui_cmd.rs` 的 card-config 加载模式 |
| `sebas-feishu` | 原样保留为库，宿主改为 im |

`sebas-dispatch` 保留：`SessionMap`、turn_log（transcript 是核心的会话内容）、ACP 事件→transcript 的入账、session events、`state_store`、`settings.rs`（settings.json 文件读取，DB 优先逻辑留在 core，im 经通道读）。

`Out` 枚举瘦身：执行类指令（`SpawnAcp/SpawnResume/SendAcp/WebSpawn` 等）留在 core 内部；呈现类指令（`SendCard/UpdateCard/UpdateCardByMsgId/React/AckMsg/PlainText/HelpText`）从 `Out` 删除，im 自建出站渲染队列。

备选：把 IM UX 留在 core 只拆传输（thin split）——否决，违背「消息处理代码从 sebas-dispatch 拆出」的目标。

### D2 im↔core 的接缝：port trait（webui 同款）

`sebas-im` 定义并只面向一个窄端口 `CoreSessionPort`（snapshot / ensure_message / close / cancel / turns / subscribe / approval_answer / state_snapshot / state_mutate / service 探活），root crate 的 `src/im_cmd.rs` 提供经 `core_channel::client` 的实现并装配 feishu 适配器。im 逻辑层（卡片状态机、命令、表单）对协议零依赖。

理由：协议类型在 root crate（webui 既有格局），port trait 复制 webui 的成功模式、避免为协议再做 crate 手术；将来若协议下沉共享 crate，只是换一个 impl。

### D3 卡片重建的输入：Turns 拉取 + SessionEvent，不用推送流

im 侧每会话维护 view-model（原 CardState 的角色），输入：

- `SessionEvent`（Created/Updated/Removed）：相位（👀→🚧→✅ 对应 `SessionInfo.phase`）、生命周期、卡片轮换触发；
- `Turns` 增量拉取：有活跃 turn 的会话以 ~250ms 定时轮询 + 事件触发即拉，抵达后在 im 本地按 150ms 防抖合并出卡（与今日 `feishu-cards` 节奏规格一致）；
- usage footer：`SessionInfo` additive 增列 `usage` 字段（serde default；core 把 usage 累计从 CardState 挪到映射上随快照/Updated 发布）。

备选（否决，留作后手）：通道加 per-session turn 推送订阅——协议面更大、core 侧要 diff，轮询不达节奏再立项。

thinking 折叠、长文截断、工具面板等渲染策略全部由 im 的 `[card]` 配置解释（core 已把 thinking/tool 文本入账 transcript，im 拿到的是内容不是事件语义）。

### D4 协议扩展（全部 additive）

1. `EnsureMessage { key, message }`：未知 key 自动建会话、dormant 会话懒复活——把 router 今日的 auto-spawn/复活语义暴露给 IM 前端（spec「IM 前端语义」）。旧 `Message` 行为不变。
2. `Cancel { key }`：取消在飞 turn（`/cancel` 需要）。
3. 审批面扩展到 ACP 桥：ACP `PermissionRequest` 也以 `SessionStreamFrame::ApprovalRequested` 推流，`ApprovalAnswer` 按 request_id 路由回 ACP 会话；无客户端连接时 fail-closed（原生内核既有姿势）。建立在 wire-webui 落地后的原生审批面之上。
4. `SessionInfo` 增 `usage`（serde default，见 D3）。

### D5 控制命令与表单

- 控制命令：im 进程持 watchdog 注入的控制凭据，复用既有 control RPC 客户端直发 watchdog（root crate 装配注入 port，或经 `sebas-ipc` 的既有客户端——以复用不复制为准）；`/upgrade` 等不再绕道 core。
- `/settings` 与 `/provider` 表单：im 经 `StateSnapshot/StateMutation`（settings/providers 域）读写；若 CRUD 面有缺口，additive 扩域，不另开旁路。

### D6 进程/部署/watchdog

- CLI：`sebas im -c <config>`（镜像 `sebas webui`）；`sebas core` 移除 `--test-msg`/`--dump-inbound`（随迁 im 子命令参数）。
- watchdog：`ServiceName::Im`、托管表加 im 条目、`[watchdog.im]`（enabled 缺省跟随 feishu 启用判定，host 类字段同 `[watchdog.webui]` 形）；控制凭据照常注入。
- 配置归属：im 消费 `[feishu]`/`[card]`/`[media]`（download_dir 迁 im 侧）；core 解析器容忍这些节但绝不据此建连接。
- 部署物：ansible 剧本、升级/回滚服务清单、sandbox 调试食谱（core+im 两进程）同步。

### D8 图片双向链路（入站解析在 im，执行体真进模型）

范围裁决：双向都做（入站用户图 + 出站 agent 产出图）；webui 工作台面（composer 传图、turn 流渲染）**另立项**——通道协议按附件就位，webui 后续零协议改动接入。

**入站（用户图 → 模型）**：

1. im 激活 `sebas-feishu::media` 下载（今日死代码）：飞书 media API 流式落盘到 `[media] download_dir`，超过 `[media] max_file_size` 拒绝并纯文本反馈；
2. im 把 `ChannelEvent::Media` 蒸馏为「本地附件引用」（`{path, mime, file_name}`）随 `EnsureMessage` 上通道——同机部署（Unix socket）路径共享成立，不传字节流；
3. core 校验附件路径存在（不存在 typed rejection），按执行体投递：native 走既有 `ContentBlock::Image`（读文件 Base64，`media_type` 从 mime）；ACP 走 `ContentBlock::Image(ImageContent)`，`InitializeRequest` promptCapabilities 协商 `image` capability——agent 不支持时**如实降级**为 `[图片: <路径>]` 文本标记并告知用户该执行体不收图。

**出站（agent 产出图 → 用户）**：

1. 执行体图片来源 = 工具结果/agent 消息里的 image 块（native `ContentBlock::Image` 已建模；ACP 会话更新可携带 image 块）；
2. core turn_log 增 `kind = "image"` 条目（`{path, mime}`，position 单调，serde additive 旧客户端可解析）；
3. im 从 Turns 增量拉到图片条目 → 上传飞书换 `image_key`（会话内同文件复用 key）→ 卡片 2.0 `img` 元素；上传失败降级为路径文本行。图片元素计入既有卡片预算/轮换。

**备选取舍**：通道传 base64 字节流——否决（报文膨胀、socket NDJSON 不适合大 payload；同机路径已够）；URL 直塞飞书——否决（飞书卡片 img 只认 `image_key`，必须上传）。

### D7 落地顺序（三里程碑，每步可验证）

- **M1 crate 内聚（纯重构，行为零变化）**：建 `sebas-im`，按 D1 迁代码，core 仍以库方式链接它跑既有 in-process 路径；router 测试随迁。此形态是脚手架，不是交付形态。
- **M2 detached im 服务**：D2 端口 + D4 协议扩展 + im 逻辑切换到「通道驱动」，`sebas im` + watchdog 托管 + e2e（core+im+webui 三进程）绿；此阶段 core 的旧 in-process 飞书路径仍在（供对照与回退）。
- **M3 彻底剪断（BREAKING flip）**：删 core 的飞书装配与 `sebas-feishu`/`sebas-im` 依赖、`Out` 呈现变体、`dispatch.rs` 飞书半边；升级文档/剧本同步。

## Risks / Trade-offs

- [卡片保真回归：渲染输入从进程内 ACP 事件变为 turn 流] → 既有 feishu-cards 断言测试随迁 im 并改造为「Turns+事件驱动」输入；用 replay journal（`replay-debug` capability）对照旧输出；真实飞书端到端仍需操作员凭据，沙箱内如实标注边界。
- [审批卡时序（stale click、迟到应答、无连接 fail-closed）跨进程后更微妙] → 复用 request_id 严格关联既有规格；wire 测试覆盖迟到/重复/未知 request_id 的 typed rejection。
- [轮询延迟 vs 推送] → 250ms 轮询 + 150ms 本地防抖与今日 PATCH 频率同量级；不达标再立项推送流（Non-goal 已留）。
- [双视图漂移（im 缓存 vs core 权威）] → core 单一权威规格不变；Resync + 快照收敛 + 诚实降级（core-session-channel 既有 requirement）。
- [工程量 ~4-5k 行迁移] → 三里程碑各自 e2e 绿再进下一步；M1/M2 期间行为可对照。
- [图片是新的外部依赖面：飞书 media 下载/上传的限流与失败] → 下载/上传复用 feishu-bridge 既有「业务错误刷新 token 重试 ≤3 次」规格；失败一律如实反馈或文本降级，不静默丢图。
- [大图内存峰值] → 下载与读取流式/分块，`max_file_size` 双向硬顶；Base64 注入 native 前校验大小。
- [与 wire-webui-sebas-agent-e2e 的文件冲突] → 时序前置（先落地再动工）；本 change 的审批面增量按其落点续写。

## Migration Plan

1. 前置：`wire-webui-sebas-agent-e2e` 合入。
2. M1 → M2 → M3 顺序落地（D7），每里程碑独立提交。
3. 部署：升级后 watchdog 按 `[watchdog.im]` 拉起 im；回滚 = watchdog rollback 回上一 release（im 条目随托管表消失，core 独立可跑）。
4. BREAKING 面向的是「core 进程内直挂飞书」的部署形态——发布说明明确：飞书部署必须升级为 core + im 两服务形态。

## Open Questions

- `EnsureMessage` 用新请求变体还是给 `Message` 加 ensure 标志——实现期按 serde 兼容性定，两者都满足 spec。
- control RPC 客户端复用点（root 装配注入 vs `sebas-ipc` 现成客户端）——以「不复制协议代码」为准在 M2 定。
- provider 表单 CRUD 对状态库域面的缺口清单——M2 对着 `crud.rs` 逐操作盘点，缺口走 additive 扩域。
- ACP 子代理 `image` capability 协商失败的降级 UX 细节（文本标记措辞、是否附缩略信息）——M2 对着 fake-claude 扩展收图断言时定，不影响协议形状。
- fake-claude 测试桩的图片断言形态（echo 图尺寸/mime 即可，不需真识别内容）——M2 实现期定。

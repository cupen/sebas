# Tasks: decouple-feishu-channel

依赖前置:无(不依赖 `add-state-store`;`core-session-channel` 已归档,协议在场)。
明确决策:抽中立通道抽象 + 允许改协议 + webui 也进适配层(design D1–D9);不实现第二个 IM/agent 适配器(Open Question 留待独立 change)。

## 1. sebas-channels crate(中立抽象)

- [x] 1.1 新建 `sebas-channels` crate(workspace member),定义 `ChannelName`、`ChannelKey{channel, reference}`、`ChannelEvent`(Text/Media/ButtonCb/FormCb)与 `ChannelCard`(中立累积呈现模型:标题/正文/思考/工具/用法/交互元素/生命周期)。验证:`cargo build -p sebas-channels` 通过,`ChannelKey` 为不透明 String reference(不暴露 chat_id/thread_id 语义)。
- [x] 1.2 定义 `ChannelAdapter` trait(ChannelName/spawn/shutdown/render)与 `AdapterRegistry`(名称→adapter、query 活动/健康)。验证:单元测试覆盖注册/查询/未注册报错;`Cargo.toml` 无飞书依赖。

## 2. core 与 webui 协议类型上移(允许 BREAKING)

- [x] 2.1 `core_channel` 协议 `SessionKey` → `ChannelKey`(protocol.rs 的 Request/Response、server.rs 的 dispatch、client.rs 的 SessionBackend),wire shape 同步调整。验证:`core_channel/tests.rs` 改夹具后全绿;`grep` 确认核心/协议无 `sebas_feishu::events::SessionKey`。
- [x] 2.2 webui 的 `routes.rs`/`models.rs`/`session_backend.rs` 从 `SessionKey` 换 `ChannelKey`;`encode_session_key`/`decode_session_key` 改为编码 `ChannelKey`(channel + 不透明引用),移除 `web-*`/`oc_*`/`ou_*` 前缀特判。验证:`cargo build -p sebas-webui` 通过,`sebas-webui` 对 `sebas-feishu` 的依赖收敛为仅配置/适配(见 D9)。
- [x] 2.3 `sebas-router` 领域类型换 `ChannelKey`(`router/mod.rs` 的 update_card/reaction/reply、`native_bridge.rs` 的 default-native 语义);`router/inbound.rs` 的 `dispatch(&FeishuIn)` 改为 `dispatch(&ChannelEvent)`。验证:`cargo build -p sebas-router` 通过;确认核心/协议/领域不再引用飞书事件与 Key 形状。
- [x] 2.4 `replay.rs` 从 `FeishuEnvelope` 改走 `ChannelEvent` 中立类型(JSON 记录格式随 `ChannelEvent` 调整)。验证:replay 测试用新夹具重放通过;确认 `replay` 不引用飞书。(实施说明:envelope 解析/门禁/翻译收敛到飞书边界 `ws_loop::ingest_feishu_frame`,dump 翻译后的中立事件;`replay` 只解析 `ChannelEvent`,零飞书代码引用;旧 replay 文件失效已按 D6 接受。)

## 3. feishu 适配器实现

- [x] 3.1 `sebas-feishu` 实现 `ChannelAdapter`(`feishu` channel):WS 出入站生命周期(现存 `ws_loop` 逻辑迁入)、将 `Feishu` 事件翻译为 `ChannelEvent`、将出站 `ChannelCard` 渲染为飞书 card schema 2.0 JSON 并调 API。验证:适配器单测覆盖"FeishuIn→ChannelEvent"(Text/Media/ButtonCb/FormCb)与"ChannelCard→card JSON(v2 button,无 V1 action)"。
- [x] 3.2 卡片渲染规则迁入 adapter(truncation/budget/rotation/thinking 折叠/主题,取自 `render_accumulated_card`);`feishu-cards` 全部渲染规则逐条保留。验证:迁移后的卡片渲染测试套件(现有 feishu cards 单测)全绿。
- [x] 3.3 飞书回调解析下沉到 adapter:adapter 维护"channel key → feishu message_id / thread target"映射,按钮/表单 callback 解析回 `ChannelKey`。验证:adapter 单测覆盖回调→`ChannelEvent`(source 还原);router 不再需要飞书 message_id 表(`maps.rs` 对应逻辑移除或下沉)。(**有意取舍**:回调解析与还原测试已落地;但 `MsgIdMap`/`ReplyTargetMap` 保留在 router——它们现按 `ChannelKey`/session_id 存**不透明字符串引用**,即中立 `CardRef` 概念的簿记,router 作为卡片生命周期(冻结/轮转/perm 翻转)的属主需要它;飞书语义(PATCH endpoint、线程 root)在 dispatcher/adapter 边界解释。决策记录见 design.md D10。)

## 4. 适配器注册与装配

- [x] 4.1 core bin(`run.rs`/`ws_loop.rs`)改为注册表装配:按配置实例化已启用 adapter(飞书 `[feishu] enabled`+凭据 → feishu adapter;webui `web` 常驻),adapter 经 `inbound_tx` 把 `ChannelEvent` 交给 core。验证:手动 feishu 关闭时进程不建立 WS、不取 token(现有 feishu-option 场景),开启时适配器注册。
- [x] 4.2 `src/config.rs` 的 `[feishu]`/`[card]` 段类型依赖从 `sebas_feishu::cards::CardConfig` 改为 `sebas-channels` 等价类型;`[card]` 渲染 knob 由 adapter 解释。验证:`cargo build` 通过,`[card]` 未知 key 仍拒绝(严格解析语义保留)。(实施说明:`[card]` 段保留 `sebas_feishu::cards::CardConfig` 作为反序列化目标——它是 feishu 渲染配置,由 adapter 解释;严格解析与 0600 原子写语义不变。)
- [x] 4.3 `sebas-router` 移除 `sebas-feishu` 依赖,`sebas-webui` 对飞书依赖收敛为适配器注册(不引用 `sebas_feishu::events/cards` 于核心路径)。验证:`grep sebas-feishu Cargo.toml` 确认 router 无该依赖;`cargo build ` 全 workspace 通过。(webui 的 feishu 依赖收敛归 5.x 收尾;router 已零依赖,`grep` 验证通过。)

## 5. webui 通道化 + 双通道共享验证

- [x] 5.1 `sebas-webui` 实现 `ChannelAdapter`(`web` channel)注册进 core;`SessionBackend` trait 已换 `ChannelKey`(2.2),读操作经 snapshot/events、写操作经 drive 方法。验证:webui 会话与飞书会话同 `ChannelKey{channel:web|feishu}` 平级,`GET /api/sessions` 双通道会话同现。
- [x] 5.2 端到端:webui 与 feishu 同时启用,任一侧创建/变更/移除会话,另一侧经共享状态可见(对应 feishu-option "双通道共享会话状态"、"webui 会话对飞书不可操作")。验证:沙箱(见 AGENTS.md)手动跑 webui+core,飞书会话进 webui 列表、webui 会话不进飞书卡片;`cargo test` 双通道集成用例通过。(沙箱记录:feishu disabled 形态验证——注册表仅 ["web"]、零 WS/token 活动;spawn→消息→typed rejection 全链路过 core.sock;kill core → reachability {ok:false, cause:connection refused} 诚实降级;重启 → 自动恢复 ok;SIGTERM → state dump + socket 清理。双通道同现由 cargo test `web_and_feishu_sessions_are_peers_in_one_snapshot` 覆盖——真实飞书凭据不可得,飞书侧入站无法沙箱验证,如实记录。)

## 6. 收尾

- [x] 6.1 清理核心对飞书形状的残留引用(`grep -rn feishu|Feishu|SessionKey` 于核心/协议/router/webui 核心路径应为零或仅配置适配器边界);`session_boot.rs` 等注释同步更新(去"飞书侧"表述)。验证:`grep` 确认无残留;`cargo build` 无新增 warning。(审计结果:core_channel/、sebas-router/ 零引用;src/ 仅适配器边界文件;webui 仅 server.rs 的 CardConfig 配置消费——D9 边界。)
- [x] 6.2 文档:`sebas-channels` 模块文档写清"核心只依赖中立抽象,新通道=新 adapter"的扩展路径;`docs/design-history.md` 记本次解耦决策(D1–D10 摘要)。验证:文档就位,`docs/design-history.md` 有本次记录。
- [x] 6.3 全量质量闸:核心/router/webui/feishu/replay 测试套件全绿,`cargo build` 全 workspace 无 warning;沙箱验证 feishu enabled 与 disabled 两种形态。验证:`cargo test --workspace` 通过,AGENTS.md 沙箱步骤跑通(webui 主控 + feishu 可选)。(结果:1094 passed / 0 failed;两个 insta 快照仅键序差异、语义一致后接受;沙箱 disabled 形态全链路验证,enabled 形态需真实飞书凭据,如实记录。)
# extract-im-service — Tasks

前置：`wire-webui-sebas-agent-e2e` 已合入 main。里程碑顺序 M1 → M2 → M3（design D7），每个里程碑收尾全量测试绿后再进下一步。

## 1. M1 — crate 内聚（纯重构，行为零变化）

> 实施调整（rename-cli-surface 落地后核对耦合面）：卡片机/命令/表单模块被 sebas-dispatch engine 直接消费（engine→UX 单向），而 sebas-im 的会话面又需要 engine 的 wire 类型——M1 阶段整体平移会制造 crate 环。调整为：M1 只迁与 dispatch 无耦合的模块并建 crate（sebas-im 单向依赖 sebas-channels/sebas-feishu，不依赖 sebas-dispatch）；UX 模块的物理迁移随 M3 割接（engine 摘除卡片逻辑后迁移即无环）。

- [x] 1.1 建 `sebas-im` crate 并加入 workspace（依赖 sebas-channels / sebas-feishu）；`src/reactions.rs`（ReactionTracker，自包含）整体迁入 `sebas-im::reactions`，root crate 改走 `sebas_im::` 路径。验证：`cargo build` 全 workspace 通过，reactions 全部单测在 sebas-im 内绿
- [x] 1.2 `sebas core` 的飞书装配（token 引导/SEBAS_TEST_FAKE_TOKEN 桩/hello/test-msg/adapter 实例化/入站 WS spawn，失败语义保持「spawn 失败记日志继续」）抽取为 `sebas_im::bootstrap`，run.rs 经装配入口调用。验证：`cargo test --lib` 全绿（220 通过）
- [x] 1.3 msgid / perm_cards / allowlist 映射、`engine/inbound.rs` 交互面、`src/dispatch.rs` 拆分——按上述调整移至 M3 割接（M2 期间 in-process 旧路径完整保留以对照）。验证：随 M3 覆盖
- [x] 1.4 M1 收尾：`cargo clippy -p sebas-im -p sebas` 无新增告警（存量告警属 rename 会话遗留、未触碰文件），全 workspace 编译绿。验证：完成

## 2. M2 — 核心通道协议扩展（additive）

- [x] 2.1 `EnsureMessage { key, message }`：协议变体 + serde 往返单测（旧报文仍反序列化）；core server 实现——ensure 臂跳过存在性预检，路由交给 backend（web_send_message 的 route_text：未知 key 建/复活 dormant）；`Message` 未知即拒绝的 webui 语义保持。验证：`ensure_message_spawns_unknown_key_and_message_still_rejects` 集成测试
- [x] 2.2 `Cancel { key }`：协议 + core 实现（`web_cancel_session` → `AcpCommand::Cancel`，会话保留可续用）。验证：`cancel_rejects_unknown_and_accepts_live_session` 集成测试
- [x] 2.3 `SessionInfo` 增 `usage` 字段（serde default 兼容旧快照/旧事件）；实施调整：usage 累计保留在 CardState，经 `session_info_for` 既有 join 随快照/Updated 发布（与 phase 同源，免映射结构变更）；`UsageUpdate` 事件触发 Updated 发布。验证：`session_info_usage_field_is_additive` serde 往返 + 旧形状兼容测试
- [x] 2.4 ACP 桥审批面上流：ACP PermissionRequest 以 `SessionStreamFrame::ApprovalRequested` 推流（建立在 wire-webui 原生审批面之上），`ApprovalAnswer` 按 request_id 回路由到 ACP 会话。验证：`acp_permission_request_streams_and_answer_routes_back` 通道全环集成测试（帧到达 + 决定路由 + 未知 id typed rejection；无连接 fail-closed 由内核既有路径保证）

## 3. M2 — sebas-im 通道驱动与独立进程

- [x] 3.1 定义 `sebas_im::CoreSessionPort` 窄端口（snapshot / ensure_message / close / cancel / turns / subscribe / approval_answer / state_snapshot / state_mutate），root crate `src/im_cmd.rs` 的 `ChannelPort` 提供 core_channel 客户端实现。验证：前端单测经 FakePort 全绿 + 通道层集成测试
- [x] 3.2 im 会话前端 view-model（`sebas-im/src/frontend.rs`）：订阅 SessionEvent（相位/生命周期/新轮判定/usage）+ Turns 增量拉取（250ms 活跃轮询）+ 卡片机复用（card_events 中立化为 `CardInput`，core 内 AcpEvent 与 im 进程 turn 流同机驱动）；卡片经 FeishuClient 直发（reply 线程 + 话题感知）。验证：`turn_stream_rebuilds_card_and_freezes_on_finished` 等前端单测
- [x] 3.3 命令面随迁接通道：`/sessions`（快照）、`/new`（close+ensure）、`/cancel`（Cancel 请求）、`/cost`/`/status`/`/compact`/`/btw`/普通文本经 EnsureMessage（core 的专属臂处理）；`/help` 卡本地渲染。验证：`text_ensure_and_command_distillation` 前端单测；dispatch-commands 断言保留于 sebas-dispatch（引擎路径回归绿）
- [x] 3.4 控制命令直连 watchdog：`src/im_cmd.rs` 的 `WatchdogControl` 复用 control RPC 客户端（ControlEnvelope + RpcActor::Cli），`/upgrade`/`/system`/`/router`/`/webui`/`/confirm` 由 im 直发；凭据缺失与调用失败如实纯文本反馈。验证：`sebas im` 进程冒烟（feishu 停用时干净拒绝）
- [x] 3.5 `/settings` 与 `/provider` 走 StateSnapshot/StateMutation：/settings 读 settings 域 + 校验写回（拒绝不落库）；/provider 读 providers 域列表卡，FormCb 带 op 时直通 StateMutation。**如实降级**：crud.rs 完整表单 UI（preset/custom 卡片流）未随迁，M2 以列表卡 + 指引过渡，记 bd 跟进。验证：前端单测（state 路由）+ 编译门禁
- [x] 3.6 权限卡全环（ACP 桥）：approval_loop 消费流上审批帧 → 渲染权限卡（复用 cards_ui::permission_card）→ ButtonCb 解析 decision → ApprovalAnswer → 就地翻卡（允许/拒绝/已过期置灰）。验证：`button_callback_routes_approval_answer` 前端单测 + 2.4 通道全环测试
- [x] 3.7 `sebas im -c` CLI（`--test-msg`/`--dump-inbound` 自 core 随迁）+ 诚实降级：SEBAS_CORE_SECRET 缺失/通道不可达时 WARN 并由端口层如实回错（不伪装受理）；feishu 停用时启动即干净拒绝。验证：进程冒烟（im --help、空配置拒绝路径）
- [x] 3.8 watchdog 托管 im：`ServiceName::Im` + `ManagedService::Im` 贯通 control_rpc/executor/services、`[watchdog.im] enabled: Option<bool>`（缺省跟随 feishu 启用判定，run_watchdog 接收 im_enabled_default）、ImSpawner 注入 control+core secret。验证：全 workspace 测试绿（watchdog 服务表既有断言全数通过）
- [x] 3.9 M2 端到端：`invoke e2e`（5 通过）+ `invoke accept`（6 旅程通过）在 M3 割断后全绿（core 割断后既有 webui/通道旅程不受影响）；**如实说明**：未新增独立的 core+im+webui 三进程 im 旅程用例（真飞书凭据不可得，im 进程冒烟覆盖 CLI/装配面），记 bd 跟进
- [x] 3.10 M2 收尾：全 workspace 测试绿（447 通过；sebas-agent 存量失败与 HEAD 基线一致、与本 change 无关——见 5.3）。验证：完成

## 4. M2 — 图片双向（入站 + 出站，design D8）

- [x] 4.1 通道附件协议：`Message`/`EnsureMessage` 增可选 `attachments`（path/mime/file_name，serde default 兼容旧报文）；core 校验附件路径存在，缺失返回 typed rejection。验证：serde 往返 + 旧报文兼容单测、`ensure_message_attachments_are_validated` 集成测试（缺失路径 Unavailable、存在路径 Ok）
- [x] 4.2 im 入站图片解析：`sebas-im/src/media.rs` 激活媒体下载（飞书 media API、大小上限校验、落盘 `[media] download_dir`）；`ChannelEvent::Media` 解析为结构化附件随 EnsureMessage 上通道；下载失败如实纯文本反馈。验证：媒体路径单测面（resolve 签名）+ 前端 on_media 单测路由
- [x] 4.3 core→执行体投图：服务端附件校验通过后以本地路径标记随文本投递，执行体经文件读取把图收进模型上下文（native 内核 message 模型本就支持 Image 块）。**如实调整**：ACP `ContentBlock::Image` + capability 协商未实现（acp_driver 现仅组 Text 块，随 ACP 图片能力立项），当前 ACP 路径以本地路径标记投递。验证：`ensure_message_attachments_are_validated` 集成测试（缺失路径 typed rejection、存在路径投递 Ok）
- [ ] 4.4 **延期立项**（上游缺口：`AcpEvent` 无 image 变体，agent 驱动不产出图片事件，出站图没有数据源；bd issue 跟进）：native 工具结果/agent 消息中的 image 块落 turn_log `kind="image"` 条目
- [ ] 4.5 **延期立项**（依赖 4.4 的 turn 图片条目）：Turns 图片条目 → 上传飞书换 `image_key` → 卡片 2.0 `img` 元素；上传失败降级路径文本行
- [ ] 4.6 部分完成：附件校验集成测试落地（4.1）；图片往返旅程依赖 4.4/4.5，随其立项补入 e2e/acceptance

## 5. M3 — 彻底剪断（BREAKING flip）

- [x] 5.1 core 移除飞书（进程级）：删 `run.rs` 飞书装配与 enablement 门、`dispatch.rs` 飞书呈现半边（dispatch_out/topic 感知发送/控制信封等，含对应死测试）、`--test-msg`/`--dump-inbound` 参数随迁；core 配置容忍 `[feishu]`/`[card]`/`[media]` 节但零动作。**如实调整**：单二进制形态下 `sebas im` 子命令仍链接 IM 实现（隔离口径为进程行为，proposal Non-goals 已记）；sebas-dispatch 库内 UX 模块保留（引擎已不消费呈现路径）。验证：编译绿 + core 单测/e2e/acceptance 全绿
- [x] 5.2 文档与部署同步：README 飞书节补双服务形态 + BREAKING 条目；ansible 配置模板注明 im 缺省跟随 feishu 判定（watchdog 子进程，无需新 unit）；glossary 增「im 服务」术语。验证：openspec validate 4/4；ansible 零强制变更（im 为 watchdog 子进程自动托管）
- [x] 5.3 收尾与记录：全量测试（447 通过；sebas-agent 存量失败与 HEAD 基线一致、与本 change 无关）、`invoke e2e` + `invoke accept` 全绿；bd 立项遗留项（ACP 图片块与出站图展示、provider 表单完整随迁、im 三进程 e2e 旅程、turn 推送流）。验证：bd issue 已建，git-finish 提交

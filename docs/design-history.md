# 设计决策史(ADR)

本文件收录 sebas 采纳 OpenSpec **之前**的关键架构决策,从 `docs/superpowers/` 历史文档蒸馏而来(该目录已于 2026-08-30 删除,原文均在 git 历史中,每条附可回溯路径)。采纳 OpenSpec 之后的「为什么」记录在各 change 的 `design.md` 中,不进本文件。

格式:日期 / 背景 / 决策 / 后果 / 原文。

---

## ADR-1 · 弃用 ACP bridge,经 cc-agent-sdk 直连 claude(2026-08-06)

**背景**:原链路 `sebas →(ACP/JSON-RPC)→ claude-acp-bridge →(stream-json)→ claude`,bridge 是没有语义增益的纯转码层:每会话 3 进程 + 每工具调用 1 个 hook 进程 + 1 条 unix socket;`acp-claude` + bridge ≈ 全库 1/3 代码做 1:1 转码;JSON-RPC dispatch loop 不可阻塞,带来 gate 锁 / `OwnedMutexGuard` / pump 必须 `cx.spawn` 的并发复杂度;权限链 4 进程经 broker 单 FIFO 位置配对,并行工具调用会错配;`session/load` 是死代码(bridge 声明 `load_session:false`);claude v2.1.220 envelope 变更曾穿透 bridge 导致事件静默丢失。

**决策**:弃用 ACP 线协议与 bridge 进程;复用 crates.io 的 `cc-agent-sdk`(pin 精确版本,适配层 `sebas-acp-claude/src/driver.rs` 为 SDK 类型的唯一接触点);内部事件词汇表 `AcpEvent`/`AcpCommand`/`Decision` 原样保留为 router 的稳定端口;权限传输走 SDK PreToolUse hook 进程内回调(spike 实证 `can_use_tool` option 在 0.1.6 是死字段);会话恢复用 claude 原生 `resume`;不做双引擎灰度,直接替换,git revert 即回退。

**后果**:每会话 3→2 进程;权限关联进程显式化;重启后真恢复对话历史;`acp-claude` crate 名称保留但只含直连 driver。（2026-08-30 更新:crate 已更名 `sebas-acp-claude`,见 openspec change `add-sebas-crate-prefix`。）

**原文**:`docs/superpowers/specs/2026-08-06-claude-direct-sdk-refactor-design.md` §1.1/§2(git 历史)

---

## ADR-2 · 卡片流重建为「状态在 router、节流在 pump」(2026-07-30)

**背景**:首版实现每个事件重建空卡整卡 PATCH 替换——历史被冲掉、Finished 时 transcript 清空;`ThinkingDelta`/`ToolProgress`/`ToolEnd` 落入 `_ => {}` 被丢弃(cards.rs 渲染分支成死代码);`[card]` 配置解析后零使用;text delta 高频时逐条 UpdateCard 撞飞书限流。

**决策**(方案 A):router 持 `session_id → CardState` 纯状态,pump 只管时序——事件先 `router.apply_event`(只更新状态不发 Out),重置 150ms debounce,到点 `router.flush_card` 序列化整卡发 `Out::UpdateCard`;`Finished`/terminal `Error` 立即 flush;复活三类被丢弃事件的渲染;接通 `[card]` 截断/fold 配置。

**后果**:卡片同卡累积、不刷屏;「router=状态,pump=时序」的职责分离延续至今(现为 `feishu-cards` capability 的行为基线)。

**原文**:`docs/superpowers/specs/2026-07-30-card-streaming-model-design.md` §2/§3(git 历史)

---

## ADR-3 · gateway 单端口双协议面 + per-key 配置简化(2026-08-06)

**背景**:Anthropic 与 OpenAI 客户端都要接入同一网关,两套协议路径有碰撞(`/v1/models`、`/v1/files` 等);初稿的 `[[gateway.keys]]`(per-key rpm/配额/模型白名单/key 级默认 provider)设计过重。

**决策**:bare `/v1/*` 挂载 + 协议嗅探(Anthropic 客户端必带 `anthropic-version` header,以此仲裁碰撞路径),辅以显式前缀 `/anthropic/v1/*`、`/openai/v1/*`;**纯透传**——不做协议转换,provider 协议面与请求协议不一致时返回明确错误;model 提取靠 body 缓冲重放 + 路径参数回退;`[[gateway.keys]]` 简化移除,下游只做 Bearer/x-api-key 匹配。

**后果**:单端口同时服务 `ANTHROPIC_BASE_URL`/`OPENAI_BASE_URL` 两类客户端;agent 模式下 gateway 对 Claude Code 永远暴露 Anthropic 协议面(OpenAI 路径表仅服务外部直连客户端,见 `sebas-gateway/src/proto.rs` 警告)。注:per-key 限流后来在演进中回归为 token-bucket 形态,现行契约见 `openspec/specs/gateway-auth-rate-limit/`。

**原文**:`docs/superpowers/specs/2026-08-06-gateway-design.md` §4.1/§4.2(git 历史)

---

## ADR-4 · provider 状态统一进 state.json(v0→v2)(2026-08-18)

**背景**:provider 数据分居两个文件——`~/.sebas/providers.json`(CRUD)与 `~/.sebas/state.json`(mode + default_provider_for_direct),各自独立 tmp+rename 原子写。「删除当前 default provider」需要两次写入,中间被杀则 mode 指向不存在的 provider,曾以 silent fallback 掩盖。

**决策**:合成统一 `state.json`;新增 `sebas-router/src/state_store.rs` 负责 v0/v1/v2 迁移 + 原子保存 + repair-on-load;`providers.json` 仅在首次迁移路径 B 中创建后立即删除,后续 CRUD 全部写入统一文件;`OverlayFile` 结构删除。同时 `default_provider_for_direct` 改为 `default_selection { provider, model }`(反序列化兼容 legacy 字符串与新对象两种 schema,无 V3 升级)。

**后果**:单文件单次原子写,半程崩溃不再产生悬挂引用;行为变更:Off + default_selection 已设 → 隐式 Direct;配置错误经 `SEBAS_PROVIDER_ERROR` 环境变量在 spawn wrapper 显式拦截,不再 silent fallback。后续 WebUI 等外部进程**不得**写 state.json(core 每 mutation 整文件原子重写)。

**原文**:`docs/superpowers/specs/2026-08-17-provider-design-review.md` §2.6/§2.8 及 §5 决策记录(git 历史)

---

## ADR-5 · provider 设计评审决策记录(2026-08-17 ~ 08-18)

sebas-63f epic 收尾评审列出 15 项设计问题,处置结果:

- **已落地 12 项**:§2.1 gateway 协议面 doc 警告、§2.2 解析失败显式 Error 通路、§2.3 form 自愈、§2.4 protocol radio、§2.5 Protocol 三重同名消歧(`AgentProtocol`/`WireProtocol`)、§2.6 状态文件合并(ADR-4)、§2.7 AgentDriver trait→inherent impl、§2.8 default_selection、§2.10 models.dev 经 xtask `update-models` 拉取、§2.11 探测按钮条件隐藏、§2.12 字段 doc、§2.15 KeyResolver seam。
- **§2.9 routes UI(唯一遗留)**:用户拍板——**routes 改由 webui 编辑,TOML `[gateway.routes]` 配置后续移除**;webui 工作单独立 bead 跟踪(见 beads 中「gateway routes 由 webui 编辑」issue)。

**原文**:`docs/superpowers/specs/2026-08-17-provider-design-review.md` §4/§5(git 历史)

---

## ADR-6 · watchdog 控制平面分期与升级策略(2026-08-14)

**背景**:WebUI 与飞书都要控制 watchdog(重启/升级/服务开关),需统一命令面与可信边界。

**决策**:私有 control RPC(`control.sock`,密钥 + peer uid 双重校验)是唯一命令面;分期 Phase 0 updater hardening → Phase 1 ControlService + RPC + auth/confirmation/events → Phase 2 watchdog 托管 WebUI ∥ Phase 3 飞书 proxy adapter → Phase 4 ServiceManager 生命周期模型 → Phase 5(可选)飞书 transport broker → Phase 6 公开 CLI/socket client。升级策略 P0:**core restart only**——`sebas update` 只重启 core child,watchdog 自身暂不 reexec,watchdog↔child IPC 保持至少一版向后兼容;新 core readiness 失败必须可恢复(记录原因、不无限自动重试、支持 rollback),触及控制面语义时响应须提示 restart required。

**后果**:Phase 0–4 已落地(ServiceManager / control_rpc / watchdog 托管 webui);Phase 5/6 未做;`add-core-session-channel` 的会话通道复用同一鉴权姿态与 wire shape。

**原文**:`docs/superpowers/specs/2026-08-14-watchdog-control-plane-design.md` §14/§17(git 历史)

---

## 附记 · 代码走查审计(2026-08-17)建议处置(2026-08-30 核对)

`docs/review/2026-08-17-code-design-audit.md`(本文件创建后即删除,原文在 git 历史)提出 5 条建议,删除前逐项核对:

| 建议 | 处置 |
|---|---|
| P1 webui 非 loopback 应在 watchdog 层拦截(而非子进程启动后自检、静默退出) | **未修**,开 bead 跟踪(检查至今在子进程 `webui_cmd.rs::run`) |
| P2 service_status 硬编码 "running" | 已被 Phase 4 ServiceManager 重构吸收——硬编码状态不复存在,状态源自监督句柄快照 |
| P2 run_watchdog 创建两次 ControlService | 已解——现单次创建(`watchdog.rs::run_watchdog`)经 Arc 共享 |
| P3 spawn_webui_process 过渡注释 | 已解/失效——该函数已随 ServiceManager 重构消失,模块文档保留 Phase 过渡说明(`webui_cmd.rs` 顶部) |
| P3 webui 降级行为(无 SEBAS_CONTROL_SECRET)不记录 | 已显式化——mutation 面无 secret 返回 503 + 明确错误文案(`sebas-webui/src/routes.rs`) |

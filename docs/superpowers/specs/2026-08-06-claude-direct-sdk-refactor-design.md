# sebas 弃用 ACP、直连 claude 协议重构设计文档

> 日期：2026-08-06
> 状态：待评审（**Phase 0 spike 已完成：GO**，见 §6.0；spike 报告 `~/workbench/spikes/cc-agent-sdk-spike/REPORT.md`）
> 作者：Claude
> 前置：[`2026-08-01-sebas-acp-bridge-design.md`](2026-08-01-sebas-acp-bridge-design.md)（bridge 原始设计，本设计落地后其作废）、[`2026-08-01-acp-bridge-permission-design.md`](2026-08-01-acp-bridge-permission-design.md)（hook/broker 权限流，同作废）、[`../../perm-flow/sequence.md`](../../perm-flow/sequence.md)（权限时序图，需重画）

## 1. 背景与目标

### 1.1 为什么放弃 ACP（2026-08-06 架构评估结论）

现行链路 `sebas →(ACP/JSON-RPC)→ claude-acp-bridge →(stream-json)→ claude` 中，bridge 是**没有语义增益的纯转码层**：sebas 消费的每个事件（TextDelta/ToolStart/Finished…）与 stream-json 原生事件一一对应。代价账：

| 维度 | 现状 |
|---|---|
| 进程拓扑 | 每会话 3 进程（sebas→bridge→claude）+ 每工具调用 1 个 hook 进程 + 1 条 unix socket |
| 代码量 | `acp-claude` 1084 行 + `acp-claude-bridge` 1442 行 ≈ **全库的 1/3**，做 1:1 协议转码 |
| 并发复杂度 | JSON-RPC 服务器约束：dispatch loop 不可阻塞 → gate 锁、`OwnedMutexGuard`、pump 必须 `cx.spawn`（server.rs:96-142） |
| 权限关联 | 4 进程、broker 单 FIFO channel **位置配对**，并行工具调用会错配 |
| 会话恢复 | **死代码**：bridge 声明 `load_session:false`，`session/load` 永远回退新会话；manager.rs 为走不通的路径复刻了 SDK 私有 handler（manager.rs:641） |
| drift 屏蔽 | 幻觉：bridge 自己解析 stream-json，claude v2.1.220 envelope 变更曾穿透 bridge 导致事件静默丢失 |

### 1.2 目标

- 删除 ACP 线协议、bridge 进程、PreToolUse hook + 权限 broker（含 `nc` 依赖、macOS 兼容 follow-up、位置配对隐患）
- 每会话 3 进程 → 2 进程；权限链 4 进程位置配对 → 进程内回调、显式关联
- 会话恢复从死代码变真功能（claude 原生 resume）
- **router / feishu / 卡片 UX / 命令集行为零变化**

### 1.3 非目标

- 不接第二个 agent（YAGNI；真来时以内部 trait 为缝，ACP 可作为该 trait 的实现回归）
- 不接 MCP、不改 media 链路、不搭 CI（各有独立 ticket）
- `/model`、`/cd`、`/status` 接线（SDK 已提供 `set_model()` 等能力，单独 ticket 跟进）

## 2. 关键决策

| # | 决策 | 选择 | 理由 |
|---|---|---|---|
| K1 | ACP 线协议与 bridge 进程 | 弃用 | §1.1：纯转码层，进程税/协议税/维护税三无增益 |
| K2 | claude 协议接入方式 | **复用 crates.io 的 `cc-agent-sdk`**（lib `claude_agent_sdk`，pin 版本），不自写 driver | 控制协议（can_use_tool / interrupt / set_permission_mode / resume / fork_session）已封装，API 对齐官方 Python SDK；省掉 spike + 自写 driver 的工作量 |
| K3 | 内部事件词汇表 | **保留** `AcpEvent`/`AcpCommand`/`Decision` 不变，SDK 适配在它之下 | 这是 router 消费的稳定端口，是好的接缝；router/run.rs/现有测试零改动即为本设计的验证标准 |
| K4 | 权限传输 | SDK **PreToolUse hook 回调**（进程内 async 闭包）→ 飞书卡 → 决策回传 | ⚠️ spike 实证：`can_use_tool` option 在 0.1.6 是**死字段**（定义但从未被读取，回调 0 次触发）；hooks 路径已双向验证（allow 执行 / deny 拦截）。删 hook/broker/socket/nc；SDK 内部按 control request_id 关联，位置配对隐患连根消失 |
| K5 | 会话恢复 | claude 原生 `resume`/`fork_session`（SDK options） | ACP `session/load` 在 bridge 下从未真正工作；改后 README known limitation（sebas-bob）直接消解。**语义变化**：重启后真恢复对话历史，不再是"静默开新会话" |
| K6 | 灰度与回退 | ~~transport flag~~ → **直接替换，git revert 即回退**（Phase 1 实施时修正） | 双引擎共存意味着同时维护两套 fake 协议线束（ACP + stream-json），测试面翻倍而收益为零；单用户 bot 的灰度需求用 git 历史即可满足。旧引擎在 Phase 1 删除，bridge crate 失去引用后由 Phase 4 清理 |
| K7 | SDK 供应链风险 | crates.io pin 精确版本 + `cargo vendor` 或 fork 预案 | SDK 低采纳度单维护者（见 §3.2）；适配层隔离保证最坏情况下替换成本有界 |

## 3. 尽调：cc-agent-sdk（louloulin/claude-agent-sdk）

### 3.1 能力对照（sebas 需求 ↔ SDK API）

| sebas 需求 | SDK 能力 | 出处 |
|---|---|---|
| spawn claude 子进程 | `ClaudeClient::connect()`（subprocess transport） | src/client.rs:134 |
| 发 prompt | `query()` / `query_with_content()` / `query_with_session()` | src/client.rs:321-483 |
| 流式事件（text/thinking/tool） | `Message` 流 + `include_partial_messages` | examples/10, 14 |
| **权限闸门** | PreToolUse hook 回调 → `HookJsonOutput`（`permission_decision: allow/deny` + reason）；入参 `PreToolUseHookInput{tool_name, tool_input, session_id, cwd}` | src/types/hooks.rs；~~`can_use_tool`~~ 死字段（0.1.6，spike 实证） |
| cancel turn | `interrupt()`（对齐 Python SDK） | src/client.rs:699 |
| 会话恢复/新会话 | options `resume` / `fork_session` | examples/16_session_management.rs |
| 换模型 | `set_model()` | src/client.rs:739 |
| 权限模式/回滚 | `set_permission_mode()` / `rewind_files()` | src/client.rs:719, 802 |

### 3.2 供应链事实（2026-08-06 核查）

- 包名 **`cc-agent-sdk`**（crates.io，8 个版本，最新 2026-03-15；workspace version 0.1.6）。⚠️ 与 crates.io 上另一个 `claude-agent-sdk` v0.1.1（2025-09-30，repository 链接指向不存在的 `anthropics/claude-agent-sdk-rust`）**不是同一个，勿装错**。
- License: **MIT**。edition 2024 / rust 1.85（sebas 为 edition 2024 / rust 1.90，兼容）。
- 依赖与 sebas 兼容：tokio 1.48、thiserror 2、reqwest 0.12 rustls、serde 1。
- 风险面：GitHub 18★ / 2 fork / 单维护者（"Loumos AI"）；repo main 最后 push 2026-02-21（约半年未动），crates.io 版本比 repo 新（版本不同步，pin 时以 crates.io 或 git tag 为准并复核）。
- 对冲：K7 的 pin + vendor/fork 预案；适配层（§4.3）隔离 SDK 类型，最坏情况退回自写 stream-json driver（bridge 里 200 行 parser 可搬回），沉没成本有界。

## 4. 架构与数据流

### 4.1 新拓扑（每会话 2 进程，无 hook 无 socket）

```
☁️ Feishu ──WSS(入)/HTTPS REST(出)──▶ sebas
                                        │
                                        │  crate: acp-claude（内部换引擎，API 不变）
                                        │  SessionManager ──▶ 适配层(driver.rs) ──▶ cc-agent-sdk ClaudeClient
                                        │                                          │ spawn
                                        │                                          ▼
                                        │                                    claude 子进程
                                        │                              (stream-json + 控制协议, stdio)
                                        ▼
                              AcpEvent / AcpCommand（词汇表不变）
                                        │
                                     router ──Out──▶ dispatch_out ──▶ Feishu
```

### 4.2 权限流（全部进程内，显式关联）

```
claude ──stdout: control_request(hook_callback, request_id)──▶ cc-agent-sdk
  ──▶ PreToolUse hook 回调（适配层）──▶ AcpEvent::PermissionRequest ──▶ router
  ──▶ 飞书权限卡 ──▶ 用户点击 ──▶ AcpCommand::PermissionReply ──▶ 适配层 oneshot
  ──▶ 回调返回 HookJsonOutput{permission_decision} ──▶ SDK ──stdin: control_response──▶ claude 执行/跳过
```

hook 脚本、broker、sidecar、nc、macOS nc 兼容 follow-up、位置配对 —— 全部删除。

**spike 实测补充**（修正事件映射表）：
- 工具结果在 **`Message::User`**（tool_result 块）而非 Assistant —— `ToolEnd` 从 User 帧映射；
- CLI 每 turn 刷约 80–100 条 `system/thinking_tokens` 帧 —— 适配层必须过滤，否则事件洪流；
- `Result` 帧带 `total_cost_usd`/`modelUsage` —— `/cost` 接线的数据源（范围外但已备好）；
- `/cancel` 语义：`interrupt()` 取消 turn 但会话进程不可复用（同 client 再 query 必失败）→
  映射为「interrupt + 标记死亡 + 下条消息懒 respawn(resume)」，sebas 已有该机制。

### 4.3 适配层（`acp-claude/src/driver.rs`，SDK 类型的唯一接触点）

`SessionManager` 公共 API 不变，内部映射：

| 现状（ACP SDK） | 改为（cc-agent-sdk） |
|---|---|
| `create_session` → AcpAgent spawn + initialize + session/new | `ClaudeClient::new(options)` + `connect()` |
| `AcpCommand::CreateSession/ContinueSession` → cmd channel | `query_with_content_and_session()` |
| SDK dispatch → `translate_update` → `AcpEvent` | `Message` 流 → `map_message` → `AcpEvent`（映射函数替换，事件不变） |
| `PermissionReply` → pending_responders 旁路 | 回调内挂起的 oneshot（**顺带消掉 AcpCommand 旁路问题**） |
| `Cancel` → session/cancel notification | `interrupt()` |
| `resume_session` → session/load 回退链 | options `resume` + `fork_session` 语义 |
| `kill`/`kill_all` → cancel_tx | `disconnect()` |

事件映射表（实现时以 Phase 0 抓取的真实帧为准）：

| SDK `Message`/内容块 | `AcpEvent` |
|---|---|
| assistant text block / partial text | `TextDelta` |
| assistant thinking block | `ThinkingDelta` |
| tool_use block | `ToolStart` |
| user tool_result block | `ToolEnd`（is_error → 失败语义） |
| result envelope（stop_reason） | `Finished`（refusal → `Error{terminal:true}`） |
| 子进程退出/传输错误 | `Error{terminal:true}`（保底语义同现状） |

## 5. 文件改动

**删除**（约 -1900 行 + 一个外部依赖）：
- `acp-claude-bridge/` 整个 crate（src 1442 行 + tests + build.rs + vendored hook）
- `hooks/pretooluse.sh`
- workspace `agent-client-protocol` 依赖；bridge 的 uuid/tracing-subscriber 等随之减少
- `Cargo.toml` workspace members 中的 bridge 条目

**重写**（公共 API 不变）：
- `acp-claude/src/manager.rs` —— 内部从 agent-client-protocol SDK 换成 cc-agent-sdk 适配
- 新增 `acp-claude/src/driver.rs` —— 适配层（§4.3），内含小 trait 便于 mock 测试

**修改**：
- `Cargo.toml`：加 `cc-agent-sdk = "=0.1.6"`（精确 pin，K7）
- `src/config.rs`：`[acp.claude] transport` flag（K6）
- `README.md`：架构描述、删掉 hook 注册部署负担、known limitation（sebas-bob）消解、config 示例注释修正（"under the hood 加 flags" 的 drift 一并修掉）
- `docs/perm-flow/sequence.md`：重画权限时序（hook/broker 移除）
- 三篇 2026-08-01 bridge spec 头部加 `> 已被 2026-08-06 设计取代` 标注

**保留（零改动即验证标准）**：
- `router/`、`feishu/` 全部
- `acp-claude/src/session.rs` 的 `AcpEvent`/`AcpCommand`/`Decision`
- `src/run.rs` 的 pump/dispatch 逻辑

**测试资产处理**：
- `acp-claude-bridge/tests/bin/fake_stream_claude.rs`：bridge 删除后失去作用；其 stream-json fixtures 已由 cc-agent-sdk 封装解析，适配层测试改为**构造 `Message` 值**断言 `AcpEvent` 映射（不再需要 fake 进程）
- `tests/full_e2e_test.rs` / `permission_flow_test.rs` 等：fake 对象从"fake bridge 进程"改为"mock driver trait 对象"

## 6. 迁移阶段（每阶段独立可验证、可回退）

| Phase | 内容 | 验证 | 规模 |
|---|---|---|---|
| 0 | **Go/No-Go spike**：cc-agent-sdk 起本机 claude 2.1.206，跑通 connect/query/流式事件/权限回调/interrupt/resume 六件事；记录真实 wire 帧（喂给 `record` 留 fixture） | ✅ **已完成：GO**（权限改走 hooks 路径；详见 spike 报告与 §8a/8b） | ~0.5d（实际） |
| 1 | acp-claude 内部换引擎（适配层 + trait），公共 API 不变；引入 `transport` flag | `cargo test --workspace` 全绿（router/feishu 测试零改动） | ~1-2d |
| 2 | 权限回调接 PermissionRequest/Reply；permission_flow_test 改编 | 权限 e2e 绿 | ~0.5-1d |
| 3 | resume 接 claude 原生 resume；restart_recovery_test 预期从"回退新会话"改"真恢复" | 恢复 e2e 绿；README 更新 | ~0.5d |
| 4 | 默认切 `direct`；观察一周；删 bridge crate + hooks + ACP SDK 依赖 + transport flag | 全量测试 + 手动冒烟清单（README §Manual smoke test） | ~0.5d + 观察期 |

**No-Go 退回路径**：Phase 0 若发现 wire 格式与 claude 2.1.206 不兼容或 SDK 行为有坑 → 决策点改为"自写 stream-json driver"（bridge parser 搬回 + 控制协议自实现），其余阶段不变。

## 7. 测试

- **零改动验证**：router/feishu 全部测试在 Phase 1 后原样通过 —— 证明词汇表接缝正确。
- **适配层单元测试**：构造 SDK `Message` 值 → 断言 `AcpEvent` 映射，用例覆盖现有 `translate_update` 测试全集（session.rs 内联测试迁移）。
- **e2e**：mock driver trait 替换 fake 进程；保留 `tests/bin/fake-claude.rs` 的协议语义断言思路。
- **真实 claude 冒烟**：Phase 0/4 各跑一次 README 手动冒烟清单（hello → 权限卡 → /new → /sessions → 重启恢复）。

## 8. 风险与限制

1. **SDK 成熟度**：低采纳、单维护者、repo 半年未动 → K7（pin + vendor/fork）+ 适配层隔离；最坏情况退回自写 driver（spike 已用 ~100 行裸 stdio 走通权限全环，沉没成本上界有实证）。**fork 优先级上调**：见 8a/8b。
2. **wire 兼容性**：~~未验证~~ → **spike 已实测钉死**（fixtures 留存于 spike `out/wire-frames.jsonl`）。
3. **crates.io 与 repo 版本不同步**（2026-03-15 vs 2026-02-21）→ 以 crates.io 或 git tag pin 为准，安装后复核 `Cargo.lock`。
4. **interrupt 语义差异**：已实测 —— interrupt 取消 turn 但会话进程不可复用（同 client 再 query 必失败）；按 §4.2 spike 补充第 4 条映射（interrupt + 标记死亡 + 懒 respawn/resume）。
5. **resume 语义变化的用户可见性**：重启后真恢复历史（原为静默新会话）—— 这是功能修正，但需在 README 更新说明。
6. **不用 SDK 的 pool**：现状每会话一进程一 ClaudeClient，pool 属未来优化，不引入。

**8a.（spike 新增）SDK 解析器偏严**：`ThinkingBlock` 缺 `signature` 字段即整个流报错（interrupt 后实测触发 `missing field 'signature'`）。对策：fork 修 lenient 解析（`#[serde(default)]`），或适配层预先容错。

**8b.（spike 新增）SDK 默认不隔离宿主设置**：`setting_sources: None` 文档称不读文件系统设置，实测子进程仍加载宿主的 SessionStart hooks/agents/skills → 生产必须显式 `setting_sources: Some(vec![])`（或等效 flag），否则宿主用户的 hooks 会在 sebas 驱动的会话里执行（安全/行为双重污染）。

## 9. 范围外（独立 ticket）

- 第二个 agent 接入（届时以 driver trait 为缝）
- `/model`（SDK `set_model()`）、`/cd`、`/status` 接线（sebas-3ti）
- media 下载补全（feishu/media.rs 硬编码 bug 修复）
- CI 搭建（sebas-nya）与 wire-level fixtures（sebas-vw5.3，Phase 0 顺手产出）

## 10. 自审清单

- 范围：单架构决策 + 5 阶段迁移；每阶段独立可验证，符合"小步可回退"。
- 零改动验证标准（router/feishu/词汇表）写死在 §5/§7，防止重构漂移成行为变更。
- 供应链风险有对冲（K7 + §8.1），No-Go 路径明确（§6）。
- 文档同步（README/perm-flow/旧 spec 标注）列入 §5，防止再次出现文档漂移。

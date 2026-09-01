# sebas-agent (Phase 1a) — 深入设计

## Context

See `proposal.md` — Why. 塑造本设计的四个事实：

- **蓝图已定稿并提交**（`docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md`，f4b51c8）：D1–D7 决策、§7–§10 架构、C1–C9 验收 checklist。本 design 是蓝图的**实现级展开**，不重新裁定方向。
- **gateway 可选（用户决策 2026-09-01，修订蓝图 D3）**：agent 直接拿到 provider 数据直连上游（Anthropic 兼容端点），gateway 降级为可选的路由/计量层。本 design 的 N3 / N5 / N9 已按此修订。
- **`add-core-session-channel` 正在分支上实现**（SessionBackend 缝未合入）。本 change（Phase 1a）与其完全正交：内核不碰任何 UI 面，接线留给 1b。
- **crate 命名事实**：分支 `feat/sebas-crate-prefix` 表明仓库正向 sebas-* crate 命名演进（acp-claude → sebas-acp）。故 crate 定名 **`sebas-agent`**（蓝图中的 "agent-core" 降级为内核**能力域名**，即 spec 的 capability path；产品名同样是 sebas-agent）。
- **可复用范本**：`acp-claude` 的 SessionManager / AcpSessionHandle / AcpCommand 模式（acp-claude/src/manager.rs、session.rs:79 的 `AcpEvent`）与 gateway 的纯透传约定（`gateway/src/proto.rs:43`——"sebas 自身走 Gateway 模式时，agent 只发 Anthropic 协议"）。

## Goals / Non-Goals

**Goals:**

- 内核四模块（llm / loop / tools / session）落地，通过 fake LLM 驱动的场景测试达成 checklist C1 / C2 / C4 / C5 / C6 / C7 / C8。
- `cargo run --example agent-dev` 提供 headless 冒烟路径——不改 CLI 命令表。
- 为 1b 预留清晰的宿主接口：SessionManager 可被 webui / router 直接嵌入，实现未来的 SessionBackend。

**Non-Goals:**

- 不做 UI 接线、权限规则、沙箱、持久化、compaction、子代理、MCP（见 proposal Non-goals）。
- 不做真模型联调与评测（FakeLlmClient 即验收环境；真 provider / gateway 冒烟是 1b 的事）。

## Decisions

### N1 — crate 定名 `sebas-agent`，能力域保留 `agent-core`

沿 sebas-* crate 惯例（feat/sebas-crate-prefix 的方向），且"sebas 自身的 agent"这个立项名就该是 crate 名。备选 `agent-core`：弃——与仓库命名演进相悖；`agent-core` 保留为 spec 能力域名与蓝图词汇。

### N2 — Phase 1a/1b 切分：1a headless（本 change），1b 接线（等 channel）

备选：等 channel 合并后再立项——弃。内核（循环/工具/LLM 客户端）与 SessionBackend 缝**完全正交**：内核只暴露"创建会话 / prompt / cancel / 事件流"的进程内 API，谁宿主它、事件往哪推，都是 1b 的 adapter 细节。早做早用 fake 验收，不阻塞在任何分支上。

### N3 — 运行时拓扑：纯 library + example 宿主

```
        Phase 1a（本 change）                     Phase 1b（等 channel 合并）
┌─────────────────────────────────┐    ┌─────────────────────────────────────┐
│ examples/agent-dev.rs（宿主）    │    │ webui / router（宿主）               │
│        │ SessionManager         │    │        │ SessionBackend 缝（adapter）│
│        ▼                        │    │        ▼                            │
│ ┌─────────────────────┐         │    │ ┌─────────────────────┐             │
│ │ sebas-agent crate   │         │    │ │ sebas-agent crate   │             │
│ │ session/ loop/      │         │    │ │ （同一内核，零改动）  │             │
│ │ tools/   llm/       │         │    │ └──────────┬──────────┘             │
│ └──────────┬──────────┘         │    └────────────┼────────────────────────┘
└────────────┼────────────────────┘                 │ HTTP（Anthropic 协议）
             │ HTTP                                 ▼
             ▼                            LLM 端点（Anthropic 协议）
      直连 provider（默认）或可选 gateway → 上游
```

1a 的宿主只有 example（进程内直调 SessionManager）；1b 换宿主时内核零改动。备选：1a 就内置一个 HTTP 服务暴露会话——弃，那是 channel 正在做的事，重复建设。

### N4 — 并发模型：每会话一 task，mpsc 命令 + broadcast 事件

镜像 `acp-claude` 的成熟模式（未来宿主零学习成本）：

```
SessionManager
  │ create_session(workdir) → SessionHandle（可克隆）
  ▼
┌─ 每会话 tokio task ──────────────────────────────┐
│  cmd_rx: mpsc<SessionCmd>{Prompt, Cancel}        │
│  event_tx: broadcast<AcpEvent> ──► 所有订阅者     │
│  state: turn 状态机 + Vec<Message> 历史           │
└──────────────────────────────────────────────────┘
```

- `SessionCmd::Prompt(text)` 入队后由会话 task 串行处理；turn 进行中再收到 Prompt → 排队（语义与 router 现有会话一致）。
- `Cancel` 同时置 `CancellationToken`（供工具层立即感知）。
- broadcast channel 天然支持多订阅者（1b 的 SSE 订阅 + 日志），滞后订阅者用 `lag_recv` 容忍。
- 备选：`Arc<Mutex<State>>` 轮询——弃，流式场景推送是天然形态。

### N5 — LLM 客户端：reqwest 流 + eventsource-stream 解帧，`input_json_delta` 累积

`LlmClient` trait（crate 内抽象；生产实现 `AnthropicMessagesClient` 面向任意 Anthropic Messages 端点——直连 provider 或 gateway，`FakeLlmClient` 为测试实现）：

- POST `{base_url}/v1/messages`，鉴权头携带配置的凭证（直连 provider 为其 api key；经 gateway 为 gateway auth token）+ `anthropic-version`，body 带 `stream: true` 与 `tools` 数组（六工具的 name/description/input_schema）。
- Anthropic 流事件处理表：

| SSE 事件 | 处理 |
|---|---|
| `message_start` | 记录 message id、usage（预留预算统计） |
| `content_block_start`（type=tool_use） | 新建工具参数累积缓冲 |
| `content_block_delta`(text_delta / thinking_delta) | 立即发 `TextDelta` / `ThinkingDelta` |
| `content_block_delta`(input_json_delta) | 追加到工具参数缓冲，**不发事件** |
| `content_block_stop`（tool_use） | 组装完整 JSON → 待执行工具队列 |
| `message_delta` | 记录 `stop_reason`（tool_use / end_turn / max_tokens） |
| `message_stop` / `error` / `ping` | 收尾 / 报错 / 忽略 |

- 关键正确性点：**工具参数分片到达，必须在 content_block_stop 后才解析执行**（spec「Tool arguments arrive as fragments」场景）。
- 备选：手工解析 SSE 字节流——弃；`eventsource-stream` 很小，帧边界 case（多行 data、跨 chunk 分割）不值得手写。

### N6 — 工具执行器细节

| 工具 | 实现要点 |
|---|---|
| bash | `tokio::process::Command` + `process_group(0)`；超时/cancel 一律 `killpg`（孙进程不留孤儿，watchdog 有同款先例）；输出合并流式拉取，**尾部 30k 截断**；非零退出码 = `ok:true` 携带 `exit_code`（spec「Model recovers from a failed command」） |
| read | `tokio::fs` 按行读取，`offset/limit` 分页，行号前缀；目录/二进制（含 NUL 探测）→ 错误结果 |
| write / edit | 全部先做 read-before-write 检查（会话级已读集合）；原子落盘 tmp + rename（沿 `router/src/state_store.rs` 惯例）；edit 精确字面量匹配，匹配数 ≠1 且未开 replace_all → 错误**报实际匹配数** |
| glob | `walkdir` + `globset`，跳过 `.git`；收集到 100 条即停（不是遍历完再裁）；mtime 排序 |
| grep | `walkdir` + `regex`，`include` 用 globset 过滤文件名；逐行匹配按文件分组，250 条即停；不依赖 rg 二进制（跨平台、零外部依赖） |

备选：glob/grep shell 出 rg——弃（外部二进制依赖 + Windows 兼容）。Phase 3 若性能不足再换 `ignore` crate（ripgrep 的库内核）。

### N7 — 提示词装配与历史管理

- system = 身份段（"你是 sebas-agent，sebas 的原生编码代理……工具使用纪律：优先 read 后 edit、bash 失败自愈、不臆测文件内容"）+ workdir 说明 + AGENTS.md + CLAUDE.md（存在才注入，二者都有则 AGENTS.md 在前——spec「Memory files are injected」）。
- 历史 `Vec<Message>` 内存态：user/assistant/tool_result 消息全量保留；工具输出**入库前已在工具层截断**，历史层不再二次处理（1a 无 compaction，预算靠 7/N8 停境兜底）。
- 身份段文本落在 crate 内常量模块，1b 可外置配置。

### N8 — 取消与预算的机制层

- `CancellationToken` 贯穿 loop 主 select 与每个 `ToolCtx`；bash 额外 killpg。
- 会话 task 主循环：`tokio::select! { cmd = cmd_rx.recv(), ev = 当前步骤 fut, _ = sleep_until(deadline) }`——三条腿任一触发都有明确语义（新命令 / 步骤完成 / 预算到点）。
- 预算三计数：`model_calls`、`tool_calls`、turn deadline（默认 20 / 50 / 10 分钟，`[agent]` 可配）；超限 → `Finished` 事件携带 budget 标记（**不是** Error——spec 明确）。
- 失败分级：网络/HTTP 5xx → `Error{terminal:false}`；SSE 中途协议崩坏或 panic → `Error{terminal:true}`（沿用 `AcpEvent::Error` 的 terminal 约定，router 已按此移除映射）。

### N9 — 配置面：直连 provider 为默认，gateway 可选（2026-09-01 修订）

- **直连（默认）**：provider 数据直接可用——example/测试用 env：`SEBAS_AGENT_PROVIDER_BASE_URL`（如 `https://api.anthropic.com` 或任意 Anthropic 兼容上游）、`SEBAS_AGENT_PROVIDER_API_KEY`、`SEBAS_AGENT_MODEL`；宿主进程（1b+）可直接读 sebas 既有 provider 注册表（config.toml `[provider.*]` / provider overlay），按模型名解析出协议 / base_url / key。
- **经 gateway（可选）**：`SEBAS_AGENT_GATEWAY_URL` + auth_token——需要多 provider 模型名路由与用量计量时启用；对客户端只是另一组端点 + 凭证。
- 可选 `[agent]` 节：`max_model_calls` / `max_tool_calls` / `turn_timeout_secs`，缺省用 N8 默认值。
- 依据：用户决策（2026-09-01）——agent 不必访问 gateway，可直接拿到 provider 数据；headless 冒烟不再要求先起一个 gateway 进程。
- 备选：仍强制经 gateway——弃：wire protocol 完全相同，多一个必跑进程只增摩擦、无技术收益。

### N10 — 测试策略：FakeLlmClient 双模式 + 场景矩阵

- **脚本式**：预设 `Vec<LlmTurn>`（每轮 = 文本 + 工具调用列表），按序回放——验证 C1 多步循环、C2 事件序（delta 先于 ToolStart 先于 ToolEnd）、C8 预算。
- **有状态式**：闭包按上一轮 tool_result 动态生成下一轮——验证 C4 自愈（第一轮让 bash `exit 1`，第二轮 fake 看到失败结果后改发成功命令）。
- SSE 解析：录制的帧序列 fixture（含 input_json_delta 分片、跨 chunk 分割）做解析层单测。
- 集成测试不依赖真 gateway / 真 provider / 真模型；example 冒烟手动跑（直连 provider，或可选经 gateway）。

场景矩阵（验收 = spec 全部 Scenario 有对应测试）：

| spec 场景 | 测试形态 |
|---|---|
| 两会话事件不串扰 | 脚本式并行两会话 |
| 取消保历史 / killpg 无孤儿 | 有状态式 + 进程存活断言 |
| ≥5 工具多步循环 | 脚本式 |
| 预算三上限 | 脚本式 × 3（每个上限一个） |
| input_json_delta 组装 | 解析层 fixture 单测 |
| 六工具契约（读前写拒 / edit 匹配数 / glob 100 / grep 250 / read 分页 / bash 超时） | 工具层单测 × 6 |
| 自愈 / 非零退出码 | 有状态式 |
| AGENTS.md 注入 | 脚本式断言 system 内容 |

## Risks / Trade-offs

- [input_json_delta / 流事件对协议细节敏感] → 解析层独立模块 + 帧 fixture 单测锁定；协议升级只动一个文件
- [walkdir 全仓遍历在大仓库慢] → 上限即停（100/250）+ 跳过 `.git` + `target`；不够再换 `ignore` crate（Phase 3 议题）
- [无持久化，进程退出丢会话] → 1a 定位即内核验证，OQ1 已记录；1b 由宿主接 session-lifecycle
- [与 channel 分支并行演进的合并冲突] → 契约面只有 `AcpEvent` + 未来 SessionBackend 形状（蓝图 §9）；本 crate 不碰 router/webui，合并面为零
- [直连与经 gateway 都只覆盖 Anthropic 兼容上游] → gateway 纯透传不转换协议，agent 亦只说 Anthropic 协议——OpenAI 协议 provider 两条路都够不着；这是已知取舍，`LlmClient` 抽象为将来新增 OpenAI 协议客户端留位（1b+ 决策）
- [broadcast 滞后订阅者丢事件] → 1a 单订阅者无风险；1b SSE 接入时用 lag 策略（断线重放按 session 历史，属 1b 设计）

## Migration Plan

纯新增 crate + example，不部署、不迁移。回滚 = 删除 crate 目录。1b 接线时：宿主（webui/router）持有 SessionManager，实现 SessionBackend（channel 定义的缝），内核零改动。

## Open Questions

- **OQ3（延后）**：默认模型与 `/model` 的衔接——example 用 env 指定模型名，宿主策略留 1b。
- **OQ5（新增）**：身份段提示词的中英文与具体纪律条目——1a 先给最小可用版本，随真实使用迭代。
- **OQ6（新增）**：example `agent-dev` 的归宿——长期保留为调试入口，或转 `tests/bin/`（fake-claude 先例）后删除，1b 定。

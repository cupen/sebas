# Design: session-slash-commands

## Context

调研结论（源码级，两条 subagent 并行核实）决定了本设计的可行边界：

- **透传已天然成立**：claude 的 SDK/ACP 层与 opencode 的 ACP `prompt()` handler 都把首个文本块 `/` 前缀当命令解释；`session/prompt`/既有消息链路原样送达即可，sebas 无需翻译层。
- **opencode 静默吞陷阱**：未广告命令 → `stopReason: end_turn`、无 LLM 调用、无报错。前端拦截是唯一能诚实的层。
- **`/goal` 非「通用」**：claude 内建（`/goal <condition>`、`/goal clear`，装 Stop hook 跨回合追目标）；opencode 全仓无此命令。`/compact` 两家都有（opencode 硬编码 → `session/summarize`），但 opencode **不把它放进广告列表**。
- **两侧命令发现均已实现**：claude 经 `cc-agent-sdk` 0.1.7 `client.get_server_info()`（初始化握手含 `commands` 数组）；opencode 经 ACP `available_commands_update`——`agent-client-protocol` 2.1.0（workspace 实际解析 2.1.0/schema 1.7.0）已带 `SessionUpdate::AvailableCommandsUpdate` 变体，零依赖新增。
- **既有物化模板**：`AcpEvent::ModelChanged/ModeChanged` → session_boot 消费 → dispatch 引擎写会话状态 → `SessionInfo` 快照 → webui 载荷/WS `Updated`。命令表完全同构。

## Goals / Non-Goals

**Goals:**

- 命令表 = agent 自己广告的事实，per-session、可刷新、经快照流达 webui。
- composer 面板：增量过滤、两段式提交、键盘可达（复用 model chip 菜单的 listbox 键盘模式）。
- 未支持命令前端拦截，杜绝 agent 静默吞掉的空转回合。

**Non-Goals:**

- 见 proposal Non-goals：无翻译/无新会话框接入/无 `_meta` 扩展协商/无 IM 面板/不动 native 引擎词汇。

## Decisions

**D1 — 两条 intake 汇入一个事件变体。** `AcpEvent::AvailableCommands { session_id, commands: Vec<AvailableCommand> }`，`AvailableCommand { name, description, hint: Option<String> }`（serde 加 `#[serde(default)]` 保旧 fixture 兼容）。claude 路径：driver 建立会话后调 `get_server_info()`，防御性映射 `commands` 数组（字段形状随 CLI 版本漂移，缺字段置 None）；通用 ACP 路径：`acp_driver/codec.rs` 加一个 match arm。替代案（每 agent 一套 API）被否：事件词汇是 acp-driver 的稳定契约面。

**D2 — 物化进 SessionInfo 快照，不新增端点。** 引擎在 `apply_event` 收到 AvailableCommands 时写入会话状态；`SessionInfo` 增可选字段 `available_commands`（`#[serde(default, skip_serializing_if)]`，只增不破旧消费端），随既有快照/`Updated` 事件到 webui。替代案（独立 `GET /sessions/{key}/commands`）被否：多一个端点、多一条加载时序，快照模式已被 model/mode 验证。

**D3 — 拦截集 = 广告表 ∪ {compact}。** 纯广告表会误拦 opencode 的 `/compact`（可用但不广告）；纯放行则吞陷阱复活。前端判定：命令名 ∈ 广告表 ∨ `compact` → 放行；否则内联拦截。会话命令表为空（native 等）→ 不拦截不面板，`/` 即普通文本——native 引擎里 `/` 本无命令语义。该例外集在 spec 中钉为「universal built-in」，新 agent 若同样特例 compact 即自动覆盖。

**D4 — 面板复用既有组件模式。** model chip 菜单已实现 listbox + `menuKeydown` 键盘导航 + aria；命令面板同构（浮层定位到 textarea 上方）。增量过滤纯前端（前缀匹配，不区分大小写）。两段式 Enter 防止参数型命令（`/goal ...`）误发——用户选定。

**D5 — busy/queued 不特判。** 命令提交走 workbench-turn-queue 既有路径（排队态提交已支持）；`/compact` 在 turn 进行中到达 agent 的行为由 agent 自身语义决定，sebas 如实呈现结果即可。

## Risks / Trade-offs

- [claude `get_server_info()` 的 `commands` 字段形状未在 crate 文档钉死] → 防御性映射 + 沙箱实测真实 CLI 输出；解析失败 = 空表 = 面板隐藏（诚实退化，不炸会话）。
- [opencode 未来把更多命令特例化而不广告] → 拦截误伤面扩大；缓解：例外集可配置为常量表，出现真实误伤时按证据扩充。
- [`/compact` 在 claude 的无回显形态（无 user-message echo）] → transcript 呈现为 compaction 生命周期事件或静默完成，属 agent 侧事实，sebas 不补渲染（design 不扩大范围）。
- [SessionInfo 变大] → 命令表 < 30 条 × 短字符串，增量可忽略。

## Migration Plan

原子落地、单次回滚：① sebas-acp 事件变体 + 双 intake（含 fake-claude/fake-acp-agent 测试桩扩展）→ ② 引擎物化 + SessionInfo 字段 → ③ composer 面板 + 拦截 → ④ `cargo test` + vitest + 沙箱端到端（claude stub 与 opencode 配置各验一轮：面板、过滤、透传、拦截）。旧后端 + 新前端组合下命令表为空 → 面板隐藏，天然向后兼容。

## Open Questions

（无——清单来源、拦截规则、Enter 语义均经 grilling 裁定；`/goal` 的 opencode 缺位经源码核实并写入 spec。）

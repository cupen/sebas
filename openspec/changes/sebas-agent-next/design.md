# sebas-agent (Phase 2) — 深入设计

## Context

See `proposal.md` — Why. 塑造本设计的六个事实：

- **Phase 1a 已归档**（`openspec/changes/sebas-agent`，19/19 任务完成）：sebas-agent 四模块（llm / loop / tools / session）落地，六件套工具、三重预算、取消安全、AGENTS.md 注入全部通过 fake-LLM 场景测试；example `agent-dev` 提供 headless 冒烟（直连 provider 或可选 gateway）。
- **checklist 缺口**（蓝图 §5.3）：C3（危险操作门控）完全未做——写操作畅通、无审批、无 allowlist；C8 的"token 维度"未做——历史全量重放，长会话必然撞顶；C9（任务跟踪）未做；网络能力（web_search/web_fetch）未做——agent 无法调研外部资料。
- **参考对象已开源**：DeepSeek Harness 2026-08-13 开源（`deepseek-ai/deepseek-harness`，MIT，~208k stars）。蓝图 §4 靠第三方观测（S9）的机制——文件沙箱、一次性审批升级、todo/goal/job 编排、web_search/web_fetch——现在有第一手源码与官方文档可对照（见 §7），部分裁决需要修正（§7.1）。Codex（`openai/codex`）源码同样公开且 2026 年大幅演进（见 §7.2），蓝图 §3 的两个裁决需要更新：CX-3「单工具收敛路线」在 2026 架构中已被 `unified_exec`/`apply_patch`/`update_plan` + MCP + 多 agent + code_mode 取代，不再是"收敛"；CX-1「Linux 用 Landlock」实际默认已是 **bubblewrap + seccomp**（Landlock 降级为 legacy fallback，`use_legacy_landlock=true`）。
- **webui 缝已合入**：`add-core-session-channel` 的 tasks 1.1–1.3/2.1/2.3 已上 main——`SessionBackend` trait（`sebas-webui/src/session_backend.rs:61`）、`InProcessBackend`（`:105`）、`FakeBackend` 都在。Phase 2 的 webui 接线可以在这条缝上做**进程内**实现，不需要等待 channel 的 socket 部分（4.1–8.6）落地。
- **事件词汇的 1a 约束"零 PermissionRequest"是 1a 的，不是永恒的**：1a spec 说 "SHALL NOT emit permission-request events" 是"结构性保证不发出权限事件"。Phase 2 启用 `PermissionRequest` 是**能力成长**，不是违反 1a——1a 的语义是"我没做权限，所以不撒谎说需要审批"。Phase 2 补上权限后，发送 `PermissionRequest` 是诚实表达。消费端（webui 卡片 / 飞书）的 `AcpEvent` 词汇早已为 `PermissionRequest` 定义渲染面（`permission-flow` spec），只是此前没有生产者。
- **本 change 不依赖 channel 剩余部分落地**：所有交互面走 `run --webui` 进程内 `InProcessBackend` 的同类缝；channel 的 socket 后端（4.1 起）与 Phase 2 并行推进、互不阻塞；飞书审批卡仍属 Phase 3（经由 feishu crate 的既有卡片能力）。

## Goals / Non-Goals

**Goals:**

- 权限与沙箱（C3）：统一 `PolicyEngine`（allow/deny/ask + 会话 allowlist + 一次性升级），webui 审查卡为首个回答者，bash 平台隔离尽力而为并如实报告。
- 网络工具面：web_search / web_fetch（默认拒、门控放行、硬上限）。
- 上下文管理第一步：工具结果改写（~8k 首段 + `[truncated]`）、Assembly 预算（max_messages + token 估算）、只读并行。
- 首个交互面：webui 进程内后端 + 审查卡 + 会话行后端选择（acp-claude / sebas-agent）。
- sebas-agent 首个 benchmark（agent-bench）：冒烟 CLI + 轨迹 JSONL + DAL 式 dashboard + 自愈与失败用例。
- 蓝图 §4/§11 对照更新（DSH 开源源码）。

**Non-Goals:**

- 不做上下文 compaction / 摘要（Phase 3a 范畴；本 change 只做"改写 + 预算"这一层）。
- 不做任务清单工具、agent 主动提问、plan mode、apply_patch、subagent、skill 系统、MCP（Phase 3+/4）。
- 不做 OS 级强沙箱承诺（Landlock/namespace 尽力而为；防火墙回退是**预设默认**）。
- 不接飞书审批卡、不接 CLI 命令表（example 与 benchmark CLI 例外）。
- 不做 OpenAI 协议客户端、不做多 provider 路由（那是 gateway 的职责）。
- benchmark 不算综合分、不接 webui 报表；DAL 是我的产物，不在此设计（架构评审另行）。

## Decisions

### N1 — 权限模型：三层 allowlist + 幂等拒绝（checklist C3，DH-2/CX-2 语义）

`PolicyEngine::evaluate(tool, args) -> Decision`，三个**可叠加**的层，先查先中：

1. **静态规则**（配置项，`[agent.policy]`）：`allow` / `deny` 名单（工具名 + 可选参数 glob）——非交互、启动即定。
2. **会话 allowlist**（运行时、精确签名）：`(tool, args)` 精确匹配 → 静默放行（`permission-flow` spec 的 allowlist 语义）。
3. **交互审批**（ask）：无静态命中、不在 allowlist → 需要回答者；**无回答者 → 拒绝**（fail-closed，DH-2）。

优先级：`deny` > `allow` > ask。拒绝幂等：`deny` 永远返回结构化拒绝，不进入 ask（防"deny 了又弹卡"循环）。

一次性升级（DH-2 "升级 = 带理由的重试"）在 `Decision::Escalate` 表达：升级带 `reason`，仅放行**那一次**；会话策略不变。与 DSH 的 "request carries agent/tool/callId/reason — deliberately no tool args" 对齐。

工具分类（决定默认策略；从轻到重）：

| 工具 | 默认策略 | 说明 |
|---|---|---|
| read / glob / grep | allow（只读，工作区内） | 不做审批 |
| write / edit | ask（**存在文件且要覆盖**时） | 新文件仍静默；read-before-write 规则保留在工具层 |
| bash | ask 且**默认拒绝写**（见 N2） | 只读 bash 静默（探测类命令不该弹卡） |
| web_search / web_fetch | deny（默认关）→ 配置/审批放行 | 本 change 的"危险面"之一 |

决策结果经 `permission-flow` 卡面返回（允许一次 / 本会话 / 拒绝）。升级形态 `Escalate{reason}` 在卡上呈现为带理由框的"允许这次"。

**Codex 对照（2026 源码）**：`ApprovalStore::with_cached_approval` 的 `ApprovedForSession`（按 exec command id / apply_patch 文件集缓存）正是"会话 allowlist"的 Codex 形态——我们保留 **args 精确签名**（比 Codex 的 id/文件集更细粒度，DSH 则刻意不带 args；三家中我们取"可精确吸收重复"这一档）。`AskForApproval::Never`/`OnRequest`/`RealUser` + `escalate_on_failure`（`SandboxOverride::BypassSandboxFirstAttempt`）与我们的 `deny > allow > ask` + 一次性升级同构；Codex 的**拒绝可由策略规则（`.rules`）持久化**是它的 heavier 形态，我们本期不做（保持会话内 allowlist 最小面）。

### N2 — bash 沙箱：Landlock 进程内为主 + 防火墙回退（CX-1 adapt，诚实降级）

**缺省 = Landlock（内核 ≥6.7 / ABI v4，含网络位）**。选型结论（2026-09-02 研究 + 本机实测）：纯 Rust 单 crate（`landlock` 0.4.7——内核 Landlock LSM 作者本人维护，MIT/Apache-2，MSRV 1.71）、无外部二进制、免 root、**默认 Docker 内可用**（moby 默认 seccomp 白名单放行 `landlock_*` syscall；而 bwrap 依赖的 userns 在默认 Docker 里被禁）：

```text
landlock（缺省；在 bash 子进程 pre_exec 内实施：fork → restrict → exec，killpg/超时语义不变）
  规则面（实测 ~45 行）：
    handle：fs from_all(V9)（BestEffort——旧内核尽力限制）
            net BindTcp + ConnectTcp（HardRequirement——内核不支持即 Err → 回退）
    只读：/（from_read）
    可写：workdir + /tmp（from_all 目录权限）；/dev/null、/dev/urandom
          （文件级 from_read|WriteFile——目录专属权限（MakeDir 等）不能授给文件 FD，实测踩过）
    网络：零 AccessNet 规则 = 拒绝所有 TCP bind/connect（fail closed）
  生效判定：LandlockStatus::Available + no_new_privs；RulesetStatus（Fully/Partially）
            **如实标注**、绝不断言 full——本机实测即报 PartiallyEnforced 且拒绝实际生效
  回退：任何 Err（内核 <6.7 / 受限容器）→ firewall，绝不半隔离
firewall（回退档；无 Landlock 能力的内核/环境）
  1. env 清洗：*_KEY/*_TOKEN/*_SECRET/*_PASSWORD 与 SEBAS_* 模式从命令环境剥离
  2. 危险二进制字面探测：spawn 前 command -v + readlink 比对（rm -rf /、mkfs、dd 到块设备…），
     命中即 Denied（工具层拒绝结果）
  3. 结果如实标注 [bash conf: firewall]
配置：bash_sandbox = "auto"（默认：Landlock 可用即用，否则 firewall）| "firewall"（强制回退档）
macOS（后续）：/usr/bin/sandbox-exec + deny-default .sbpl（Codex 同款），本 change 不做
```

实现落点：`bash` 工具持有 `SandboxBackend`（枚举 `Landlock` / `Firewall`），每次执行选一个后端（绝不 split 单命令跨后端），结果附 `[bash conf: <mode>]` 注释与 `ToolOutput` 可选字段；`landlock` crate 走 `[target.'cfg(target_os = "linux")'.dependencies]`。

**本机实测（2026-09-02，Arch 内核 7.1.10 / effective ABI v9，/tmp 探针 cargo 项目）**：工作区写 ✅ / `$HOME` 写拒（Permission denied）✅ / 全盘读（/etc/os-release）✅ / TCP connect 拒（Permission denied）✅；无沙箱对照运行同命令：HOME 写成功、connect 报 Connection refused（端口态）——证明拒绝来自 Landlock 本身。

已知边界（对齐 Claude Code Linux 默认，记录不隐瞒）：Landlock **只拒不藏**——`stat/ls` 仍可见 `~/.ssh` 等路径（"读 /"规则的代价，allow-only 无减法规则）；无 PID/IPC 隔离；UDP 不在 v4–v9 的 TCP 位内（DNS 泄露面，[推断]）。硬化路径：后续可加 **bwrap tier**（路径隐藏 + PID/IPC 隔离；本机已装 bwrap）——但需 userns，默认 Docker 内不可用，故不作缺省。

**Codex 对照（2026 源码）**：Codex Linux 默认 **bwrap + seccomp**（Landlock 为 `use_legacy_landlock` legacy 路径），macOS `sandbox-exec` + deny-by-default `.sbpl`，Windows RestrictedToken——它选"更强隔离 + 外部依赖/复杂参数面"；我们选"纯 Rust + 零外部依赖 + Docker 内可用"，用强度换简单——威胁模型上先封 coding agent 的两大实际风险（**TCP 外传**与**工作区外写**），与"简单实用"目标一致。另采纳 `is_likely_sandbox_denied`（exit 2/126/127 过滤 + SIGSYS + 关键字扫描）作为"疑似被沙箱拒绝"的**标注**（只标注不改判定）；沙箱拒绝不改写会话策略（一次一授权），与 Codex denial 非持久化一致。

### N3 — 网络工具：默认拒 + 门控 + 硬上限（DH-8 精神）

`web_search(query, max_results=8)` 与 `web_fetch(url, max_bytes=100k)`：

- **默认 deny**：网络能力在 `[agent.policy]` 配置为 `off`（默认）时，`evaluate` 直接返回 `Denied`，工具不联网。配置为 `ask` 时，首次调用走审查卡；`allow` 时静默。
- **校验**：URL 仅 `http`/`https` scheme（`url` crate）；`web_fetch` 重定向 ≤3 跳；`web_search` 无重定向（查询词不落网）。
- **上限**：search 条目 ≤8（截断标记）；fetch ≤100KB 正文（截断标记）；超时 30s。
- **robots.txt**：fetch 侧尽力遵守（读 `robots.txt` 的 `Disallow`，命中即返回 "robots.txt 拒绝" 结果；失败不阻塞——best-effort）。
- **结果形状**：与六件套一致的 `ToolOutput`（`ok` / `output` / `truncated` / `error`），错误是数据。
- 新依赖：`url`、`mime_guess`、`reqwest` 已存在。HTML 简化用最小 `scraper`/`html5ever`（本 change 允许——见 §6 风险）。

### N4 — 上下文管理第一步：工具结果改写 + Assembly 预算 + 只读并行（C8 token 维度）

1. **工具结果改写**（`message.rs` 常量）：工具返回的 `output` 入库前统一改写为 `首段 ~8k 字符 + "\n[truncated: 余下 N 字符省略，可用 read/offset 查看]"`——**结构化、确定性**，并写进工具 description（模型知道截断，需要更多就调用 read 分页）。这替换 1a "cap 后全量入库"的做法（入口防上了，上下文总量仍被大文件 read 撑爆）。`truncated` 语义保留在 `ToolOutput`；重写只影响**入库副本**，`ToolOutput` 本身不被改。
   - 注意：`ToolEnd` 事件的 `result` 文本仍发**改写前**上限内版本（事件面体验完整）；只有回填给模型的 tool_result 走改写。
2. **Assembly 预算**（`session/mod.rs`）：构造 `LlmRequest` 前——`history` 条数 > `max_messages`（默认 80）→ `Finished{reason:Budget{which:"messages"}}`（新 budget 维）；token 估算（`min` 式：chars × 0.25 + blocks 常数）> `est_token_budget`（默认上下文 40%）→ 同样干净收尾。请求体 `max_tokens` 继续 8192。
3. **只读并行**（`loop_/mod.rs`）：单响应内**连续只读段**（read / glob / grep / web_search / web_fetch）并行（按 `max_concurrent_readonly` 默认 8 分批 `join_all`），写工具串行且仅与相邻段保持先后（修订：原"全部只读先于全部写"会破坏 `[write → read]` 同响应依赖语义——实现期裁定为连续段并行，见 spec 同步修订）。事件序：`ToolStart` 按响应顺序发射；`ToolEnd` 按响应顺序发射；tool_result 按响应顺序回填。budget 的 `max_tool_calls` 计数含并行段内每个调用。

### N5 — webui 接线：进程内后端 + 审查卡 + 会话行选择（D4 展开）

一个 `sebas` binary crate 侧的 adapter，走既有 `SessionBackend` 缝（`sebas-webui/src/session_backend.rs:61`）：

```text
run --webui（进程内）
  sebas_webui::run*（持 Arc<dyn SessionBackend>）
    ▲
    ├─ acpBackend（现有 InProcessBackend，持 RouterHandle）        ← acp-claude 会话
    └─ NativeAgentBackend（新；持 SessionManager<AnthropicMessagesClient>）
          create_session(workdir) → SessionHandle(key)
          events(key) = session.subscribe() → 映射为 AgentEvent → SSE（既有 sse.rs）
          message(key,text) = handle.prompt(text)                  ← 串行队列
          close(key) = 取消 + 停 task
          focus/switch = SessionKey 已有语义
```

- **会话行下拉**：webui 会话创建表单加 `backend: acp|native` 选项；`spawn()` 据此选后端（进程内 `run` 两后端都持）。
- **审查卡**：`NativeAgentBackend.events()` 里 `AgentEvent::PermissionRequest{request_id, tool, args, reason}` → webui `/api/permissions` 呈现为卡片（允许一次 / 本会话 / 拒绝 + 理由框）→ `permission_decision(request_id, decision, reason)` 回 `SessionHandle`（新增 `answer_permission` 命令）。
- **词汇**：`PermissionRequest` 加进 `AgentEvent`（Phase 2 启用）；`ToolFinish{ok, truncated, exit_code}` 与 `SessionSummary{turn_ms, model_calls, tool_calls, output_chars}` 作为**辅助事件**（供 SSE/日志；1a 语义不变）。消费端渲染：webui 卡片新模板；飞书等其余面不变。
- **绑定**：webui 后端字段改为 `Vec<Box<dyn SessionBackend>>`（按 id 选择）或 `enum Backends{Acp, Native}`；`server.rs`/`routes.rs` 的会话路由把 `spawn`/`message`/`close`/`turns` 分发到选中的后端。

### N6 — 事件词汇与工具契约升级（agent-core spec）

- `AgentEvent` 新增：`PermissionRequest{session_id, request_id, tool_name, args, reason}`、`ToolFinish{session_id, tool_name, ok, truncated, exit_code}`、`SessionSummary{session_id, turn_ms, model_calls, tool_calls, output_chars}`。`PermissionRequest` 的 `request_id` 与工具 `tool_use_id` 一致（`permission-flow` 的关联契约）。
- `LlmClient` / 消息模型升级：`ContentBlock::Image{media_type, data}`（多模态；`strip_thinking` 兼容处理）；`LlmConsult` 常量（tools 数量上限 128、上下文窗口逼近 90% → finish）。
- 后端选择与伪工具：`bash conf` 以独立伪工具（`session_tools`）事件通报，避免污染 bash 结果。
- 新工具 schema 注册：web_search / web_fetch / read_image / lsp 在 `ToolRegistry` 条件注册（read_image 与 lsp 只在能力门开放时出现在 `schemas()`）。

### N7 — agent-bench：CLI + 轨迹 + dashboard（评估面）

```text
sebas agent-bench [--smoke] [--tasks a,b,c] [--model m] [--record trace.jsonl]
  └─ run_task(): 临时工作区 → 会话 → 收满事件 → 断言（文件内容匹配 / 轨迹包含物）
  └─ 输出：per-task 结果 + 树状 dashboard（桶分组：web-tooling / apply_patch / subagent——apply_patch 与 subagent 桶本期为「占位、标记 skipped」）
  └─ --smoke：固定小子集（1 个自愈 + 1 个静态处理）
```

- 分桶打分：每任务 `Result{passed, score, budget_flags}`；smoke 只跑固定子集。
- 轨迹 JSONL：`# task: <id>` 头 + 事件流（复用 agent-dev 的 `Recorder` 逻辑，演进为共享模块）。
- **不算综合分、不接 webui 报表**；DAL's dashboard 是我的独立产物，归架构评审另议。
- **Codex 对照（2026 源码）**：`rollout-trace` crate（`CODEX_ROLLOUT_TRACE_ROOT`，opt-in）与我们同构——"原始 `trace.jsonl` + `payloads/*.json`，经 `codex debug trace-reduce` **确定性规约**成语义图（ConversationItem / ToolCall / InferenceCall / Compaction / InteractionEdge）"，是"observe first, interpret later"的既有先例。我们本期只做**原始轨迹 + 断言**（不建规约子命令），但 schema 预留 `event graph` 演进位；`codex exec` 的 `--json`/`--output-schema`/resume/fork、`codex-rs/state/` 的 SQLite 状态库、`thread-store` 的规范化 `TurnItem` 持久化（**不再是扁平 transcript.jsonl**）都指向**持久化是下一阶段的事**——本 change 只做"内存会话 + 轨迹文件"，落地顺序与 Codex 验证一致（先 trace 后 store）。
- 失败自愈固定为任务集第 1 个（`ERROR-RECOVERY`）。
- 重放：`--replay trace.jsonl` 用 fake LLM 复现事件序列（断言相等）；`--debug` 逐工具打印（复用 `Recorder`/事件打印）。

### N8 — 蓝图对照修订（DSH 与 Codex 均开源后）

`docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md` §3/§4/§11 修订：

- **§3 Codex 拆解**：证据列升级为源码（`openai/codex`，2026-09 快照）——CX-1 修正（Linux 沙箱默认 **bubblewrap + seccomp**，Landlock 为 legacy fallback；macOS sandbox-exec；Windows RestrictedToken）；CX-3 修正（"单工具收敛路线"**已被 2026 架构取代**——unified_exec / apply_patch / update_plan + MCP + 多 agent v2 + code_mode 工具面）；新增采纳行——会话批准缓存 `ApprovedForSession`（≡ 会话 allowlist）、`is_likely_sandbox_denied` 拒绝启发式、`rollout-trace` 原始轨迹 + 确定性规约、`codex exec --json/resume/fork` 无头事件（→ 我们的 benchmark 面）。
- **§4 DSH 拆解**：证据列 `S9 [观测]` 升级为 `S10 [源码] deepseek-ai/deepseek-harness（2026-08-13 开源，MIT）` + 官方 docs 引用；机制行按源码重写/修正（如 DSH 有 `exit_plan_mode`、`workflow`、`lsp`、`session_search` 等新工具；跳转策略：对无条件冲突、可验证的机制修正裁决）。
- **§11 路线图**：**Phase 2 = 权限/沙箱 + 网络面**（本 change 范围，入口标准 = 1a 稳定）；Phase 3 拆 **3a 上下文 compaction / 摘要**、**3b 任务清单 + agent 主动提问**、**3c plan mode + apply_patch**，各自独立 change；Phase 4 subagent / MCP / skill / registry 中式设计共享。持久化（OQ1）升为路线图显式条目——Codex thread-store/SQLite 与 DSH session-log 都证明持久化是下一阶段的地基（不属本 change）。
- 新增 §12 修订台账：本 change 对 §3/§4/§11 的裁决变更逐条记录（供审计）。

## Naming conventions（沿用 1a/仓库）

- crate 名 `sebas-agent`（不变）；能力域名 `agent-core`（spec path 不变）；本 change 新增 `agent-bench`（spec path）。
- 类名/模块：`policy::PolicyEngine`、`PolicyDecision`、`SandboxBackend`、`web::WebSearchTool`/`WebFetchTool`、`image::ReadImageTool`、`lsp::LspTool`、`bench::run`。
- 事件变体沿用 `snake_case` serde tag；`request_id` 与 `tool_use_id` 对齐。

## Risks / Trade-offs

- [PolicyEngine 弹卡太频繁拖慢循环] → 只对破坏面 ask（写覆盖 + 写 bash + 网络）；只读与"只读 bash"静默；allowlist 吸收重复（Codex `ApprovedForSession` 同构——按 args 精确签名，比 Codex 的 id/文件集更细粒度）
- [Landlock 在旧内核（<6.7）或受限容器不可用] → 自动回退 firewall 并如实标注（`RulesetStatus` 部分生效也如实上报，实测已验证该路径必要）；bwrap 硬化 tier 留作后续（需 userns，默认 Docker 内不可用）
- [web 工具质量（HTML 解析）不稳] → 上限硬性 + robots best-effort；`scraper`/`html5ever` 新依赖经评审（1a 零外部依赖原则让位给"最小有用网络面"）
- [并发执行改变 1a 事件序假设] → ToolStart/ToolEnd/回填仍严格按响应顺序；只读并行只在组内、写串行——1a 顺序断言（如 `full_turn_event_sequence`）不破
- [AgentEvent 增 variant 影响既有消费端] → 只增不改（serde 兼容）；`PermissionRequest` 的消费端（webui）本 change 同步交付；飞书等其余面按"未知事件忽略"语义不受影响（既有 SSE 已容忍）
- [上下文改写后模型信息缺失] → 改写结构化 + 工具 description 明示 + `truncated` 标记保留；benchmark 覆盖"大文件处理靠 read 分页"用例
- [webui 双后端并行推进与 channel 分支冲突] → 进程内 `NativeAgentBackend` 只依赖已合入的缝（trait + InProcessBackend 形状）；channel 的 socket 部分与本 change 并行、合并面仅 `session_backend.rs`（若两分支都改，冲突面小）
- [拒绝解析（is_likely_sandbox_denied 启发式）误判] → 只做**标注**不改结果语义（拒绝仍走结构化决策，不因启发式改判定）；启发式失败的回退是"不标注"，不改变安全性

## Migration Plan

- 纯新增模块 + 事件新变体 + 新工具（默认拒/条件注册）+ webui 新后端（可选下拉）。不破坏 1a 行为：默认策略下——只读静默、新文件 write 静默、会话内已 read 后 write 静默；bash 默认 `firewall`（比 1a 更安全，非更宽松）。`PermissionRequest` 只在 ask 触发时发出；`agent-dev` example 在无回答者时按 fail-closed 拒绝（脚本可设 `--answer allow-once` 模拟）。
- 回滚：webui 后端下拉默认 `acp`；`[agent.policy]` 全部 `allow` → 退化为 1a 行为（除 bash 沙箱不可关——auto：Landlock→firewall，这是安全基线）。

## Open Questions

- **OQ7**：web 工具面浏览器内核（headless chromium）与纯 HTTP（web_fetch）二选一？——设计倾向纯 HTTP + robots best-effort；评审裁决。
- **OQ8**：`max_messages` 触顶后的 UX——直接 `Finished`（简单） vs 自动对旧消息做摘要（Phase 3a compaction）？——设计倾向前者（本 change 的预算语义），摘要留给 3a。
- **OQ9**：`agent-bench` 的宿主——`sebas` binary 子命令（proposal 倾向） vs sebas-agent 自带 example/bin？——实现 change 裁决；proposal 按 binary 子命令写。
- **OQ10**：apply_patch / subagent 桶在 agent-bench 中的处理——本期占位 skkiped vs 彻底不列出？——设计倾向占位（dashboard 可见 roadmap）；benchmark spec 已写占位语义。
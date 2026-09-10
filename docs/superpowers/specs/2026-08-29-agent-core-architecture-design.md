# agent-core — sebas 原生 Coding Agent 架构调研与设计

> 日期：2026-08-29
> 状态：调研 + 设计蓝图（纯文档，未实现；对应 openspec change `add-agent-core-architecture`，`skip_specs`——行为规格将由后续实现 change 从本文派生）
> 作者：DeepSeek Harness（与 cupen 协作）
> 修订 2026-09-01：crate 定名 **sebas-agent**（沿 sebas-* crate 惯例，"agent-core" 保留为能力域名）；D3 修订为 **gateway 可选**——agent 可直连 provider（见 §6 / §7.2 / §10）。
> 修订 2026-09-02：DSH 与 Codex 均已开源，§3/§4 证据升级为源码对照（S10/S11），§3 两条裁决修正（CX-1/CX-3）、§11 路线图拆分细化、新增 §12 修订台账（openspec change `sebas-agent-next`）。

## 0. 摘要

sebas 目前只是 agent 的"桥"：`acp-claude` 拉起外部 Claude Code 子进程，agent loop、工具契约、上下文管理都在别人进程里。本文回答两个问题：**专业 coding agent 由什么构成**（Part I，拆解 Claude Code / Codex / DeepSeek-Harness）——答案是七条共性不变量 + 九条可验收 checklist（§5.3）；以及 **sebas 自己的 agent 内核长什么样**（Part II）——新 crate `sebas-agent`（能力域名 agent-core）：in-process 事件驱动循环（§7）+ 六件套工具（§8）+ Anthropic 协议 LLM 通道（D3，直连 provider 或经可选 gateway）+ webui 优先的会话面（§9），与 `acp-claude` 并存不替换（§10），按四阶段演进（§11）。Phase 1 的验收基准就是 §5.3 的 checklist。

---

# Part I — 专业 coding agent 机制拆解

## 1. 研究方法

三个参考对象：**Claude Code**（终端优先、工具最全的标杆）、**OpenAI Codex CLI**（OS 级沙箱路线、同为 Rust 实现）、**DeepSeek-Harness**（策略级文件沙箱 + 显式审批升级 + 目标/子代理编排）。

统一拆解格式——**机制拆解表**，每行一个机制：

| 列 | 含义 |
|---|---|
| 机制 | 机制名称与编号（CC-x / CX-x / DH-x，供全文引用） |
| 它如何工作 | 机制的行为描述，不含营销词 |
| 证据 | [文档] 公开文档引用编号（附录 A）；[观测] 可观测行为；[推断] 明确标注的推测 |
| 迁移判定 | **adopt**（照搬）/ **adapt**（改造后用）/ **skip**（不用）+ 一句话理由，落点是 sebas 现状 |

sebas 现状落点（判定依据）：Rust workspace（`src/` 编排 + `router` / `feishu` / `acp-claude` / `gateway` / `webui` 五 crate）；`acp-claude` 已定义流式事件词汇 `AcpEvent`（TextDelta / ThinkingDelta / ToolStart / ToolProgress / ToolEnd / PermissionRequest / Finished / Error）；`gateway` 双协议纯透传且自身流量走 Anthropic 协议；`webui` 已有 SSE 消费与控制台地基；`permission-flow` 已有交互卡片（允许一次 / 本会话 / 拒绝）。

## 2. Claude Code 拆解

#### 2.1 机制表

| # | 机制 | 它如何工作 | 证据 | 迁移判定 |
|---|------|-----------|------|---------|
| CC-1 | 单主循环（agent loop） | 模型调用 → 解析响应中的 `tool_use` 块 → 本地执行工具 → 以 `tool_result` 消息回填 → 再次调用模型；循环直至响应不含工具调用、或用户中断（Esc）、或上下文触顶触发 compaction。没有隐藏的第二循环——所有"自主性"都来自这个循环的重复。 | [文档 S1][观测] | **adopt** — 即 D1 的 in-process loop。sebas 已有 `AcpEvent` 流式词汇（TextDelta/ThinkingDelta/ToolStart/ToolEnd…），可直接承载该循环的事件面 |
| CC-2 | 专用工具集 | 数十个各司其职的工具：Bash、Read、Write、Edit（精确字符串替换 + `replace_all`）、Glob、Grep、WebFetch/WebSearch、TodoWrite/TaskCreate（任务跟踪）、Task/Agent（子代理）、Skill、AskUserQuestion、ExitPlanMode 等；每个工具有独立参数契约与"何时用我"的说明 | [文档 S1] | **adapt** — 首批只取 bash / read / write / edit / glob / grep 六件套（D2）；WebFetch、Todo、Skill 等进路线图不进 Phase 1 |
| CC-3 | 权限模式 + 审批卡 | 权限四态：default / acceptEdits / plan / bypassPermissions；settings.json 配 allow/deny/ask 规则；未命中白名单的危险工具调用弹交互审批，选项形如"允许一次 / 总是允许 / 拒绝" | [文档 S3] | **adapt** — sebas `permission-flow` 卡片（允许一次 / 本会话 / 拒绝）语义同构，Phase 2 只需把规则引擎接到工具调用前 |
| CC-4 | Hooks | PreToolUse / PostToolUse / SessionStart 等生命周期钩子执行 shell 命令，可阻止或改写工具调用（如 `setMode` 动态改权限模式） | [文档 S2] | **skip**（本期）→ Phase 4+ 路线图。sebas 的等价物是 router 状态机 + 卡片按钮；钩子机制不是地基必需品 |
| CC-5 | 子代理（Task/Agent tool） | 派生拥有独立上下文窗口的子代理执行搜索/大任务，仅把结果摘要回传主代理，保护主上下文不被工具噪音灌满 | [文档 S1][推断：上下文隔离动机为公开访谈所述，内部实现未公开] | **skip**（本期）→ Phase 4。方向正确，Phase 1 单循环足够 |
| CC-6 | Plan mode | 只读探索模式：模型只能用只读工具做调研，`ExitPlanMode` 把计划呈给用户批准后才切回执行模式 | [文档 S1] | **adapt**（Phase 3/4）— 与 webui workbench"等待操作者"状态天然对应；Phase 1 不做 |
| CC-7 | 项目记忆（CLAUDE.md） | 项目根的 CLAUDE.md 自动注入每次会话上下文，承载项目约定/命令/风格；`@路径` 引用其他文件 | [文档 S1][观测] | **adopt** — agent-core 启动会话时读取项目 AGENTS.md / CLAUDE.md 注入 system。成本极低、收益极大（sebas 仓库自己就维护着 AGENTS.md） |
| CC-8 | 流式呈现 | 思考 / 文本 / 工具调用全部以增量事件流式到界面（终端 UI 或卡片），工具调用与输出在界面内联可见，而非等最终结果 | [观测] | **adopt** — sebas 卡片流式（`AcpEvent` 增量事件 → 飞书/webui）已是同一模型，对齐词汇即可 |

#### 2.2 迁移小结

Claude Code 证明的是**"专用工具 + 结构化契约"路线**（与 Codex 的单工具路线形成对照，见 §3）：六工具集、权限模式 + 审批卡、CLAUDE.md 项目记忆、全程流式——这四件事构成 sebas Phase 1/2 的直接蓝本；hooks、子代理、plan mode 是正确但不紧急的方向，进路线图（§11）不进本期设计。关键的好消息是：sebas 现有资产（`AcpEvent` 事件词汇、`permission-flow` 交互卡片、webui SSE 地基）与这条路线**天然同构**，迁移成本低——Phase 1 的事件面不需要发明任何新词汇。

## 3. Codex（OpenAI）拆解

> **2026-09-02 源码对照修订（S11，`openai/codex`，2026-09 快照）**：本节基于第三方文档的裁决按源码修正——
> **CX-1 修正**：Linux 沙箱默认已是 **bubblewrap + seccomp**（ro-bind 根、可写层、unshare user/pid/net、seccomp 网络过滤 + 代理桥；`codex-rs/linux-sandbox/`、`codex-rs/sandboxing/src/bwrap.rs`），Landlock 降级为 `use_legacy_landlock` legacy 路径；macOS `sandbox-exec` deny-by-default `.sbpl`；Windows RestrictedToken。拒绝检测用启发式 `is_likely_sandbox_denied`（exit 2/126/127 + SIGSYS + 关键字扫描）。
> **CX-3 修正**：「Codex 是单工具收敛路线」**已被 2026 架构取代**——工具面现为 unified_exec（带进程管理）/ apply_patch / update_plan / view_image / get_context_remaining / request_permissions / request_user_input / tool_search + MCP + 多 agent v2（spawn/send_message/interrupt/wait）+ Code Mode（`codex-rs/core/src/tools/`）。原裁决"skip（路线）"的**结论仍成立**（sebas 走专用工具路线），但前提"收敛"不成立——专用 vs 收敛的对照应改为"面可控性"而非"面大小"。
> **新增采纳行**：会话批准缓存 `ApprovalStore::with_cached_approval`（ApprovedForSession，按 exec id / apply_patch 文件集——≡ sebas 会话 allowlist）；审批 `AskForApproval{unless-trusted, on-request(默认), never, granular}` + `escalate_on_failure` 一次性未沙箱升级（≡ 一次性升级重试）；`rollout-trace` crate（opt-in 原始 trace.jsonl + 确定性 trace-reduce 语义图——sebas agent-bench 的轨迹/重放即其最小形态）；`codex exec` 无头 ThreadEvent JSONL + resume/fork。
> **持久化佐证**：thread-store 规范化 TurnItems + SQLite 状态库（`codex-rs/state/`）取代扁平 transcript.jsonl——sebas 的会话持久化（OQ1）照此形态列 Phase 3+/4。

#### 3.1 机制表

| # | 机制 | 它如何工作 | 证据 | 迁移判定 |
|---|------|-----------|------|---------|
| CX-1 | OS 级沙箱（三档） | `read-only` / `workspace-write` / `danger-full-access` 三档执行环境；macOS 用 Seatbelt、Linux 用 Landlock + seccomp 做内核级文件/网络约束；workspace-write 下工作区可写、网络默认禁用 | [文档 S4][文档 S6] | **adapt**（Phase 2）— sebas Phase 1 用策略级门控（工具执行前检查，见 §8），OS 级隔离（Linux namespace/seccomp 包裹 bash 子进程）作为 Phase 2 加固项；Codex 同为 Rust 实现证明该路线在 Rust 生态可行 |
| CX-2 | 审批策略（四档） | `untrusted` / `on-request` / `on-failure` / `never`：被沙箱拦截的操作按策略升级为用户审批——`on-failure` 即"先在沙箱里跑，被拦了再问人" | [文档 S4][文档 S6] | **adopt** — 四档枚举 + "失败后升级"形态与 DeepSeek-Harness 的显式升级（DH-2）同构，Phase 2 审批策略直接参考；`on-failure` 尤其适合 webui 异步交互节奏 |
| CX-3 | 工具面（单 shell 工具路线） | 与 Claude Code 相反：核心只暴露一个 shell 执行工具 + `apply_patch`（结构化补丁）+ `update_plan`（计划跟踪），工具面刻意收敛 | [文档 S5][推断：工具面收敛为公开系统提示词所示，进程内实现未公开] | **skip（路线）** — 证明"单工具也能跑通"，但 sebas 已裁定走专用工具路线（D2）：更好的模型 affordance、更细的权限挂点、更可读的 transcript。`update_plan` 值得 Phase 3 单独借鉴 |
| CX-4 | apply_patch | 自定义补丁格式（`*** Begin Patch` / `Update File` / `*** End Patch`）做批量文件修改：diff 语义清晰、可整体审阅、可整体拒绝 | [文档 S5] | **adapt**（Phase 3+）— Phase 1 的 edit（精确字符串替换）覆盖日常编辑；补丁格式留给"多文件批量重构"场景 |
| CX-5 | AGENTS.md | 项目指令文件：会话启动时发现并注入上下文——CLAUDE.md 的跨厂商中立版本（Codex、DeepSeek-Harness 等均识别） | [文档 S6][观测：sebas 仓库自身就维护着 AGENTS.md] | **adopt** — 与 CC-7 同一机制，agent-core 优先读 AGENTS.md（更中立），CLAUDE.md 作兼容回退 |
| CX-6 | Rust 实现 | Codex CLI 核心为 Rust（openai/codex 开源仓库），沙箱、进程管理、协议面均在 Rust 侧成熟落地 | [文档 S4][推断：仓库语言构成为公开可查] | **note** — 非机制但消除可行性疑虑：sebas 用 Rust 做 agent-core + 沙箱有直接先例 |

#### 3.2 迁移小结

Codex 的价值在**约束面**而非工具面：沙箱三档 + 审批四档给出了 Phase 2 的现成词汇表（枚举几乎可以照抄），`on-failure`"先跑、被拦再问"与 webui 的异步审批卡片天然契合；单工具路线被 D2 明确否决，但 `update_plan` / `apply_patch` 是 Phase 3 的两个具体借鉴点；同为 Rust 实现则消除了"Rust 做进程沙箱是否可行"的疑虑（CX-6）。

## 4. DeepSeek-Harness 拆解

> **2026-09-02 源码升级（S10）**：DSH 于 2026-08-13 开源（`deepseek-ai/deepseek-harness`，MIT，~208k stars；npm `@deepseek-ai/dsh`）。S9 [观测] 的机制行升级为源码可证：`docs/tool-catalog.md`（工具清单含 exit_plan_mode / workflow / lsp / session_search / run_code / ralph / schedule_* 等）；`docs/subsystems/sandbox.md`（SandboxMode read-only / workspace-write / danger-full-access；bwrap/Landlock/Seatbelt/ACL 后端 + full/partial 如实上报）；`docs/subsystems/approval.md`（ApprovalPolicy ask/never；fail-closed 五态闭集；**请求带 agent/tool/callId/reason 而刻意不带 args**）；`docs/agent-lifecycle.md`（goal 轮次 + blocked floor 3 轮；send_message 步界注入）；**执行流水线**：`tools/pre-execute` 瀑布（hooks→permission→sandbox）→ 单调 guard → ctx.approval（缺席即拒）→ `tools/execute` → fs 写意图门 → `tools/post-execute`（S7 的五段流水线由此印证）。本节机制表保留作为首次观测记录，行级差异以源码为准。

#### 4.1 机制表

| # | 机制 | 它如何工作 | 证据 | 迁移判定 |
|---|------|-----------|------|---------|
| DH-1 | 策略级文件沙箱 | workspace-write 模式下：写操作限定在会话工作区 + 平台临时目录，读面更宽；违规**不炸进程**，而是返回结构化拒绝标记（形如 `[sandbox: file access denied under <mode> mode]`——明确告知这是策略拒绝而非故障），并提示可在更宽模式下重试 | [观测 S9][文档 S7] | **adopt** — "拒绝是数据、不是崩溃"是 agent 长跑稳定性的关键性质（checklist C4）；sebas 的 ToolResult 错误语义照此设计（§8.1）。OS 级隔离不跟——那是 Codex 路线（CX-1），Phase 2 再议 |
| DH-2 | 显式审批升级 | 审批政策 ask：操作可经配置的回答者向用户提问；**无回答者则 fail closed**；升级形态是"同一操作带一句话理由做一次性重试"，用户批准仅放行这一次 | [观测 S9] | **adopt** — "升级 = 带理由的重试"非常干净：权限请求自带上下文，`permission-flow` 审批卡正好承载；Phase 2 采用此形态 |
| DH-3 | 结构化提问工具 | ask_user_question：稳定 id 的问题 + 预设选项 + 推荐项置顶标注；答案结构化回传，agent 据此继续——歧义在开工前消解而非开工后返工 | [观测 S9] | **adapt**（Phase 3/4）— agent 主动提问的界面即 webui 表单 / 飞书卡片（`feishu` crate 已有表单能力）；Phase 1 不做 |
| DH-4 | 目标与长任务编排（goal） | create_goal / get_goal / update_goal：目标含 id、revision、objective、轮次上限；完成/阻塞/暂停/恢复是显式动作；声明 blocked 需同一条件连续多轮（防过早放弃）；revision 做乐观并发控制 | [观测 S9] | **adapt**（Phase 3/4，路线图深度）— 长任务的"目标-轮次-阻塞"模型值得借鉴，本期只记录不设计（Non-goals 约束） |
| DH-5 | 后台作业与子代理 | 后台 job：id 化、job_output 拉取、job_kill 终止、完成主动通知；subagent（自包含提示词、默认后台运行）与 subagent_fork（继承完整对话的子代理）两种形态；interrupt / send_message 追加指令或中断 | [观测 S9][文档 S8] | **skip**（本期）→ Phase 4。值得记录的洞察：**自包含提示词 vs 继承上下文是子代理的两种基本形态**，各有适用（前者做独立调研，后者做上下文延续） |
| DH-6 | 技能系统（skills） | SKILL.md 指令包按确切名称按需加载；用户可直接触发；未加载前不得臆测其内容——技能是"可复用的任务指令集"，不是代码插件 | [观测 S9][文档 S7] | **adapt**（Phase 4/5）— sebas 可把技能做成"提示词包 + 触发命令"，与 router 的 slash 命令体系天然契合；本期不设计 |
| DH-7 | 任务清单工具 | todo_write：全量替换式清单（pending / in_progress / completed），"开工前加条目、完成即标记"的纪律；仓库级持久任务外置到 bd（beads）而非塞进会话 | [观测 S9][推断：beads 为仓库集成组件，非 harness 内核] | **note** — 会话内任务清单是 Phase 3 的廉价加分项（对应 CX-3 `update_plan`）；不发明新语义，直接抄三态模型 |
| DH-8 | 观测工具契约 | read 返回带行号文本（offset / limit 续读）；glob 只返回文件、按修改时间排序、上限 100 条且注明截断与完整列表落盘位置；grep 用 ripgrep 语法、行号分组、上限 250 条；read_image 直读图片；指南明确"优先用结构化工具而非 shell" | [观测 S9] | **adopt** — 直接喂给 §8.2 工具契约：read 的 offset/limit、glob 的 mtime 序 + 上限 + 续读提示、grep 的 include 过滤 + 上限——三个参考对象中最具体、最可照抄的一手资料 |

#### 4.2 迁移小结

DeepSeek-Harness 的独有价值是**策略沙箱 + 显式升级**这对组合（DH-1 / DH-2）：它证明不用 OS 沙箱、纯策略层也能把安全性做扎实——拒绝是结构化数据、升级是带理由的一次性重试、无回答者时 fail closed。这三条加上 DH-8 的工具契约细节，是 sebas §7–§8 最直接的输入。编排层（goal / job / subagent，DH-4 / DH-5）验证了 Phase 4 的方向，但也确认了本期不做的判断：它们依赖核心循环先稳定。

## 5. 跨 agent 综合

### 5.1 共性不变量

三家在最本质的层面上**没有分歧**——这些就是"专业 coding agent"的不可裁剪项：

1. **规范循环**：模型 → 工具调用 → 结果回填 → 再调模型。三家皆此，无一例外（CC-1、CX-3 明确描述；DSH 循环为 S9 一手观测）。所谓 agent 自主性，全部来自这个循环的重复，不存在隐藏的第二引擎。
2. **结构化工具契约**：无论工具面宽（CC-2 数十个）还是窄（CX-3 一个 shell），每个工具都有明确的参数/结果语义——契约是模型正确用工具的前提（CC-2、CX-4、DH-8）。
3. **危险操作门控 + 显式升级路径**：默认受限，越界要么被拦（CX-1、DH-1）要么先问（CC-3、CX-2、DH-2），且升级路径用户可见、结果可预期。
4. **项目记忆文件自动注入**：CLAUDE.md / AGENTS.md 在会话启动时进入上下文（CC-7、CX-5、S9 观测）。
5. **全程流式可观测**：文本、思考、工具调用、结果逐事件呈现给人（CC-8、DH-8、S6 所述 Codex 行为流）。
6. **错误是数据**：工具失败、沙箱拒绝以结构化结果回给模型自行纠正，而不是终止会话（CX-2 的 on-failure、DH-1 的拒绝标记）。
7. **任务/计划跟踪面**：会话内有可见的计划/清单状态（CC-2 的 TodoWrite、CX-3 的 update_plan、DH-7 的 todo）。

### 5.2 有意的分歧

分歧处是各家**价值观差异**，也是 sebas 必须自己选边的地方：

| 分歧点 | 路线 A | 路线 B | sebas 的选择 |
|---|---|---|---|
| 工具面宽度 | Claude Code：数十个专用工具（CC-2） | Codex：单 shell 工具（CX-3） | 取中：六件套（D2）——专用契约的 affordance + 可控的面 |
| 约束机制 | Codex：OS 内核沙箱（CX-1） | DSH：策略级文件沙箱（DH-1） | 策略门控起步（Phase 1），审批卡承载（现有 `permission-flow`），OS 沙箱 Phase 2 加固 |
| 编排半径 | DSH / Claude Code：子代理、目标编排（CC-5、DH-4、DH-5） | Codex CLI：单会话聚焦 | Phase 1 单会话；编排进 Phase 4（依赖核心循环先稳） |
| 提问方向 | CC / DSH：agent 可主动向用户结构化提问（CC-2 的 AskUserQuestion、DH-3） | Codex：以审批为主，无主动提问工具 | Phase 3+ 引入（webui 表单 / 飞书卡片）；Phase 1 只有人→agent 单向 |

### 5.3 "专业 coding agent"的操作性定义（checklist）

综合三份拆解，一个 agent 配得上"专业"二字，当且仅当满足以下九条（每条回溯拆解行）：

- [ ] **C1 多步自主**：一次提示驱动 N 轮"模型-工具"循环直至任务完成，无需人逐步驱动（CC-1、CX-3、DH-4）
- [ ] **C2 全程流式可观测**：文本 / 思考 / 工具调用 / 结果逐事件到达界面（CC-8、DH-8）
- [ ] **C3 危险操作门控**：默认受限 + 显式升级 + 无应答时 fail closed（CC-3、CX-2、DH-1、DH-2）
- [ ] **C4 错误是数据**：工具失败、沙箱拒绝以结构化结果返回模型自愈，不炸会话（CX-2、DH-1）
- [ ] **C5 结构化工具契约**：每工具声明参数 schema、结果与错误语义（CC-2、DH-8）
- [ ] **C6 项目记忆**：AGENTS.md / CLAUDE.md 启动时自动注入（CC-7、CX-5）
- [ ] **C7 可中断**：用户随时打断，进行中的工具被安全终止，会话状态保留（CC-1、DH-5）
- [ ] **C8 预算止境**：轮次 / token 预算是一等停止条件，agent 不会无限烧钱（CC-1 的 compaction 触发、DH-4 的轮次上限）
- [ ] **C9 任务跟踪**：会话内计划/清单可见、可勾选（CC-2、CX-3、DH-7）

**这九条就是 Part II 的验收基准**：§7–§8 的设计逐条对应，Phase 1 至少达成 C1/C2/C4/C5/C6/C7/C8（C3 的规则引擎与 C9 完整版在 Phase 2/3）。

---

# Part II — sebas agent-core 目标架构

## 6. 决策总览（对应 change design D1–D7 的展开）

| 决策 | 一句话 | 展开位置 | Phase |
|---|---|---|---|
| D1 | in-process 事件驱动循环，不做第二个子进程桥 | §7 | 1 |
| D2 | 统一 Tool trait + JSON Schema 参数；首批六件套 | §8 | 1 |
| D3 | Anthropic Messages 协议优先；端点可配置——**直连 provider（默认）或经可选 gateway**（2026-09-01 修订：gateway 非必经层） | §7.2 / §10 | 1 |
| D4 | webui 优先，经 SessionBackend 形状的会话 API + SSE | §9 | 1 |
| D5 | 调研方法：机制拆解表 + adopt/adapt/skip 判定 | Part I 全部 | —（已完成） |
| D6 | 单文档 ADR 风格（本文） | —（已体现） | — |
| D7 | 与 `acp-claude` 并存，替换为零 | §10 | 1 |

对照 §5.3 checklist 的达成承诺：Phase 1 达成 C1/C2/C4/C5/C6/C7/C8；C3 的规则引擎在 Phase 2；C9 完整版在 Phase 3。

## 7. Agent Loop 设计（D1 展开）

### 7.1 状态机

每个会话一台小状态机（sebas 会话状态机的既有词汇沿用 `MappingState` / `AcpEvent::Finished` 语义）：

```
            prompt                 stop_reason=tool_use            无 tool_use / 预算尽
 Idle ───────────────► AwaitingModel ────────────► ExecutingTools ──────────────► Finished
   ▲                        │  ▲    SSE 增量          │  工具结果回填                    ▲
   │        cancel          │  └──────────────────────┘                                 │
   ├────────────────────────┤            cancel（kill 工具进程/标记取消）                │
   │                        ▼                                                           │
 Cancelled ◄────────────────┘            不可恢复错误（进程死/协议崩）                   │
                                            └──────────────► Failed{terminal} ────────┘
```

- **Idle**：等待 prompt（会话历史保留）；cancel 为 no-op。
- **AwaitingModel**：已向 gateway 发起流式请求，正在消费 SSE 增量。
- **ExecutingTools**：正在执行本轮响应中的 `tool_use` 块（Phase 1 串行执行）。
- **终态**：`Finished`（正常结束或预算耗尽）、`Cancelled`（用户打断）、`Failed{terminal}`（provider 不可达、协议层崩溃等不可恢复错误）。

### 7.2 一轮 turn 的数据流

1. webui 提交 prompt → 会话离开 Idle。
2. 构造 messages：**system = agent-core 基础提示词 + 项目 AGENTS.md / CLAUDE.md**（C6，机制同 CC-7/CX-5）+ 历史消息 + 本轮用户输入。
3. HTTP POST `{llm_endpoint}/v1/messages`（Anthropic 协议，`stream=true`）。**端点可配置（D3 修订，2026-09-01）**：默认直连 provider——base_url 与 api key 直接来自 provider 数据（Anthropic API 或任意 Anthropic 兼容上游），不需要跑 gateway；也可指向本地 gateway（`WireProtocol::Anthropic` 面 + `auth_token`，见 `gateway/src/proto.rs`）以换取多 provider 模型名路由与用量计量。两种端点对循环是同一个 wire protocol，区别只是端点与凭证配置。仍拒绝内嵌任何 provider SDK（路由/用量逻辑属于 gateway 或宿主，不属于内核）。
4. 消费 SSE 增量（`gateway/src/sse.rs` 已有 SSE 处理先例）：`content_block_delta` 按 block 类型分流——text → `TextDelta`、thinking → `ThinkingDelta`——边收边发事件（C2）。
5. `stop_reason = tool_use` → 进入 ExecutingTools：按序执行每个 `tool_use` 块，逐个发 `ToolStart{tool_name, args}` → （可选 `ToolProgress`）→ `ToolEnd{result}`。
6. 全部工具结果以 `tool_result` 块回填为下一条 user 消息 → 回到 AwaitingModel。
7. `stop_reason = end_turn` → `Finished`。**循环的停境**（C1/C8）：无工具调用、或预算触发（7.3）。

### 7.3 取消安全与预算

- **取消（C7）**：loop 用 `tokio::select!` 同时监听 cancel 信号与当前步骤（对齐 `acp-claude` 的 `AcpCommand` 会话命令模式）。取消语义：进行中的 bash 子进程被 kill；非进程型工具（read/edit 等）天然原子，直接丢弃；已产生的增量事件照常保留（不回滚 UI）；状态回 **Idle**、历史完整保留——取消是"打断这一轮"，不是"销毁会话"。
- **预算（C8）**：每 turn 三重上限——`max_model_calls`（模型调用次数）、`max_tool_calls`、`wall-clock deadline`。触发即 `Finished{reason: budget_exhausted}` 并附摘要事件，把"我停在这里"告诉用户。这是 DH-4 轮次上限精神的 Phase 1 子集（goal 编排本身留 Phase 4）。
- **失败分级**：gateway 不可达 / 4xx → `Error{terminal:false}`，可重试；会话进程级死亡 → `Error{terminal:true}`——语义完全沿用 `AcpEvent::Error` 的现有约定（acp-claude/src/session.rs 中 `terminal` 注释：router 据此移除映射并显示 ❌）。

### 7.4 事件词汇表（与 `AcpEvent` 对齐）

**零新增变体**。agent-core 的对外事件面 = 现有 `AcpEvent`（acp-claude/src/session.rs:79）：

| AcpEvent 变体 | agent-core 中的来源 |
|---|---|
| `TextDelta` / `ThinkingDelta` | gateway SSE `content_block_delta` 透传 |
| `ToolStart{tool_name, args}` | 工具执行前 |
| `ToolProgress` | 长工具的过程回报（bash 输出尾部等） |
| `ToolEnd{result}` | 工具结果（截断后） |
| `PermissionRequest` | **Phase 2 启用**（DH-2/CX-2 的审批升级），变体已存在 |
| `Finished` / `Error{terminal}` | 7.1 终态 |

价值：webui 卡片与飞书卡片消费端**零改动**即可渲染新 agent 的会话——`acp-claude` 与 agent-core 只是同一事件词汇的两个生产者（D7 并存的技术基础）。agent-core 内部可以有更细的中间事件，在驱动层折叠为上述词汇。

## 8. 工具接口与基础工具集契约（D2 展开）

### 8.1 Tool trait

一个 trait，全部工具同构；`name` / `description` / `parameters` 三元组直接映射为 Anthropic `tools` 数组条目（name / description / input_schema）：

```rust
#[async_trait]
trait Tool {
    fn name(&self) -> &'static str;
    /// 何时用 / 何时不用——写给模型看的契约（description 的质量直接影响工具选用）
    fn description(&self) -> String;
    /// JSON Schema，映射 Anthropic tool.input_schema（C5）
    fn parameters(&self) -> JSONSchema;
    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolResult;
}

struct ToolCtx {
    workdir: PathBuf,            // 会话工作目录（webui 传入 project_dir）
    cancel: CancellationToken,   // 7.3 的取消信号
    sink: EventSink,             // 发 ToolProgress
    // Phase 2: sandbox_tier / permission hook（C3 的挂点，D2 预留的"权限检查面"）
}

struct ToolResult { ok: bool, output: String, truncated: bool, error: Option<ToolErrorKind> }

enum ToolErrorKind { InvalidArgs, NotFound, Denied { reason: String },
                     Cancelled, Timeout, Io(String) }
```

两条铁律：

- **错误是数据（C4，DH-1）**：工具失败返回 `ToolResult { ok: false, error: … }` 回填给模型自行纠正，绝不 panic、绝不静默——这与 DSH"拒绝是结构化标记"同构。
- **输出有界**：`output` 超限截断（bash 取尾部 ~30k 字符；grep 上限 250 条；glob 上限 100 条），截断必须置 `truncated: true` 并在 output 里注明（DH-8）。

MCP 不进这个 trait（OQ2）：外部工具协议将来以独立 adapter 落地，Phase 1 工具面只有六个原生实现。

### 8.2 六工具契约

| 工具 | 参数 schema 要点 | 结果语义 | 错误语义（都是数据） | 边界（non-goal） |
|---|---|---|---|---|
| **bash** | `command`(必填)、`timeout_ms?=120_000`、`workdir?` | stdout/stderr 合并输出的尾部；**非零退出码不是工具错误**——返回 `ok:true` + `exit_code` 字段，让模型看到失败输出并自愈 | 超时 → `Timeout`；被取消 → 进程组 kill + `Cancelled` | 无交互 stdin、无 PTY（Phase 3+）；网络访问不受限（OS 沙箱是 Phase 2 议题，CX-1） |
| **read** | `path`(必填)、`offset?=1`、`limit?=2000` | 带行号文本，`offset/limit` 续读大文件（DH-8） | 不存在 → `NotFound`；是目录 → `InvalidArgs` | 二进制文件 Phase 1 只返回"`binary, N bytes`"；读图 Phase 3+ |
| **write** | `path`(必填)、`content`(必填) | 写入成功 + 字节数 | **已存在且本会话未 read 过 → `Denied{reason:"read-before-write"}`**（防盲写覆盖，借鉴 DSH 的 fs-observation-policy） | 不做 mkdir -p 之外的目录管理 |
| **edit** | `path`(必填)、`old_string`(必填)、`new_string`(必填)、`replace_all?=false` | 替换成功 + 替换处数 | `old_string` 匹配 0 次或多于 1 次且未开 `replace_all` → `InvalidArgs` **报出实际匹配数**（模型可据此重试）；未先 read → 同 write 的 `Denied` | 不做正则替换（精确字面量，对齐 CC-2） |
| **glob** | `pattern`(必填)、`path?` | 文件路径列表（**只含文件**），按修改时间排序；上限 100 条 + 截断注明（DH-8） | 非法 pattern → `InvalidArgs` | 不做内容搜索（那是 grep 的事） |
| **grep** | `pattern`(必填, ripgrep 语法)、`path?`、`include?`(glob 过滤) | `文件:行号:行` 按文件分组；上限 250 条 + 截断注明（DH-8） | 正则编译失败 → `InvalidArgs` | 不做多行正则（Phase 1） |

六个工具全部相对 `ToolCtx.workdir` 解析路径；`write` / `edit` 的原子落盘沿用仓库既有惯例（tmp + rename，同 `router/src/state_store.rs` 状态文件的原子重写）。Phase 2 的权限门控挂点就是 `ToolCtx` 的预留位：`Denied` 的产出方从"写前未读"规则扩展为"allow/deny/ask 规则引擎 + `permission-flow` 审批卡"。

## 9. 会话集成：webui 优先（D4 展开）

**状态澄清**：`SessionBackend` 是 `add-core-session-channel` 规划中的会话缝，**当前代码尚不存在**（`webui/src/routes.rs:207` 仍直接调用 `state.router.web_spawn(req.prompt, None)`——`add-project-workbench` 提案同样点名了这一行）。本节定义 agent-core 将实现的会话面，形状与该 change 的 `SessionBackend` 方向对齐；若该 change 先落地，agent-core 直接实现同一 trait；若未落地，agent-core 先自带同形 API，后续收敛。

会话面（trait 形状，命名待实现 change 定）：

```rust
trait AgentSessionBackend {
    async fn create_session(&self, project_dir: Option<String>) -> SessionKey;
    async fn prompt(&self, key: SessionKey, text: String);      // 触发一轮 turn（§7.2）
    async fn cancel(&self, key: SessionKey);                    // §7.3
    fn events(&self, key: SessionKey) -> impl Stream<Item = AcpEvent>;  // webui 经既有 sse.rs 消费
    async fn list(&self) -> Vec<SessionMeta>;                   // 词汇沿用现有 /sessions /resume 语义
}
```

落点与事实依据：

- **project_dir 已有一半地基**：`Mapping.project_dir` 字段存在（router/src/state.rs:37）、`RouterHandle::web_spawn(prompt, project_dir)` 存在（router/src/router/mod.rs:630），webui 恒传 `None`。agent-core 会话的 workdir 就来自这个参数（`ToolCtx.workdir`，§8.1）。
- **webui 零新通路**：事件面就是现有 SSE（webui/src/sse.rs）+ 现有 `AcpEvent` 词汇（§7.4）——webui 消费 agent-core 与消费 acp-claude **代码相同**，只差会话创建时选择的后端。
- **飞书 / CLI 不在本期**：两者将来接同一 `AgentSessionBackend`；本期不改 `feishu` crate、不改 router 命令解析（Non-goals）。
- **持久化（OQ1，只记录不决策）**：选项 A = 复用 `session-lifecycle` 的 state_file 机制；选项 B = agent-core 自有存储。倾向 A（少一套状态文件），留实现 change 裁决。

## 10. 模块边界与 crate 布局（D7 展开）

新 crate **`sebas-agent`**（workspace member；"agent-core" 保留为能力域名），四模块：

```
sebas-agent/
├── loop/      # §7 状态机：AwaitingModel ⇄ ExecutingTools、取消、预算
├── llm/       # LlmClient trait + GatewayClient（HTTP 打本地 gateway，Anthropic 协议）
│              #                  + FakeLlmClient（集成测试用——tests/ 已有 fake-claude 桩先例）
├── tools/     # Tool trait（§8.1）+ bash/read/write/edit/glob/grep 六实现
└── session/   # AgentSessionBackend（§9）+ 内部事件 → AcpEvent 折叠
```

依赖纪律（D7 的技术含义）：

- `agent-core` 依赖：tokio、reqwest、serde/serde_json、async-trait。**不依赖** `feishu` / `router` / `acp-claude` / `webui`。
- LLM 端点是**可配置的 HTTP 端点，不是 crate 依赖**（D3 修订）：默认**直连 provider**——直接使用 provider 数据（base_url / 协议 / api key）；**gateway 是可选路径**（`sebas gateway`，`src/gateway_cmd.rs`）——需要多 provider 模型名路由、用量计量时才启用。两者均为 Anthropic 协议 HTTP 端点。crate 名定 **`sebas-agent`**（沿 sebas-* 惯例；本文原暂定名 `agent-core` 保留为能力域名）。
- **不动**：`acp-claude`（原样并存）、`feishu`、router 命令面、gateway 本体。
- 后端选择发生在会话创建处：webui 创建会话时指定执行后端（acp-claude 子进程 vs agent-core）；Phase 1 用配置项表达即可，复用 `Mapping` 会话映射不动。

回滚（D7 承诺）：停用后端选项即回到纯 acp-claude 路径；本文档本身删除即净。

## 11. 演进路线

每个 Phase 给**入口标准**（达成才开工），checklist 编号见 §5.3：

| Phase | 内容 | 入口标准 | 对应 checklist |
|---|---|---|---|
| **1**（本文 §7–§10） | 核心循环 + 六件套工具 + webui 文本面（后端选项、事件零新增） | `FakeLlmClient` 集成测试跑通多步任务（创建文件 → 编辑 → grep 验证，≥5 次工具调用）；webui 流式渲染全部六工具；取消与预算生效 | C1 C2 C4 C5 C6 C7 C8 |
| **2** | 权限与沙箱：allow/deny/ask 规则引擎挂 `ToolCtx` 预留位；`PermissionRequest` 启用 → `permission-flow` 审批卡（DH-2 升级形态 + CX-2 四档策略枚举）；bash 进程隔离加固（namespace/seccomp，CX-1） | Phase 1 稳定运行；破坏性操作全部经过门控（C3 达成） | C3 |
| **3** | 上下文管理：token 预算细粒度、历史 compaction（CC-1）、会话任务清单（DH-7 / CX-3 `update_plan`）、agent 主动提问（DH-3 → webui 表单 / 飞书卡片表单） | 长会话稳定性需求真实出现（token 触顶成为日常） | C8 完整版、C9 |
| **4** | 子 agent / MCP / 技能 / plan mode：子代理两形态（DH-5 洞察：自包含 vs 继承上下文）、MCP adapter（OQ2）、技能包（DH-6，接 slash 命令）、plan mode（CC-6 → webui"等待操作者"） | Phase 3 稳定 + 具体用例拉动（不是预建） | 扩展项 |

原则：**Phase N 的入口依赖 Phase N-1 的稳定运行，不并行抢跑**；每个 Phase 是独立的实现 change，规格从本文对应章节派生。

> **2026-09-02 修订**：Phase 1a 已完成归档（headless 内核，`openspec/changes/sebas-agent`）。Phase 2 细化为**权限/沙箱（Landlock 进程内为主 + 防火墙回退）+ 网络面（web_search/web_fetch）+ 上下文管理第一步（结果改写/Assembly 预算/只读并行）+ agent-bench 评估面**（openspec change `sebas-agent-next`）。Phase 3 拆分：**3a 上下文 compaction/摘要**、**3b 任务清单 + agent 主动提问**、**3c plan mode + apply_patch**，各自独立 change。**持久化（OQ1）升为路线图显式条目**（DSH session-log 与 Codex thread-store/SQLite 双重佐证），列 Phase 3+。webui 接线（1b）沿 SessionBackend 缝推进，依赖 `add-core-session-channel` 的 channel 谱系收敛。

## 12. 风险与开放问题

风险（→ 缓解）：

- [单文档随代码演进过时] → 文档头部标注"快照于 2026-08-29"；实现 change 归档时回写头部状态与偏差记录
- [Anthropic 协议演进，而 gateway 纯透传不做转换] → agent-core 只锁定 Messages API 的稳定子集（messages + tool use + streaming）；协议升级在 agent-core 单点适配，gateway 不需要感知
- [六件套在长任务中不够用（如缺 WebFetch）] → 工具面按 trait 统一，Phase 2/3 按真实需求扩容；不预先实现（Non-goals）
- [Phase 1 串行工具执行偏慢] → 先正确后快；只读工具并行化列入 Phase 3
- [SessionBackend 缝尚未落地（OQ 风险）] → §9 已定义兜底：agent-core 先自带同形 API，`add-core-session-channel` 落地后收敛，两不阻塞

开放问题（延续 change design 的 OQ1/OQ2，新增 OQ3/OQ4）：

- **OQ1** 会话持久化：复用 `session-lifecycle` state_file（倾向）vs 自有存储——§9 记录，实现 change 裁决
- **OQ2** MCP 进入时点：不进 Phase 1 trait；Phase 4 以 adapter 落地——§8.1 已记录
- **OQ3** 默认模型与 `/model` 衔接：README 已注明 `/model` 未完整接入；agent-core 的模型选择策略留给实现 change
- **OQ4** 多 worktree 并行（S6 提及的 worktree 工作流）是否引入：与 `add-project-workbench` 的多项目视图如何配合——Phase 4 议题

---

## 附录 A — 参考资料清单

证据分级：**[文档]** = 公开文档/官方资料；**[观测]** = 可直接观测的行为（含本文作者运行于 DeepSeek-Harness 之上的一手运行时上下文）；**[推断]** = 由公开信息推断、未经官方确认的内容（正文使用处逐点标注）。

| 编号 | 资料 | 级别 | 用于 |
|---|---|---|---|
| S1 | [Claude Code — Tools reference](https://code.claude.com/docs/en/tools-reference) | [文档] | §2 工具集与工具契约 |
| S2 | [Claude Code — Hooks reference](https://code.claude.com/docs/en/hooks) | [文档] | §2 钩子与权限联动 |
| S3 | [Claude Code — Configure permissions](https://code.claude.com/docs/en/agent-sdk/permissions) | [文档] | §2 权限模式与规则 |
| S4 | [Codex CLI — CLI reference（open-docs 镜像）](https://github.com/bgauryy/open-docs/blob/main/docs/codex_cli/17-cli-reference.md) | [文档] | §3 沙箱/审批开关面 |
| S5 | [Codex — system prompts（open-docs 镜像）](https://github.com/bgauryy/open-docs/blob/main/docs/codex_cli/05-system-prompts.md) | [文档] | §3 循环约束与工具面 |
| S6 | [Codex 完整避坑指南（2026 版）：沙箱、权限、AGENTS.md、Worktree](https://cloud.tencent.com.cn/developer/article/2704656) | [文档] | §3 沙箱实践与社区经验 |
| S7 | [DeepSeek Harness 工具清单：内置工具与执行流水线](https://www.ai-indeed.com/encyclopedia/29669.html) | [文档] | §4 内置工具与流水线 |
| S8 | [dsh-agent-sdk — Embeddable runtime built on DeepSeek Harness](https://github.com/salathleizhang/dsh-agent-sdk) | [文档] | §4 运行时形态佐证 |
| S9 | DeepSeek-Harness 运行时上下文（本文作者运行环境：工具清单、文件沙箱策略、审批政策、goal/job/subagent 编排均为一手可观测行为） | [观测] | §4 全部机制行 |
| S10 | [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)（2026-08-13 开源，MIT）及其 docs/（tool-catalog、subsystems/sandbox、subsystems/approval、agent-lifecycle、tool-execution-pipeline） | [源码] | §4 修订（2026-09-02） |
| S11 | [openai/codex](https://github.com/openai/codex)（Rust monorepo，2026-09 快照）：codex-rs/{sandboxing,linux-sandbox,protocol,core/src/tools,thread-store,state,rollout-trace,exec} | [源码] | §3 修订（2026-09-02） |

注：Codex 内部实现（Seatbelt/Landlock 细节等）以官方文档与 S4–S6 所述为准，超出部分在正文标 [推断]。DeepSeek-Harness 无公开完整设计文档，S7/S8 为第三方整理，机制描述以 S9 一手观测为准、S7/S8 佐证。


---

## 12. 修订台账（2026-09-02，openspec change `sebas-agent-next`）

| # | 位置 | 原裁决 | 修订后 | 依据 |
|---|---|---|---|---|
| R1 | §3 CX-1 | Codex Linux 用 Landlock/Seatbelt | 默认 **bwrap+seccomp**，Landlock legacy 回退；拒绝启发式 `is_likely_sandbox_denied` | S11 源码 |
| R2 | §3 CX-3 | Codex 单工具收敛路线（skip） | 2026 工具面已大幅扩张（unified_exec/MCP/多 agent/code_mode）；结论改立"面可控性" | S11 源码 |
| R3 | §3 新增 | — | 采纳：ApprovedForSession 会话批准缓存、一次性未沙箱升级、rollout-trace 轨迹、codex exec 无头事件 | S11 源码 |
| R4 | §4 | DSH 机制靠 S9 观测 | 证据升级 [源码]（S10）；流水线五段印证 S7 | S10 源码 |
| R5 | §11 | Phase 2 = 权限/沙箱；Phase 3 单体 | Phase 2 扩为四件套（沙箱/网络/上下文/bench）；Phase 3 拆 3a/3b/3c；持久化升显式条目 | 本 change 实施范围 |
| R6 | §10 依赖面 | agent 不依赖 landlock/cap-std | sebas-agent 新增 `landlock`（Linux）；**弃 cap-std**（管不住子进程）；网络面新增 url/mime_guess | N2/N3 选型研究 |

注：本台账逐条对应 `openspec/changes/sebas-agent-next` 的 tasks 7.1–7.3；归档时按仓库惯例回写本文头部状态。

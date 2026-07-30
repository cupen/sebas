# sebas 走查（Audit）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 产出一份经过核实的、带 P0–P4 优先级的 sebas 问题 backlog（全部落入 beads），并把 README / spec §7 的过时声明刷新为真实状态。

**Architecture:** 三个并行只读审计 subagent（A 文档声明 / B 代码 vs spec / C 测试盘点）→ 汇总去重定优先级 → 批量建 beads → 刷新 README 与 spec §7 → 质量门 + handoff 报告。

**Tech Stack:** bd (beads issue tracker), git, cargo, markdown 中间产物（/tmp，不进 git）。

**本计划是审计计划，不是代码开发计划**：没有 TDD 循环；Task 1–3 是"派发 subagent + 验证产出"，Task 4–8 是编排、bd 命令与文档编辑。全程不修代码 bug（spec §2 原则 1）。

## Global Constraints

- **严格只读**：Task 1–3 的审计 subagent 除各自输出文件外不修改任何仓库文件。
- **允许修改的文件只有**：Task 6 的 `README.md`、Task 7 的 `docs/superpowers/specs/2026-07-26-sebas-design.md` §7。
- **不执行 git commit / push**（conservative profile）；Task 8 报告建议命令，等用户批准。
- 若用户批准提交，commit message 遵循 Conventional Commits（见 `.claude/rules/how-to.md`）。
- 中间产物目录：`/tmp/sebas-audit-2026-07-28/`，不进 git；**beads 的 description 必须自包含**（证据内联），不允许引用 /tmp 文件路径。
- 每条发现必须有证据（`file:line` 或 commit hash）；无证据的发现不进 backlog。
- 优先级口径（spec §4）：P0=daemon 不可用/丢数据；P1=核心链路不工作（消息收发、权限审批、session 管理）；P2=功能缺失有 workaround；P3=文档/测试/工具；P4=改进项。
- 任务跟踪一律用 `bd`；禁用 TodoWrite / TaskCreate / markdown TODO。

---

### Task 1: 审计 A — 文档声明核实（与 Task 2、3 并行派发）

**Files:**
- Read: `README.md`、`docs/superpowers/specs/2026-07-26-sebas-design.md` §7、`git log`、`feishu/src/`、`src/main.rs`、`router/src/state.rs`、`router/src/commands.rs`、`tests/`、`.github/`
- Create: `/tmp/sebas-audit-2026-07-28/findings-a.md`

**Interfaces:**
- Consumes: 无（首批任务）
- Produces: `findings-a.md`，含 9 个小节 `## A1` … `## A9`，每节含 `- 结论：`（已过时/仍成立/部分成立）与 `- 建议处理：`（仅文档刷新 / 建 beads）。Task 4 与 Task 6 消费此文件。

- [ ] **Step 1: 派发审计 A subagent**

用 Agent 工具（`subagent_type: general-purpose`，run_in_background 与 Task 2、3 并行），prompt 全文如下：

```
你是 sebas 项目的只读审计员。sebas 是一个 Rust daemon，把 Claude Code（经 ACP 协议）桥接到飞书（Feishu），仓库在 /home/cupen/workbench/repos-tool/sebas。

严格只读：除你的输出文件 /tmp/sebas-audit-2026-07-28/findings-a.md 外，不修改任何文件。

任务：核实 README.md 与 docs/superpowers/specs/2026-07-26-sebas-design.md §7 中"未验证/待定/局限"声明是否仍然成立。逐项核查：

A1. README "Status" 段："The WebSocket long-connection URL and handshake against a real Feishu workspace have not been verified end-to-end"
    线索：git log --oneline -30 中是否有只能在真实环境发现的修复（如 8d1cd66 card.action.trigger、ef9dbfc duplicate prompt）；feishu/src/client.rs 长连接实现是否完整。
A2. README "Status" + Known limitations："Coverage tooling (cargo-llvm-cov) is not yet configured in CI" / "No coverage thresholds enforced in CI"
    线索：ls .github/workflows/；全仓库 grep llvm-cov。
A3. README "Status" + Known limitations + spec §7："record subcommand deferred / not implemented"
    线索：src/main.rs 的 CLI 子命令枚举；src/replay.rs（注意 replay 是另一个子命令，不要混淆）。
A4. README Known limitations："SessionMap is in-memory only and lost on restart; sessions are restored lazily on next message per chat, but in-progress work is not resumable"
    线索：router/src/state.rs 是否落盘 sessions.json；src/run.rs 启动时是否 restore。
A5. README Known limitations："tests/bin/fake-claude.rs exists, but no production test harness with real ACP protocol fixtures"
    线索：tests/fixtures/ 目录内容；tests/bin/ 下有什么。
A6. README Known limitations："/compact, /cost, /model, /cd are dispatched to the ACP backend but not validated end-to-end"
    线索：router/src/commands.rs 中这 4 个命令的转发实现。
A7. spec §7："飞书群聊 @ 机器人 的具体消息格式——实现阶段确认"
    线索：feishu/src/events.rs 是否解析 @ mention；router 是否只在被 @ 时响应群消息。
A8. spec §7："Claude Code ACP 子命令的精确协议——实现前确认"
    线索：git show 76a4b56 --stat（迁移官方 SDK）；acp-claude/src/manager.rs 现在 spawn 什么命令。
A9. spec §7："SessionKey 已预留 user_id 字段"
    线索：router/src/ 中 SessionKey 结构体定义是否真有 user_id 字段。

每条结论三选一：已过时（声明所述问题已不存在）/ 仍成立 / 部分成立。

先执行 mkdir -p /tmp/sebas-audit-2026-07-28，然后写入 findings-a.md，每条格式：

## A1 — <声明一句话摘要>
- 声明出处：README.md:36 或 spec §7
- 证据：<file:line 或 commit hash，至少一条；无证据必须写"未找到证据">
- 结论：已过时 | 仍成立 | 部分成立
- 说明：<一两句>
- 建议处理：仅文档刷新 | 建 beads（附建议标题）
- 建议优先级：P0 | P1 | P2 | P3 | P4（P0=daemon 不可用/丢数据；P1=核心链路不工作；P2=功能缺失有 workaround；P3=文档/测试/工具；P4=改进项）

完成后，把你的 final message 设为 findings-a.md 的完整内容（不要省略、不要总结）。
```

- [ ] **Step 2: 验证产出**

```bash
test -f /tmp/sebas-audit-2026-07-28/findings-a.md
grep -c '^## A[0-9]' /tmp/sebas-audit-2026-07-28/findings-a.md   # 期望: 9
grep -c '^- 结论：' /tmp/sebas-audit-2026-07-28/findings-a.md     # 期望: 9
grep -c '^- 证据：' /tmp/sebas-audit-2026-07-28/findings-a.md     # 期望: 9
```

数量不符则把缺口退回该 subagent 补做（SendMessage 继续同一会话）。

- [ ] **Step 3: 审读**

通读 findings-a.md：每条证据真实可复核（抽查 2 条 `file:line` 是否属实）；"已过时"结论尤其要查证据。发现编造证据 → 退回重做。

---

### Task 2: 审计 B — 代码实现 vs spec（与 Task 1、3 并行派发）

**Files:**
- Read: `docs/superpowers/specs/2026-07-26-sebas-design.md`（§3.2/§3.3/§4.1/§5/§6.2）、`router/src/`、`feishu/src/`、`acp-claude/src/`、`src/config.rs`、`src/run.rs`
- Create: `/tmp/sebas-audit-2026-07-28/findings-b.md`

**Interfaces:**
- Consumes: 无（首批任务）
- Produces: `findings-b.md`，含 24 个小节 `## B1` … `## B24`，每节含 `- 结论：`（已实现/未实现/部分实现）与 `- 建议 issue type：`。Task 4 消费此文件。

- [ ] **Step 1: 派发审计 B subagent**

Agent 工具（`general-purpose`，后台并行），prompt 全文：

```
你是 sebas 项目的只读审计员。sebas 是 Rust daemon：飞书 ←→ router ←→ ACP ←→ Claude Code 子进程。仓库 /home/cupen/workbench/repos-tool/sebas。crate 布局：feishu/（飞书 client、卡片、事件、媒体）、router/（session 映射、slash 命令、状态持久化）、acp-claude/（ACP 子进程管理）、src/（main、config、run、replay、install_service）。

设计文档：docs/superpowers/specs/2026-07-26-sebas-design.md（下称 spec）。

严格只读：除输出文件 /tmp/sebas-audit-2026-07-28/findings-b.md 外，不修改任何文件。

任务：逐条核对 spec 中的设计是否在代码中实现。先读 spec 对应小节，再读代码：

B1–B9 — spec §4.1 错误处理矩阵，9 行逐行核对"处理"列是否实现：
  B1 Feishu transient：指数退避重试 3 次 → feishu/src/client.rs
  B2 Feishu auth 致命：log fatal + 退出 → feishu/src/client.rs, src/run.rs
  B3 ACP child crash：标记 session 死 + 卡片 ❌ + 不影响其他 session → router/src/router.rs, acp-claude/src/manager.rs
  B4 ACP child hang：5min 无 notification → Cancel×3 → SIGTERM → 5s → SIGKILL → acp-claude/src/manager.rs
  B5 ACP spawn failure：卡片 ❌ + 安装提示，不建 session → router/src/router.rs
  B6 Mapping miss（ButtonCb 对应死 session）：warn + 回复提示 → router/src/router.rs
  B7 Channel send fail：dev panic / prod log → router/src/router.rs, src/run.rs
  B8 /switch 非法参数 → usage hint → router/src/commands.rs
  B9 权限请求永不超时 → router/src/router.rs, acp-claude/src/session.rs
B10–B20 — spec §5 的 11 个 slash 命令（/new /sessions /switch /resume /cancel /status /compact /cost /model /cd /help）逐个核对：router/src/commands.rs 是否实现；处理者（router 本地 vs 转发 ACP）是否与 spec 表格一致。
B21 — spec §3.2 emoji 状态机：root card 上 👀→🚧→✅ 切换，中间 tool 不单独加 emoji → router/src/router.rs, feishu/src/cards.rs（搜 emoji 相关常量/字符串）。
B22 — spec §3.3(e) 重启恢复：启动读 sessions.json、懒加载 spawn_resume、失败 fallback create_session → router/src/state.rs, src/run.rs, acp-claude/src/manager.rs。
B23 — spec §3.3(f) 媒体消息：下载到 download_dir、caption 拼接 "<caption>\n\n[attached: <path1>, <path2>]" → feishu/src/media.rs, router/src/。
B24 — spec §6.2 配置表 vs src/config.rs：逐字段核对是否存在且有默认值，列出 spec 有但 config.rs 缺失的字段（这就是结论内容）。

每条结论三选一：已实现 / 未实现 / 部分实现。

先执行 mkdir -p /tmp/sebas-audit-2026-07-28，然后写入 findings-b.md，每条格式：

## B1 — <核查项一句话>
- spec 出处：§4.1 第 3 行 等
- 证据：<file:line；未实现则写"未找到实现（已搜索关键词 xxx）">
- 结论：已实现 | 未实现 | 部分实现
- 说明：<一两句>
- 建议优先级：P0 | P1 | P2 | P3 | P4（P0=daemon 不可用/丢数据；P1=核心链路不工作；P2=功能缺失有 workaround；P3=文档/测试/工具；P4=改进项）
- 建议 issue type：bug | feature | task | chore

完成后，把你的 final message 设为 findings-b.md 的完整内容（不要省略、不要总结）。
```

- [ ] **Step 2: 验证产出**

```bash
test -f /tmp/sebas-audit-2026-07-28/findings-b.md
grep -c '^## B[0-9]' /tmp/sebas-audit-2026-07-28/findings-b.md   # 期望: 24
grep -c '^- 结论：' /tmp/sebas-audit-2026-07-28/findings-b.md     # 期望: 24
```

- [ ] **Step 3: 审读**

抽查 3 条"未实现"结论（亲自 grep 对应关键词确认确实没有）；抽查 2 条"已实现"的 `file:line`。发现编造 → 退回重做。

---

### Task 3: 审计 C — 测试与工程质量盘点（与 Task 1、2 并行派发）

**Files:**
- Read: `*/tests/`、`Cargo.toml`（workspace 与各 crate）、`.github/`、`.cargo/`、`docs/superpowers/specs/2026-07-26-sebas-design.md` §4.3
- Create: `/tmp/sebas-audit-2026-07-28/findings-c.md`

**Interfaces:**
- Consumes: 无（首批任务）
- Produces: `findings-c.md`，含 5 个小节 `## C1` … `## C5`，每节含 `- 差距：` 与 `- 建议优先级：`。Task 4 消费此文件。

- [ ] **Step 1: 派发审计 C subagent**

Agent 工具（`general-purpose`，后台并行），prompt 全文：

```
你是 sebas 项目的只读审计员。仓库 /home/cupen/workbench/repos-tool/sebas（Rust workspace，crate：sebas 主 crate + feishu + router + acp-claude）。

严格只读：除输出文件 /tmp/sebas-audit-2026-07-28/findings-c.md 外，不修改任何文件。允许运行只读命令（ls、grep、cargo test -- --list 等）；不要安装任何东西。

任务：测试与工程质量盘点。

C1. 测试清单：统计每个 crate 的测试文件与测试数（ls */tests/ tests/；各 crate 跑 cargo test -p <crate> -- --list 计数），概括各自覆盖的区域（如 cards snapshot、router 状态机、acp 解析）。
C2. CI：.github/、.gitlab-ci.yml、Makefile、justfile 等是否存在；有无任何 CI 配置。
C3. cargo-llvm-cov：Cargo.toml / .cargo/config.toml 中有无覆盖率配置；cargo llvm-cov 子命令是否可用（只报告，不安装）。
C4. 覆盖率目标（router/cards ≥90%、整体 ≥80%，见 docs/superpowers/specs/2026-07-26-sebas-design.md §4.3）：有无任何强制机制。
C5. 真实 ACP fixture harness 差距：tests/fixtures/ 现有什么；tests/bin/ 现有什么；对照 spec §4.3 提到的 tests/acp_against_canned_binary.rs 与 tests/feishu_card_golden.rs，现有 tests/ 里对应物是什么、还缺什么。

先执行 mkdir -p /tmp/sebas-audit-2026-07-28，然后写入 findings-c.md，每条格式：

## C1 — <主题>
- 证据：<命令输出摘要 / 文件列表>
- 现状：<客观描述>
- 差距：<与 spec/目标的差距，没有差距写"无">
- 建议优先级：P0 | P1 | P2 | P3 | P4（测试/工具类一般 P3；核心链路完全无测试可 P2）
- 建议 issue type：task | chore | feature

完成后，把你的 final message 设为 findings-c.md 的完整内容（不要省略、不要总结）。
```

- [ ] **Step 2: 验证产出**

```bash
test -f /tmp/sebas-audit-2026-07-28/findings-c.md
grep -c '^## C[0-9]' /tmp/sebas-audit-2026-07-28/findings-c.md   # 期望: 5
grep -c '^- 差距：' /tmp/sebas-audit-2026-07-28/findings-c.md     # 期望: 5
```

- [ ] **Step 3: 审读**

C1 的测试计数与 `cargo test --workspace 2>&1 | grep -c "test result"` 量级是否吻合；C2 与 `ls -a` 结果是否吻合。

---

### Task 4: 汇总去重 + 定优先级

**Files:**
- Read: `/tmp/sebas-audit-2026-07-28/findings-{a,b,c}.md`
- Create: `/tmp/sebas-audit-2026-07-28/backlog.md`

**Interfaces:**
- Consumes: Task 1–3 的 findings 文件（小节 ID `A1–A9` / `B1–B24` / `C1–C5`）
- Produces: `backlog.md`，每项为 `## BK-N`，字段：`title`、`type`（bug/task/feature/chore）、`priority`（0–4）、`来源`、`description`（自包含多行）、`depends_on_titles`。Task 5 逐项消费。

- [ ] **Step 1: 派发汇总 subagent（或主编排会话亲自做）**

汇总是判断性工作，需要用户痛点上下文（可靠性/体验 + 完整性优先），建议由主编排会话亲自做；若派 subagent，prompt 全文：

```
读取 /tmp/sebas-audit-2026-07-28/findings-a.md、findings-b.md、findings-c.md，合并为去重后的 backlog。

只把"仍成立 / 部分成立 / 未实现 / 部分实现"的项进 backlog；"已过时 / 已实现"的项不进（它们是后续 README 刷新的输入，不需要 beads）。

已知重叠（必须合并，也可能发现新的重叠）：
- A2 ≈ C2/C3/C4（CI 与覆盖率）→ 一个 chore
- A3（record 子命令，若仍成立）→ 一个 feature
- A5 ≈ C5（真实 ACP fixture harness）→ 一个 task
- A6 与 B10–B20 中"已实现但未验证"的 slash 命令 → 合并为一个"slash 命令真实环境验证"task
- A4 与 B22（session 持久化/重启恢复）→ 视结论合并或拆分

另外固定追加一项（这是走查 spec §2 原则 3 延期的动态测试）：
- title: "动态 smoke test（真实飞书）：README 6 步 + slash 全表 + 群聊@ + 媒体消息"
  type: task, priority: 2
  description 要点：静态走查（2026-07-28）延期的部分。覆盖：README "Manual smoke test" 6 步；11 个 slash 命令逐个真实飞书验证；群聊 @ 机器人；图片/文件/语音消息。发现的问题各自建 beads。

用户优先级上下文：可靠性/体验问题 > 功能完整性 > 文档/测试/工具。定级时体现。

输出 /tmp/sebas-audit-2026-07-28/backlog.md，每项严格用此格式：

## BK-1
- title: <≤60 字符，直接可用作 bd 标题>
- type: bug | task | feature | chore
- priority: 0 | 1 | 2 | 3 | 4
- 来源: A2, C3
- description: |
    出处：<spec 小节 / README 位置>
    证据：<file:line 或 commit hash>
    现状：<一句话>
    建议：<修复方向一句话>
- depends_on_titles: [<其它 BK title；没有则写 []]

依赖判断示例："slash 命令真实环境验证" 依赖 "动态 smoke test"；其余按实际判断。

description 必须自包含——不允许引用 /tmp 路径（/tmp 会消失）。

final message 返回 backlog.md 完整内容 + 每项一行的一览表（BK-N | priority | type | title）。
```

- [ ] **Step 2: 验证产出**

```bash
test -f /tmp/sebas-audit-2026-07-28/backlog.md
grep -c '^## BK-' /tmp/sebas-audit-2026-07-28/backlog.md        # 期望: ≥5（至少 A2/A3/A5/合并slash/动态smoke 各一）
grep '/tmp' /tmp/sebas-audit-2026-07-28/backlog.md              # 期望: 无输出（description 不得引用 /tmp）
```

- [ ] **Step 3: 审读**

检查：无重复项；每项有证据；priority 符合口径且体现"可靠性 > 完整性 > 文档/测试"；`depends_on_titles` 只引用存在的 BK title。

---

### Task 5: 建 beads issues

**Files:**
- Read: `/tmp/sebas-audit-2026-07-28/backlog.md`
- Modify: beads 数据库（经 `bd` CLI）

**Interfaces:**
- Consumes: Task 4 的 `backlog.md`（BK-N 项及其字段）
- Produces: 一个 epic + N 个子 issue 的 beads ID 列表；Task 6/7 在文档中引用这些 ID。

- [ ] **Step 1: 查重**

```bash
bd list                 # 全部状态，确认现有 9 条 closed 与 backlog 无重叠
bd search record        # 确认 record 子命令没有已有 issue
bd search smoke         # 确认动态 smoke test 没有已有 issue
```

如发现某 BK 项已有等价 issue（包括 closed），在 backlog.md 中标注映射关系，跳过该项的创建。

- [ ] **Step 2: 建 epic**

```bash
bd create --title="2026-07-28 走查 backlog" --type=epic --priority=2 \
  --description="静态走查发现的问题合集。设计: docs/superpowers/specs/2026-07-28-sebas-audit-design.md"
```

记录返回的 epic ID（下称 `<epic>`）。

- [ ] **Step 3: 逐项建 issue**

对每个未跳过的 BK 项执行（description 多行用单引号包裹即可，zsh/bash 均支持）：

```bash
bd create --title="<BK title>" --type=<type> --priority=<priority> --parent=<epic> \
  --description='<description 全文，含 出处/证据/现状/建议 四行>'
```

示例（假设 BK-1 是 CI/覆盖率）：

```bash
bd create --title="配置 CI 与 cargo-llvm-cov 覆盖率门槛" --type=chore --priority=3 --parent=sebas-xxx \
  --description='出处: spec §4.3 覆盖率目标；README Known limitations
证据: 无 .github/workflows/（C2）；无 llvm-cov 配置（C3）
现状: 无任何 CI，覆盖率目标（router/cards ≥90%，整体 ≥80%）无强制机制
建议: 加 GitHub Actions workflow，跑 cargo test + cargo llvm-cov 并设门槛'
```

把每个返回的 issue ID 记回 backlog.md 对应 BK 项下（追加一行 `- beads: <id>`）。

- [ ] **Step 4: 建依赖**

对每项 `depends_on_titles` 非空的 BK：

```bash
bd dep add <被阻塞 issue-id> <依赖的 issue-id>
```

至少应有：`bd dep add <slash验证> <动态smoke-test>`。

- [ ] **Step 5: 验证**

```bash
bd list --status=open    # 期望: 1 个 epic + N 个子 issue
bd stats                 # open 数量与上一致
bd dep list <epic> 2>/dev/null || bd show <epic>   # epic 下能看到全部子项
```

---

### Task 6: README 刷新

**Files:**
- Read: `README.md`、`/tmp/sebas-audit-2026-07-28/findings-a.md`、`/tmp/sebas-audit-2026-07-28/backlog.md`（取 beads ID）
- Modify: `README.md`（仅 `## Status` 与 `## Known limitations` 两节）

**Interfaces:**
- Consumes: Task 1 的 A 系结论、Task 5 的 beads ID
- Produces: 刷新后的 README.md，供 Task 8 统一 diff 审阅。

- [ ] **Step 1: 按规则改写两节**

改写规则（README 保持英文，与现有一致）：

1. 结论为"已过时"的声明 → 直接删除。
2. 结论为"仍成立"且有对应 beads → 保留但改写为精确描述，行尾加 `(tracked: <beads-id>)`。
3. 结论为"部分成立" → 改写为准确描述现状 + `(tracked: <beads-id>)`。
4. `## Status` 段首句 "This is an MVP / work-in-progress." 保留。

改写示例（假设 A6 仍成立、对应 beads 为 sebas-x9）：

```markdown
# Before
- Slash commands `/compact`, `/cost`, `/model`, `/cd` are dispatched to the ACP backend but their protocol-level behavior has not been validated end-to-end.

# After
- `/compact`, `/cost`, `/model`, `/cd` are dispatched to ACP but not yet validated against a real workspace (tracked: sebas-x9).
```

- [ ] **Step 2: 验证**

```bash
git diff README.md                                    # 人工审阅：只动了 Status / Known limitations 两节
grep -n "tracked: sebas-" README.md                   # 每条保留的 limitation 都有追踪号
grep -in "not been verified end-to-end" README.md     # 若 A1 结论为"已过时"，此处应无输出
```

---

### Task 7: spec §7 标注结论

**Files:**
- Read: `docs/superpowers/specs/2026-07-26-sebas-design.md` §7（第 332–339 行）、`/tmp/sebas-audit-2026-07-28/findings-a.md`
- Modify: `docs/superpowers/specs/2026-07-26-sebas-design.md`（仅 §7「待定 / 后续」小节）

**Interfaces:**
- Consumes: Task 1 的 A7/A8/A9 结论、Task 5 的 beads ID
- Produces: 标注后的 spec，供 Task 8 统一 diff 审阅。

- [ ] **Step 1: 逐条加结论标注**

保留原文，每条前置状态标记（spec 保持中文）：

- 已解决 → `- ✅ ~~原文~~ — 结论：<一句话>（证据：<commit/file>）`
- 仍待定且已建 beads → `- ⏳ 原文 — 结论：<一句话>（tracked: <beads-id>）`
- 仍待定未建 beads（纯设计议题）→ `- ⏳ 原文 — 结论：<一句话>`

示例（A8 已被官方 SDK 迁移解决时）：

```markdown
- ✅ ~~Claude Code ACP 子命令的精确协议~~ — 已解决：迁移官方 agent-client-protocol v2 SDK（76a4b56）
```

同时把 §7 开头"> **已完成**"引用块之后追加一行："> **2026-07-28 走查**：以下各条已核实并标注结论。"

- [ ] **Step 2: 验证**

```bash
git diff docs/superpowers/specs/2026-07-26-sebas-design.md   # 人工审阅：只动了 §7
grep -cE '^- (✅|⏳|❌)' docs/superpowers/specs/2026-07-26-sebas-design.md   # 期望: 标注条数 == 原 §7 待核实条数（≥3）
```

---

### Task 8: 质量门 + 走查总结 + handoff

**Files:**
- Read: 全部产出（backlog.md、README diff、spec diff）
- Modify: 无（不 commit）

**Interfaces:**
- Consumes: Task 5 的 beads ID 列表、Task 6/7 的 diff
- Produces: 会话内总结报告 + 建议 git 命令（等用户批准）

- [ ] **Step 1: 质量门**

```bash
cargo test --workspace 2>&1 | grep -E "test result: FAILED|error\["   # 期望: 无输出（本次只动文档，测试应仍全绿）
git status --short
```

`git status` 期望：

```
 M README.md
 M docs/superpowers/specs/2026-07-26-sebas-design.md
?? docs/superpowers/specs/2026-07-28-sebas-audit-design.md
?? docs/superpowers/plans/2026-07-28-sebas-audit.md
```

若有其它改动 → 查明来源，非本计划产生的改动不得混入。

- [ ] **Step 2: 会话内总结**

输出表格：每个新建 beads（id / priority / type / title）+ findings 统计（A/B/C 各几条、已过时/仍成立分布）+ README/spec 改动摘要。

- [ ] **Step 3: 报告建议命令并等待批准**

```bash
git add README.md docs/superpowers/specs/2026-07-26-sebas-design.md \
        docs/superpowers/specs/2026-07-28-sebas-audit-design.md \
        docs/superpowers/plans/2026-07-28-sebas-audit.md
git commit -m "docs: 走查 backlog 落入 beads，README/spec 声明刷新为核实状态"
```

说明：中间产物在 `/tmp/sebas-audit-2026-07-28/`（重启自动清理，仓库无污染）；动态 smoke test 已建 beads（P2），待静态问题修完后执行。**未经用户批准不执行 commit。**

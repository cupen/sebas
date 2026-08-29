# Design — remove-superpowers-docs

## Context

`docs/superpowers/`(23 文件,~17.5k 行)是 OpenSpec 采纳前的规划产物。逐项核对结论(详见 proposal Why):

- **行为语义已同步**:`bootstrap-specs` 已把当前状态回填进 19 个 capability specs;卡片流语义(`feishu-cards`)、配置发现(`cli-service`)、gateway 行为(`gateway-*`)、provider 管理(`provider-management`)、control RPC 姿态(`watchdog` spec + `add-core-session-channel/design.md`)、fake-claude 契约(`tests/bin/fake-claude.rs` 头注释自文档化)均已覆盖,无需重复迁移。
- **未同步的只有两类**:「为什么」类设计决策(无家);两条遗留承诺/建议(provider 评审的 routes 决策、审计文档的 P1–P3 建议)。
- **死链清单**:12 处显式路径 + 64 处带日期引用(`spec 2026-08-17 §N`,15 个 .rs + `.claude/rules/how-to.md`)+ 133 处裸 `spec §N`(46 个 .rs)。

约束:纯文档/工具变更,零运行时行为变化;conservative git 政策;任务块 ≤ 2h。

## Goals / Non-Goals

**Goals**
- 删除前把仍有价值的内容安置到可持续维护的位置(ADR 文档 / beads)。
- 全源码树零幽灵引用(有 `check-docs` 防回归)。
- 删除后 README/config 的文档入口指向 openspec specs。

**Non-Goals**
- 不改运行时代码行为(注释文字修改除外)。
- 不迁移行为语义到 specs;不碰 `openspec/changes/archive/`。
- 不在本 change 内修复审计建议的代码问题。

## Decisions

### D1 — 同步目的地:docs/design-history.md(ADR 式),而非 spec 前言

用户已拍板(备选:写入 19 个 spec 前言 / 依赖 git 历史)。理由:OpenSpec specs 描述行为,rationale 是非行为内容,塞进前言会稀释 spec 结构;docs/ 已有先例(perm-flow 序列图)。每条 ADR:**日期 / 背景 / 决策 / 后果 / 原文路径(git 历史可考)**。

收录清单(apply 时从对应文档蒸馏,每条 10–20 行):
1. 弃 ACP bridge、经 cc-agent-sdk 直连(源:`2026-08-06-claude-direct-sdk-refactor-design.md` §1.1/§2)
2. 卡片流模型选型「方案 A」(源:`2026-07-30-card-streaming-model-design.md` §3)
3. gateway 单端口双协议面 + `[[gateway.keys]]` per-key 配额简化(源:`2026-08-06-gateway-design.md` §4.1/§4.2)
4. provider state v0→v2 统一进 state.json(源:`2026-08-17-provider-design-review.md` §2.6 决策记录)
5. provider 评审决策记录摘要(12/15 落地 + 未竟项)(源:同上 §5)
6. watchdog 控制平面 Phase 0–3 分期与 IPC 兼容契约(源:`2026-08-14-watchdog-control-plane-design.md` §14)

### D2 — 引用清理策略:家族映射,非逐条考古

裸 `spec §N` 按内容归 ~5 个家族,每家族一次决策、批量替换:

| 家族 | 判定特征 | 替换目标 |
|---|---|---|
| 卡片流/节流/FSM | card_state、pump、dispatch、feishu/cards.rs | 指向 `feishu-cards` spec;纯实现细节(如 debounce 值)就地内联 |
| gateway 路由/鉴权/用量 | gateway/src + tests(Task N,spec §4.x) | 指向 `gateway-core`/`gateway-auth-rate-limit`/`gateway-metrics` |
| provider 状态/解析 | state_store、crud、provider_state、spawn_env | 指向 `provider-management`;纯历史(改名记录)删标签留结论 |
| 命令/控制面 | cli.rs `/webui`、`/gateway`、watchdog | 指向 `cli-service`/`watchdog`(apply 时按 §12 实际出处核对) |
| acp 生命周期 | acp-claude/driver、manager、session | 指向 `acp-driver` |

带日期引用(`spec 2026-08-17 §N`)多为「决策已落地」的历史注解(如 §2.5 改名、§2.6 合并)→ 就地删除引用标签、保留结论文字;仍描述现行行为的 → 指向对应 spec。README×2 与 config.toml.example×2 改指 `openspec/specs/` 对应 capability 与 config example 自身(卡片默认值已内联,无需文档兜底)。

### D3 — 遗留承诺/建议的处置:开 bead,不开新 change

- 「gateway TOML routes 后续由 webui 编辑」→ bead(它依赖 webui 演进,与在途 webui changes 汇合,单独立 change 会空转)。
- 审计文档 5 条建议:apply 时逐项核对现状;已修→ADR 附记;未修→bead(P1 loopback 前置拦截经查仍未落地:`webui_cmd.rs:66` 检查仍在子进程)。
- 备选(为审计建议开 openspec change)被否:建议粒度小、未定优先级,先入 bead 待认领。

### D4 — 防回归:xtask `check-docs`

扫描 `*.rs|*.toml|*.md`(排除 `target/`、`openspec/changes/archive/`、`docs/design-history.md` 自身),命中以下模式即失败:
- 字面量 `docs/superpowers/`
- 正则 `spec \d{4}-\d{2}-\d{2}`(带日期引用)
- 正则 `spec §\d`(裸章节引用)

纳入已有 xtask 子命令体系,带单元测试(命中/豁免/干净三例)。理由:209 处引用靠人肉不回归不现实;一条 20 行的 grep 检查把这次清理固化成约束。

### D5 — 执行与提交顺序

同步(ADR + beads)→ 清理引用(家族分批,每批可独立验证)→ `check-docs` 落地 → 最后删除文档。每批一个 commit(Conventional Commits,`docs:`/`chore:`/`test:`);删除 commit 放最后,保证任意中间 commit 可构建、可回退(删除仅涉及 docs/,`git revert` 即恢复)。

## Risks / Trade-offs

- [裸 `spec §N` 家族误判,替换指向错误 spec] → 每家族替换前抽查 3–5 处上下文;`check-docs` 保证零残留,但指向正确性靠人工抽查,任务块内写明抽查要求。
- [ADR 蒸馏失真(遗漏关键约束)] → 每条 ADR 注明原文 git 路径,可回溯;收录清单经用户确认(本 design)。
- [删除后发现仍需某文档] → git 历史完整保留;ADR 条目均附原文路径,恢复成本 ≈ 0。
- [`.claude/rules/how-to.md` 属 agent 规则文件,改动影响后续 agent 行为] → 仅删一处日期引用,其余内容不动。

## Migration Plan

一次性文档切换,无部署面。顺序见 D5;回退 = `git revert` 对应 commit。

## Open Questions

(无 — 三项范围决策均已由用户确认:全量清理、docs/ ADR、审计文档一并删除。`cli.rs` `spec §12` 的确切出处属 apply 时核对学生,不影响方案与任务拆分。)

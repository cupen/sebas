# add-agent-skills Design

## Context

本届决策全部来自 grill-me 九问收敛（见 proposal 与对话记录），此处只补技术实现
选型。关键现实约束：

- **仓即文件系统**：`~/.agents/skills/` 是唯一真源，无索引、无 DB。sebas 既不监听
  也不缓存它，读=现扫盘。
- **backend 方言已实证（本机 2026-09-14）**：claude code 与 codex 的 skills 目录都
  是 agentskills 同构（`SKILL.md` + frontmatter；codex 多一个*可选*的
  `metadata.short-description`，通用解析不受影响）。gemini 在 `~/.gemini/` 下**没
  有** skills 目录，疑似无此概念。
- **删除语义需要记忆**：spec 里「同步会把仓里已不存在、但上次投影过的条目从
  backend 删掉」，与「无 manifest、无状态文件」的原始减法相冲——不记住上次投过什
  么就无法做这个删除。这是设计中唯一一处必须显性加回的状态。

## Goals / Non-Goals

**Goals:**
- reconcile 内核放 core crate（`src/skills.rs`），CLI 与 webui 两个调用方共用。
- webui `/api/skills*` 沿用 provider 管理面先例（webui-only API，不碰 router）。
- 方言表是代码里的静态映射，可在不改 config 的情况下扩 backend。
- 投影删除所需的「上次投影名册」只存最少状态。

**Non-Goals:**
- 远程节点交付（node-link 接入）——spec 已明写，不重复。
- gemini skill 支持——见 Decision D3。
- 同步结果长期留痕（报告只在响应体内呈现一次）。

## Decisions

### D1: dialect 表 = 静态映射 + 同构默认

```rust
// src/skills.rs
struct Placement { backend_kind: &'static str, dir: PathBuf, convert: Convert }
enum Convert { Identity, /* 将来: GeminiCommands, … */ }
```

本期表内两条：`claude → ~/.claude/skills (Identity)`、`codex → ~/.codex/skills
(Identity)`。已实证同构，一律 Identity 整目录拷贝，frontmatter 逐字保留。
`Convert` 枚举现在就留出，但本期只有 Identity 一个变体——为将来某个真异构
backend 准备的扩展点，不为它写任何转换代码。

**Alternatives**：把落点表做成 config `[skills.placement.<kind>]` 允许用户覆盖——
否决，YAGNI；backend 目录约定是上游产品事实，不该被用户改。等出现同一 backend
不同安装路径的真实诉求再说。

### D2: 投影删除所需的记忆 = 每 backend 一份 `.sebas-projection.json`

这是唯一的隐性状态加回，reconcile 三分支变成四分支，全部幂等：

```
对 backend 目录 B 与仓 S:
  for name in S:            写 B/name（同名覆盖, 仓 wins）→ 记入新名册
  for name in 旧名册 - S:   删 B/name                     → 从新名册剔除
  for name in B - S - 旧名册:  不动 (用户私产)
  写完原子替换 B/.sebas-projection.json (tmp+rename)
```

manifest 只记 `{"projected": ["beads","my-deploy"], "store_hash": "…"}`，不记内容
hash（覆盖与否由「仓 wins」语义决定，不用比较）。manifest 丢失/被删 → 退化为只投影
不删除（名外全算私产），属安全方向，无需修复流程。

**Alternatives**：(a) 全局一份 manifest 记所有 backend——否决，backend 目录被用户
整体删掉时全局名册会指着不存在的目录，恢复逻辑无谓复杂；(b) 真不记名册、永不删
backend 条目——否决，spec 已承诺删除语义（用户从 webui 删 skill 的直觉就是
「哪都没了」）；(c) 目录级钩性（只删名字带前缀的）——否决，污染 skill 命名空间。

### D3: gemini = NoPlacement（如实报告），不降级为 commands TOML

`~/.gemini` 里无 skills 概念；gemini CLI 有 TOML 形式的 custom commands。把一个
agentskills skill「编译」成 gemini TOML 是**有损、猜测语义**的转换（SKILL.md 是
给模型的说明文档，不是命令），本期不做。方言表查无 gemini → spec 定义的
「NoPlacement 如实报告」路径生效。

**Open question → task**：方言矩阵调研（tasks 1.x）把 gemini / opencode 的真实
skill 消费方式钉实。若调研发现 gemini 真有官方 skills 目录且同构，D1 表加一行即
可，不改架构。

### D4: 存储在文件系统，API 是磁盘操作的薄壳

- `GET /api/skills`：扫仓 → `[{name, desc, attachments, valid, reason?}]`
- `GET /api/skills/:name`：返回 SKILL.md 原文 + attachment 列表（前端渲染
  markdown，后端不渲染）
- `DELETE /api/skills/:name`：`rm -rf <store>/<name>`——**只删仓，不动 backend
  目录**；backend 侧的对应删除交给下一次 sync（与 spec 一致）
- `POST /api/skills/sync`：跑 reconcile，返回每 backend 的
  `{written:[], overwritten:[], deleted:[], private_ignored: n, no_placement:[]}`

CLI 与 webui 同源逻辑：CLI 直接调 core 的 `skills` 模块（同进程），webui 走 HTTP。
`sebas skills add` 的三源（local dir / git / npx）也由 core 模块实现，CLI 是薄壳。

### D5: config 只加一行的可选覆盖

```toml
[skills]
dir = "~/.agents/skills"   # 可选, 默认即此
```

不加 `enabled` 开关（目录缺失就当空仓处理，不是错误）；不加 per-backend 开关
（spec 语义是「投影到所有有落点的 backend」）。

## Risks / Trade-offs

- **`sync` 覆盖用户手改** → 这是 spec 明定的「仓 wins」语义，不是 bug；但响应里
  `overwritten` 列表必须如实呈现，让用户能看见被覆盖的名字，可恢复手段是用户的
  git 习惯（仓本身建议 git 管理，README 提一句即可，不强制）。
- **manifest 与 backend 目录漂移**（用户手改 B 又把 manifest 删了）→ 退化为「只
  加不删」，安全方向；下次 sync 会重建名册，自动收敛。
- **`sebas skills add <git-url>` 与 `npx skills add` 都依赖外部工具在场** →
  探测 `git`/`npx` 是否可用，不在场直接报错明说缺什么，不兜底假装成功。
- **webui delete 后用户忘了 sync，backend 里还有那份旧 skill** → 语义上 backend
  目录本来就不是真源，agentskill 被多留一会儿无害；webui 删除确认文案里提一句
  「backend 目录将在下次 sync 时清理」。

## Migration Plan

无存量数据、无 breaking 变更。config 不增 `[skills]` 段即走默认路径；首启不
做任何自动写盘。

## Open Questions

（无——gemini 的真实消费方式归进 tasks 的调研任务，结论只影响 D1 表加不加一行，
不动 specs / 不动本设计的任何决策。）

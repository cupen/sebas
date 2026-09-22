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
  （**勘误（tasks 1.1 调研后）**：gemini CLI 已有官方 skills 机制且同构，本机
  `~/.gemini/` 整个目录不存在只说明本机没初始化过 gemini，不构成「无此概念」的
  证据——见文末「方言矩阵调研」。NoPlacement 是本期取舍，不是事实判断。）
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

**Open question → 已结（tasks 1.1 / 1.2 调研落盘）**：方言矩阵调研见文末
「方言矩阵调研」小节。结论：gemini 与 opencode **都**有官方 agentskills 同构的
skills 目录；但本期 D1 表仍只 claude/codex（spec 只承诺 claude 的 placement），
加行留给后续 change——届时各加一行 `Placement` 即可，不改架构。

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

### D6: 输入框可见性 = agent 广告域，sebas 不做仓来源的会话内提示

技能投影的契约终点是 backend 目录；操作者在 composer 里能否「看见」技能，归
agent 自身广告域，不由本变更提供（review 时操作者实测「已加载的技能在输入框
没有任何提示」，此节即该缺口的裁决）：

- **claude**：skills 以 slash commands 形态被 claude code 自身暴露，经
  `session-slash-commands` 的命令面板自然可见——数据源是 initialize 握手的
  `commands` 数组（会话域真相），不是本仓。快照语义要如实告知：cc-agent-sdk
  无 commands_changed 回调，命令表是 spawn 时刻快照，**sync 后旧会话看不到
  新技能，新会话才见**。
- **opencode**：skills 是上下文材质而非命令，其 `available_commands_update`
  永不含技能——面板不列是如实语义，不是缺陷（两条 intake 的刷新语义不对称：
  ACP 可中途重推刷新，claude 只在重连时）。
- **否决「仓来源」提示**（输入框徽标「已加载 N 个 skill」、仓 typeahead）：
  投影是手动的，仓不知道某个会话的 agent 实际加载了什么，这类提示是会话
  谎言——正是本仓到处在消灭的假 UI。
- **否决独立命令查询端点**：composer 是响应式消费，会话载荷字段 + 既有事件
  流已覆盖（`session-slash-commands` 2.2 已实现），无第二调用方，YAGNI。

**实证（6.4，2026-09-16）**：claude v2.1.236 的 initialize `commands` 数组
**确实含 skills 条目**——沙箱把 `HOME` 钉进零凭据临时目录、置 marker skill 于
`~/.claude/skills/`，按 cc-agent-sdk 同款 wire 帧（stream-json 双向 + stdin 发
`control_request{subtype:"initialize"}`）直接驱动真 CLI，应答 commands 首条即
该 skill：`{"name":"z-visibility-probe","description":"… (user)",
"argumentHint":""}`——用户级 skill 的 description 带 `(user)` 后缀、argumentHint
恒空。凭据问题一并实证：**握手先于任何 turn、不需凭据**（零凭据 HOME 下
initialize 正常应答、stderr 干净、无 onboarding 门）。claude 面板可见性成立。
附带观察：同沙箱走 sebas 全链路 spawn 时，无上游凭据则首回合必然失败、会话
随即移除——属「子进程诚实死亡」，非通道故障；上表证据取自 wire 级直驱。

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

## 方言矩阵调研（D3 注释，tasks 1.1 / 1.2 实证落盘 2026-09-14）

### gemini CLI —— 有官方 skills 机制，且与 agentskills 同构

- **官方文档**（Get started with Agent Skills | Gemini CLI,
  https://geminicli.com/docs/cli/tutorials/skills-getting-started/ ）：gemini CLI
  按 agentskills 开放标准发现 skill——个人级 `~/.gemini/skills/`（不受 trust 门控）、
  项目级 `<workspace>/.gemini/skills/`（需 workspace 被 `/trust` 标记），且把
  `.agents/skills` 作为**别名**一并发现。格式=目录 + `SKILL.md`：frontmatter 必须含
  `name` 与 `description`、`---` 必须是文件第一行，缺任一即被静默跳过（与本仓
  `scan_store` 的 invalid 标记策略不同——我们宁可显式报 invalid）。另有自带的
  `gemini skills install <url-or-path>` / `gemini skills link <path>` 安装命令。
- **custom TOML commands 是另一套机制**（`~/.gemini/commands/*.toml`），与 skills
  无关——无需把 skill 降级编译成 TOML，D3 的「不做有损转换」结论维持，但理由从
  「gemini 没有 skills」更新为「gemini 有同构目录，直接投影即可（后续 change）」。
- **本机旁证（只读）**：`~/.gemini/` 整个目录不存在（本机未初始化 gemini CLI），
  无目录级证据可取，结论以官方文档为准。
- **结论**：`gemini → ~/.gemini/skills (Identity)` 具备加表条件。**本期不加表行**
  （spec 只承诺 claude 的 placement），NoPlacement 路径照旧。

### opencode —— 有 agentskills 同构的 skill 目录（旧注「无目录证据」就此更新）

- **官方文档**（Agent Skills | OpenCode, https://opencode.ai/docs/skills/ ）：
  六处发现路径，每处都是「一 skill 一目录 + `SKILL.md`」——项目级
  `.opencode/skills/<name>/SKILL.md`、`.claude/skills/<name>/SKILL.md`、
  `.agents/skills/<name>/SKILL.md`；全局级 `~/.config/opencode/skills/<name>/SKILL.md`、
  `~/.claude/skills/<name>/SKILL.md`、`~/.agents/skills/<name>/SKILL.md`。格式同
  agentskills：frontmatter 必须含 `name`/`description`，且 `name` 须与目录名一致
  （`^[a-z0-9]+(-[a-z0-9]+)*$`）。
- **本机旁证（只读）**：`~/.config/opencode/` 顶层为 `node_modules` /
  `opencode.jsonc` / `package-lock.json` / `package.json`，无 `skills/` 子目录——
  本机装了 opencode 但还没装任何 skill，与文档不冲突（那个目录是装了才有的）。
- **结论**：同构目录存在，加行条件成立；**本期同样不加表行**。附带实证收益：
  `~/.agents/skills`（本仓默认 store 路径）与 `~/.claude/skills` 本身就在 opencode
  的全局发现路径里——opencode 用户对投影零依赖也能读到材质。

### npx skills CLI（`sebas skills add <pkg>` 所封装的社区方案）

- **出处**：vercel-labs/skills（https://github.com/vercel-labs/skills ，索引站
  https://skills.sh/ ）。`npx skills add <source>` 接受 GitHub shorthand（
  `owner/repo`）、git URL、本地路径、SKILL.md 直链与压缩包；相关旗标：`-g`（装到
  用户目录而非项目）、`-a <agents…>`（选目标 agent）、`-s <skills…>`（选 skill）、
  `--copy`（拷贝代替符号链接）、`-y` / `--yes`（跳过交互）、`--all`。
- **落盘位置不可控**：没有「指定安装目录」的旗标，目的地由它按本机 agent 检测
  自行决定（项目 `./<agent>/skills/` 或用户 `~/<agent>/skills/`）。因此
  `add_from_npx` 只实现到「命令执行成功」层、不把结果收编进 store（`store` 参数
  为签名一致保留）——如实降级，6.2 手测兜底。

## Open Questions

（无——原「gemini 的真实消费方式」已由 tasks 1.1 / 1.2 调研钉实并落盘于上方
「方言矩阵调研」：gemini / opencode 均有官方同构 skills 目录，本期不加表行，
后续 change 各加一行 `Placement` 即可，不动任何决策。）

# design — review-add-state-store

## Context

评审对象：`openspec/changes/add-state-store/`（2026-09-01 规划，实现未动，依赖 `add-core-session-channel` 已归档）。评审时点的新 reality：webui 主控（`make-feishu-optional-webui-primary` 已归档）、三个 ACP change（`add-opencode-acp` 13/16、`add-acp-session-id-mapping` 规划完、`add-acp-model-selection` 规划完）。详见 proposal.md 的 Findings。

本 change 是**评审决议**（不改代码），产出整改项清单，供后续修订 `add-state-store` 及 ACP change 时执行。

## Goals / Non-Goals

**Goals:**
- 确认 `add-state-store` 收编契约在新 reality 下的成立性
- 产出**可执行**的整改项（归属哪个 change、改什么），不留"待议"悬空

**Non-Goals:**
- 不实现 state-store / ACP change 的代码
- 不代改 `add-state-store` 的 artifacts（整改项由后续修订执行）
- 不做数据迁移、不动运行系统

## Decisions（评审决议）

### R1 sessions 表字段契约：必须在 state-store spec 中定义（high）

**现状**：`add-acp-session-id-mapping` 要在 session 记录加 `acp_session_id`（MappingDto `#[serde(default)]`），`add-acp-model-selection` 要加 `current_model`；state-store 的 sessions 表定义只在 tasks 3.1 一句带过（spec 未锁列）。两 ACP change 若先于 state-store 落 SQLite sessions 表，字段会丢。

**决议**：state-store 修订时在 spec 里明确 sessions 表契约（至少）：
- `session_id`（PK，路由 id / mapping key）
- `key`（web/feishu 会话 key，可空或另表）
- `last_active_unix`
- `acp_session_id`（nullable；ACP agent 真实会话 id）
- `current_model`（nullable；ACP 会话当前模型）

ACP change 先落时，MappingDto 的新字段就是 SQLite 列的来源；state-store 迁移 SQL 必须包含这些列（**属载体切换，见 R4**）。

### R2 settings 收编边界：core 收编、webui 主控读写（medium）

**现状核实**（修正初评）：`settings.json` **真实存在**（`sebas-router/src/settings.rs`，full-snapshot 语义——首写固化 TOML 默认，之后文件整体权威），run.rs 启动 merge 进 `card` 配置。D8「settings.json 收编」有依据。

**决议**：
- settings 表收编 `CardConfig` 运行时设置（继承 full-snapshot 语义：写入即整体替换，删除即回落 TOML bootstrap）
- **权威归 core 状态库**（单写者），但 **webui 主控经通道读写**（webui 设置页 → `state.settings.*` 方法），替代 D8 的「router 进程内读写」——router 仍直调（in-process），但 webui 不再有第二条文件路径
- TOML `[card]` 保持 bootstrap 角色，不进库

### R3 归档顺序更新（low）

原「console → channel → state-store → workbench」更新为：**ACP 三 change（opencode → mapping → model-selection）→ state-store → workbench**。state-store 最后收编，使 sessions 表覆盖 ACP 持久化需求，避免 ACP change 在 SQLite 落地后补字段。

### R4 迁移语义澄清：内容不迁、载体必迁（medium）

proposal Non-goals「不做 legacy JSON 数据迁移」需精确化：
- **内容不迁**：state.json 的 mode/default、providers.json 的 provider 数据等**内容**不导入新库（开发阶段重配，沿用 D5）
- **载体必迁**：`MappingDto` 的 session 映射（key → session_id + acp_session_id + current_model）**必须**从 JSON 迁到 SQLite sessions 表——这是收编核心，session 持久化不能丢。若 MappingDto 已是 state.json 的一部分且 state.json 内容不迁，需明确 session 映射是"结构迁移"而非"内容迁移"

## Risks / Trade-offs

- [整改项若不被执行，state-store 落地丢 ACP 字段] → 整改项落进 tasks，标注归属；实施时按 R3 顺序执行
- [settings 权威改 webui 增加通道往返] → 设置低频，可接受；单写者在 core 保证一致性
- [R4 的"载体必迁"与 proposal 的 jargon 冲突] → 修订 proposal 的 Non-goals 措辞，消除歧义

## Migration Plan

无代码迁移（评审决议）；整改项的执行 = 修订 `add-state-store` 的 specs/design/tasks + 在 ACP change 的 tasks 标注字段衔接。

## Open Questions

- `settings` 表是否需要版本化（多档卡片配置）还是单记录：本期单记录（全量快照语义），多档留待 workbench
- sessions 是否分离 `key` 与 `session_id`（一对多：同一会话多入口）：由 ACP mapping change 实施时按 mapping 结构定，state-store 表定义预留
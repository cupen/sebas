# review-add-state-store

## Why

`add-state-store` 规划于 2026-09-01，其 design 依赖的 `add-core-session-channel` 已归档、但 webui 主控、三个 ACP change（opencode/mapping/model-selection）随后落地。评审需确认：state-store 的收编契约（settings 边界、sessions 表字段、MappingDto→SQLite 迁移范围、归档顺序）在**新 reality** 下是否仍然成立、是否与新 change 冲突。

## Findings（评审结论）

**方向仍成立**：三 JSON 的问题（陈旧快照、providers 双写者）原样存在，SQLite 单写者 + core 权威的收编方向正确。

**需整改的缝隙（按严重度）**：

1. **sessions 表字段契约未定义（high）**。`add-acp-session-id-mapping` 要在 session 记录加 `acp_session_id`、`add-acp-model-selection` 加 `current_model/available_models`——而 `add-state-store` 的 sessions 表定义只在 tasks 3.1 一笔带过、spec 未锁定列。**两个 ACP change 若先落 SQLite sessions 表，字段会丢**。需在 state-store spec 里明确 sessions 表契约：`session_id`（PK）、`key`、`last_active_unix`、`acp_session_id`（nullable）、`current_model`（nullable）等。

2. **settings.json 收编边界存疑（medium）**。design D8 说"settings.json(CardConfig) 进入 settings 表"、"旧文件不迁移"——但代码里 `CardConfig` 来自 `[card]` TOML 配置（`cfg.card`），**不是独立 JSON 文件**。settings 表到底收什么（运行时动态设置 vs 静态配置）必须厘清；若 CardConfig 是静态配置，则不该进状态库，需从 D8 撤出。

3. **settings 权威在新 reality 下应归 webui（medium）**。design D8 说"router 进程内读写 settings"，但 webui 已是主控（make-feishu-optional-webui-primary 归档），设置页要读 settings——**权威归属需按 webui 主控重写**（webui 经通道读、core 写，或反向）。

4. **MappingDto→SQLite 迁移范围需显式（medium）**。proposal Non-goals 说"不做 legacy JSON 数据迁移"——但要区分：**不做旧 JSON 内容迁移**（state.json 的 mode/default 等）vs **必须做载体切换**（MappingDto 的 session 映射从 JSON 迁到 SQLite sessions 表，否则 acp-* 的 resume/mapping 持久化会丢）。后者是收编核心，不能归入 Non-goals。

5. **归档顺序需后移（low）**。原顺序"console → channel → state-store → workbench"应更新为：ACP 三 change（opencode → mapping → model-selection）先于 state-store，使其 sessions 表字段覆盖 acp-* 的持久化需求；state-store 最后收编。

## What Changes

- 本 change **不改代码**，只产出评审决议（proposal/design/tasks），并指名整改项归属：修订 `add-state-store` 的 specs/design/tasks，以及（若 ACP change 先落）`add-acp-session-id-mapping` / `add-acp-model-selection` 与 state-store 的字段衔接。

## Capabilities

### New Capabilities

- （无 new spec——本 change 为评审决议，`skip_specs: true`）

### Modified Capabilities

- （无 spec 级行为变更——整改项由后续对 `add-state-store` / ACP change 的修订落地）

## Non-goals

- 不实现 state-store 任何代码（评审只立决议）
- 不代改 `add-state-store` 的 artifacts（整改项列出，修订由后续动作执行）
- 不做数据迁移、不动运行系统

## Impact

- `add-state-store`（specs/design/tasks 待按 Findings 修订）
- `add-acp-session-id-mapping` / `add-acp-model-selection`（sessions 字段契约衔接）
- 归档顺序（ACP 三 change 先于 state-store）
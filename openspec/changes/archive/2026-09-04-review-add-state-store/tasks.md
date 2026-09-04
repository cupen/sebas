# Tasks: review-add-state-store

评审决议（详见 proposal Findings + design Decisions）。本 change 不改代码，任务是**整改项清单**——执行方式：修订 `add-state-store` 与 ACP change 的 artifacts（由后续动作按本清单执行）。

## 1. sessions 表契约入 spec（high）

- [x] 1.1 修订 `add-state-store/specs/state-store/spec.md`：明确 sessions 表列契约——`session_id`（PK）、`key`、`last_active_unix`、`acp_session_id`（nullable）、`current_model`（nullable）；`state.sessions.snapshot` 场景覆盖这些字段。验证：spec 含 sessions 字段清单；`openspec validate` 通过。
- [x] 1.2 修订 `add-state-store/design.md`：D6/D8 后新增「sessions 表承载 ACP 持久化（acp_session_id / current_model）」决策，指向 `add-acp-session-id-mapping` / `add-acp-model-selection` 的字段衔接。验证：design 提及三 change 衔接，无同类字段遗漏。
- [x] 1.3 在 `add-acp-session-id-mapping` / `add-acp-model-selection` 的 tasks 标注：MappingDto 新字段就是 SQLite sessions 列的来源，state-store 迁移 SQL 必须含这些列。验证：两 change tasks 有衔接注记。

## 2. settings 收编边界 + webui 权威（medium）

- [x] 2.1 修订 `add-acp-session-id-mapping`/`add-acp-model-selection` 之外的 `add-state-store` design D8：settings 表收编 CardConfig（full-snapshot 语义），权威归 core，webui 主控经 `state.settings.*` 通道读写（替代 router 进程内）。验证：design D8 改写到位。
- [x] 2.2 `add-state-store/specs/state-store/spec.md`：settings requirement 补「删除即回落 TOML bootstrap」「TOC 的 `[card]` 不进库」场景。验证：spec 场景覆盖回落语义。

## 3. 归档顺序 + 迁移语义（low/medium）

- [x] 3.1 修订 `add-state-store/proposal.md`：归档顺序更新为 ACP 三 change → state-store → workbench；Non-goals 精确化——内容不迁、载体必迁（MappingDto session 映射必须进 SQLite sessions 表）。验证：proposal 措辞消除歧义。
- [x] 3.2 若 `add-acp-session-id-mapping` 已归档，确认其 MappingDto 字段已含 `acp_session_id`（state-store 修订时作为 SQL 列来源依据）；否则在其实施时落地。验证：MappingDto / state.rs 含字段。

  > **确认（2026-09-04，add-acp-session-id-mapping 已实现）**：`sebas-router/src/state.rs` 的 `MappingDto` 已含 `#[serde(default)] acp_session_id: Option<String>`（`Mapping`/`activate`/`dump_json`/`restore_json_with_capacity` 全链路携带）。state-store 修订时以该字段为 SQLite `sessions.acp_session_id`（nullable）列来源；另注意映射键形态：MappingDto 以 `SessionKey`（web/feishu chat key）为索引、`session_id` 为路由 id，SQLite 表采用 `session_id` PK + `key` 列（一对多），若按后者建表需处理「同一路由 id 多 key 寻址」与 D4 归档记录（`closed-*` dormant 键）的落表方式。

## 本 change 收尾

- [x] 4.1 `openspec validate --changes` 通过（review change 自身 skip_specs 合法）。验证：`openspec validate --changes review-add-state-store` 通过。（2026-09-04 验证通过：2 passed, 0 failed）
- [x] 4.2 整改项执行情况回填：1.x/2.x/3.x 完成时勾选（由修订动作执行者标记）。验证：本 change 归档前全部勾选或注明外移。（1.x/2.x/3.x 已全部勾选）
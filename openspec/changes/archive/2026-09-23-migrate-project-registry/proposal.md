# Proposal: migrate-project-registry

## Why

项目注册今天有**两份存储**：`projects` 表是本地项目的权威，而 `projects.json` 是 webui 自持的第二份；关键在于**远程节点项目只存在于文件里**——表里连 `node_id` 列都没有（`sebas-webui/src/api.rs:1560-1563` 记录了这一点）。两个后果：

1. **远程项目不参与任何库级的备份、迁移与一致性检查**；`projects.json` 是 webui 进程局部写出的文件，随 webui 部署形态漂移（内嵌于 core 还是 standalone 各走各的）。
2. **两个存储可以静默漂移**：本地项目的增删改走库，远程条目与分支缓存走文件，没有事务覆盖两者。

还有一处与既有 spec 相悖：core 不可达时 standalone webui 会**回退到文件并把文件派生值当作当前状态呈现**（`api.rs:1569-1590`），而 `state-store`「Unavailable store degrades honestly」要求「present an explicit unavailable state naming the cause… MUST NOT… present stale snapshots as current」。项目的诚实降级面今天没遵守这条。

迁移的落点已具备：`projects` 表已注册且已是本地项目的权威，state store 的 projects CRUD 方法已在 channel 上，webui 已有 channel 客户端。而且**产品尚未发布**，所以这次可以顺手把两个并行形状（`ProjectRow` 存储形状 / `ProjectEntry` 线形状）合成一个——原计划因「合并会改 schema 与线格式、需要迁移路径」而把它推迟，该约束现在不存在了。

## What Changes

- **`projects` 表按目标形状重建**：加入 `node_id`（承载节点维度，本地项目为 `local`），不再受「只能加列」束缚；无发布版，旧库重置一次即可。落点是 `projects.db`（与 `session_map` 同库；分层规则见 `single-state-dir`）。
- **`ProjectRow` 与 `ProjectEntry` 合一**为一处规范定义（存储与线格式同一形状）：删掉两处并行的字段清单与它们之间的约定式 JSON 搬运，并配一个「形状钉」测试。
- **项目注册统一由 core 状态库承载**：本地与远程项目同库，全部读写经 state 方法；standalone webui 不再自持文件。
- **退休 `projects.json` 的写入与读取回退**：不可达时呈现 unavailable 状态并给出 cause，而不是文件派生值。
- **不做遗留导入**：无发布版，没有需要搬运的既有安装；文件留在盘上可随手删除。
- **退休 `SEBAS_PROJECTS_PATH`**（由 `single-state-dir` 的映射表接管逻辑名）。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `workspace-root`: 新增「项目注册持久化在 core 状态库」要求——注册表含节点维度、由 core 独占写入、其他角色经 state 方法读写；存储不可达时呈现 unavailable 而不是文件派生值；不再有独立的项目注册文件。并新增「注册记录只有一个规范形状」要求——存储与线格式共用一处定义，字段增删不能在一侧静默漂移。

## Impact

- **改动**：`src/sebas_state/repo.rs`（`ProjectRow` 重建 + `node_id`）、`sebas-webui/src/projects.rs`（删文件读写、改经 state 方法）、`sebas-webui/src/api.rs`（远程条目的读改写路径与回退分支）、`src/core_channel/server.rs`（projects CRUD 的节点维度）、`tasks.py` / `AGENTS.md`（`SEBAS_PROJECTS_PATH` 退休）。
- **测试面**：`tests/testsuite-webui/tests/helpers/detached.ts` 与相关 Playwright journey 钉了 `projects.json`；`tests/support/mod.rs` 的路径钉要改。
- **依赖**：只依赖 `single-state-dir`（路径逻辑名接管）。**不再依赖 schema 加固**——原计划排在 `harden-schema-migration` 之后是怕「改真实库不可调和即删库」，而现在本 change 主动接受一次重置，且 `quarantine-database-reset` 会让那次重置留下可恢复的痕迹。
- **schema**：`projects` 重建（含 `node_id`）。无发布版，故不要求加列兼容；开发机上旧库重置一次。
- **验收**：远程项目重启后仍在（含节点归属）＋ 不可达时呈现 unavailable（不回退文件）＋ 形状钉测试（单侧加字段即红）＋ 既有 webui/Playwright 旅程全绿 ＋ `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。

## Non-goals

- **不动 `archive.json`**：`webui` spec 要求归档「persist to its own file, separate from the project registry and the core state store」，且归档条目含**完整转录**（体积与保留期是独立设计问题）。无发布版意味着这条 spec 要求**可以改**，但体积与保留期仍需独立设计，故仍留作后续 change。
- **不改项目 id / 分支缓存 / `default_agent` 的语义**——只换存储位置与增加节点维度。
- **不做遗留导入**、不保留旧 `projects.json` 的可用性。
- **不为远程项目引入新的生命周期语义**（离线节点的项目可见性规则不变）。
- **不改项目列表的对外形状**：合并两个形状后字段集不变，`node_id` 的序列化拼写与默认值（`local`）保持不变，前端无需改动。

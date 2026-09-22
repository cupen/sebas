## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **表已存在、缺一列**：`projects` 表（`repo.rs:588-603`）列为 `path, name, branch, branch_at, added_at, sort_order, id(UNIQUE), default_agent`，**没有 `node_id`**；主键是 `path`。
2. **两个并行形状**：`ProjectRow`（存储，`repo.rs:335`）与 `ProjectEntry`（线，`sebas-webui/src/projects.rs:16`）字段几乎一一对应但**不一致**——`ProjectEntry` 有 `node_id`（默认 `local`）无 `sort_order`，`ProjectRow` 反之；两者之间从来没有 `impl From`，全靠 `serde_json::Value` 往返（`src/sebas_state/engine.rs:61,73`）与约定式键名。
3. **文件里的东西是「本地没有的」**：本地项目以库为权威；`projects.json` 独家持有远程节点条目，并且是 core 不可达时的回退源（`api.rs:1569-1590`）。
4. **webui 两种部署形态**：内嵌于 core 与 standalone，**两者都必须走 state 方法**；projects CRUD 已在 `state-store`「State methods on the core channel」里，本 change 不引入新的通道能力。
5. **诚实降级已是既有要求**：`state-store`「Unavailable store degrades honestly」——项目的文件回退是这条要求在 webui 项目面上的**未被执行**，不是新设计。
6. **产品尚未发布**：这是本 change 可以重建表、合并形状、不做导入的依据。

## Goals / Non-Goals

**Goals:**

- 让「项目」只有一个权威、一个形状，且远程与本地同库同事务语义。
- 让项目面遵守既有的诚实降级要求（今天没有）。
- 顺手清掉一处并行形状——这是 `add-domain-layer` 立下的规则（同一概念只定义一次）在项目记录上的落实。

**Non-Goals:**

- 不动 archive.json（见 proposal Non-goals）。
- 不改项目 id 语义、分支缓存策略、离线节点可见性规则。
- 不做遗留导入。

## Decisions

### D1 表按目标形状重建，`node_id` 是正式列

`projects` 表重建为承载节点维度的形状：`node_id` 为正式列（本地项目取 `local`），列可声明 `NOT NULL` 并带默认值。**理由**：无发布版，重置一次的成本远低于「为兼容把每个新列都做成可空」的长期包袱。**被否备选**：只加可空列（原计划）——那会在每个读取处留下「None 代表旧行」的隐性分支。**被否备选**：新建 `remote_projects` 表——两个表就要两处查询与两次事务，而本 change 的目的正是让注册表只有一处。

### D2 `ProjectRow` 与 `ProjectEntry` **合一**为 ActiveRecord struct（原计划推迟，现在做）

合并为一处规范定义——它同时就是 `projects` 表的 ActiveRecord struct（`#[derive(ActiveRecord)]`，`project.save(&store)` 即持久化），并经 serde 承载线形状；删除两处并行的字段清单与它们之间基于 `serde_json::Value` 的约定式搬运。

**为什么现在可以**：原计划把它推迟的理由是「合并会改 schema 与线格式，需要版本 bump 与迁移路径」。无发布版 → 该约束消失。**为什么值得**：这是本批计划的一条主线（`add-domain-layer` 的「同一概念只定义一次」），而项目记录正是最典型的一处——两处字段清单已经**实际不一致**（Context 2），且没有编译器能发现漂移。ActiveRecord 让「存储即实例、实例即可存」，规范形状与持久化形状不再是两个东西。

**归属**：struct 定义在 `sebas-models`（core 各表的 ActiveRecord 之家，见 `extract-sebas-db` D2），webui 经 channel 消费其 serde 形状；`sebas-node` 不依赖它（依赖图无 SQLite）。

**边界**：合并**只到规范记录**；展示层计算字段（如 `status_label` 那类）仍由转换产出，不进记录本身（spec 场景「presentation-only fields stay out of the record」）。**风险**：`node_id` 的序列化拼写与默认值必须保持 `local`，否则前端可见形状变化——用形状钉测试锁住（spec 场景「a local project keeps its existing serialized spelling」）。

### D3 **不做遗留导入**（取消原方案）

无发布版，没有需要搬运的既有安装。原方案的「文件在、库中缺该 id → 导入该条」连同其去重规则、标记键、四条测试一并取消。旧文件不再被读取，留在盘上可随手删除。**被否备选**：保留导入（只为让开发机的远程项目列表不为空——那不值得一套导入机制）。

### D4 依赖：只需 `single-state-dir`；不再需要 schema 加固

原计划硬依赖 `harden-schema-migration`，理由是「不在数据安全加固之前改用户真实库」。既然本 change 主动接受一次重置，该依赖消失。仍建议排在 `quarantine-database-reset` 之后**（软依赖，非必需）**，好处是那次重置会隔离旧库、留下可恢复痕迹，便于开发机找回项目列表。

### D5 诚实降级按既有要求执行，不发明新语义

不可达 → `unavailable` + cause；变更入口置灰；恢复后重新读库。这三点是 `state-store` 既有要求的直接适用，因此本 change 只补一条针对项目面的要求。**取舍**：既有的「core 不可达也能看项目列表」体验会消失——这是刻意的（呈现陈旧注册表比呈现不可用更危险），写进 spec 场景。

### D6 archive.json 仍留作后续，但 spec 障碍已降低

`webui` spec 要求归档「persist to its own file, separate from the project registry and the core state store」。无发布版意味着这条要求**可以直接改**，不再是障碍；但归档条目含完整转录，体积、保留期与清理策略仍需独立设计，所以它与项目注册不同批。**触发条件**：需要库级查询/备份历史转录时。

## Risks / Trade-offs

- **[开发机旧库被重置，项目列表清空一次]** → 接受（无发布版）；`quarantine-database-reset` 落地后那次重置会隔离旧库、可手工恢复。
- **[合并形状改变线格式]** → `node_id` 的拼写与默认值锁在形状钉测试里；前端断言（`tests/testsuite_webui` 的 API 形状用例）必须不改而通过——**若前端需要改动，说明合并越界了**。
- **[失去「core 不可达也能看项目列表」]** → 取舍见 D5；spec 场景明确该行为。
- **[standalone webui 的写路径]** → 两种部署形态都必须经 state 方法；用 standalone 拓扑的旅程验收，不能只测内嵌形态。
- **[Playwright 旅程钉了 `projects.json`]** → 明确列出改造清单（`tests/testsuite-webui/tests/helpers/detached.ts` 等），改写为经 API 断言，不删覆盖点。
- **[合并改动面较大（两处定义 + 全部构造点）]** → 编译器兜底：形状合一后未更新的构造点直接编译失败；分步提交（先建规范定义与转换，再切两侧，最后删旧）。

## Migration Plan

1. `projects` 表按目标形状重建（含 `node_id`）；开发机旧库重置一次。
2. 建立规范项目记录定义，合并 `ProjectRow` / `ProjectEntry`，把 `serde_json::Value` 往返替换为显式转换；加形状钉测试（存储 + 线两侧字段名与拼写）。
3. 远程项目的读改写路径改走 state 方法（core channel 的 projects CRUD 补节点维度）；本地路径不动。
4. 删 `projects.json` 的写入与读取回退；项目面改为不依赖文件。
5. 退休 `SEBAS_PROJECTS_PATH`；改 Playwright 助手与路径钉；更新 `tasks.py` / `AGENTS.md`。
6. 全量回归（含 standalone 拓扑与 Playwright 旅程）。

**回滚**：分步提交，每步可 revert。第 1 步（表重建）回滚需重建旧形状——旧库已被隔离（`quarantine-database-reset`）或已被重置，均不涉及数据丢失保障。无遗留导入，因此不存在「旧文件被改坏」的风险。

## Open Questions

- `node_id` 是否需要参与唯一性约束（同一路径能否同时出现在本地与某节点上）：今日 `projects` 主键是 `path`，语义是「一个路径一个项目」。若日后需要同一路径多节点放置，需要重画主键——独立评估。
- 合并后是否需要为「线形状」保留独立的 DTO 以隔离未来演进：今天不需要（无发布版、前后端锁步）；若日后出现第三方客户端，再评估。

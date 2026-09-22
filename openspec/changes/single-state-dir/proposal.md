# Proposal: single-state-dir

## Why

两件事合在一起，因为它们由同一张「逻辑名 → 落点」表决定：

**（一）路径分散，且有两处今天钉不住。** 每个状态文件各自一个环境变量（`SEBAS_STATE_DB` / `SEBAS_ARCHIVE_PATH` / `SEBAS_WEBUI_AUTH_DB` / `SEBAS_PROJECTS_PATH` …，外加 `SEBAS_HOME` 与 `HOME` 兜底）；`AGENTS.md` 把这些列为「all mandatory」，`tasks.py:_sandbox_env` 实钉 8 个。**漏钉一个就写进操作员的真实 `~/.sebas`。** 更糟的是 `services.json` 的路径硬编码在 `src/watchdog.rs:45-51`（无 env、无配置键），`nodes.json` 只有配置键——也就是说 `services.json` **今天就无法被钉进沙箱**，watchdog 一跑就必然写操作员真实目录。

**（二）数据挤在一个库里，而两类数据的增长特征完全相反。** `sebas.db` 今天同时装着：有界的系统配置（providers、model_aliases、settings —— 它们**本就是设置项**，行数由手写配置决定）与会持续膨胀的用户数据（projects；以及 `persist-session-map` 落地后**按变更写入**的 session_map，未来还会包括会话与消息/转录）。把两者放在一个文件里，意味着：备份要连膨胀的大文件一起拷；VACUUM / checkpoint 的开销落在一起；启用 per-mutation 会话映射后，每一次 card 配置或 provider 读取都骑在一个被用户数据搅动的 WAL 上；而重置用户数据会连设置一起丢。

## What Changes

- **布局改为「一个状态目录 + 按用途分层的库」**：
  - `settings.db` —— `providers`、`model_aliases`、`settings`（card 配置、runtime state 与各类标记）。有界、稀有写。
  - `projects.db` —— `projects`、`session_map`（以及后续的会话与消息）。持续增长、频繁写。
  - `auth.db` —— 保留现状（WebUI 用户库，有界）。
  - `usage.db` —— router 自有（见 `persist-router-usage`），独立进程、持续增长。
- **表按 ActiveRecord 目标形状重塑**（借重建立库之机，一次做对）：`providers` 从 `config TEXT (JSON blob)` 扁平化为类型化列（name / preset / 三个 base_url / api_key 等，一行即一个实例），终结 `Item = Map<String, Value>` 的无类型存储；各表 struct 由 `#[derive(ActiveRecord)]` 声明映射（机制见 `extract-sebas-db`），标准 CRUD 零手写 SQL。
- **分层规则（两级，写进 spec）**：先按**写入进程**（每个文件只有一个写入者），再在 core 内按**增长特征**（有界的系统配置 vs 会增长的用户数据）。`providers`/`models` 归 `settings.db` 正是这条规则的应用——它们行数由配置决定，不会膨胀。
- **路径全部从单一状态目录派生**；逐文件变量保留为显式覆盖（向后兼容沙箱菜谱与部署）。
- **退休 `SEBAS_STATE_DB`**：`sebas.db` 不再存在，其角色由状态目录（派生全部落点）与逐库覆盖变量承担。产品尚未发布，无需为旧文件名保留别名。
- **收编无覆盖的文件**：`nodes.json` 改为在状态目录下解析并可显式覆盖。
- **`services.json` 保留为文件，watchdog 不引入持久层（守护进程定位，记录在案）**：开关命令由 webui 服务页 / CLI / feishu 发起，但终点是 **watchdog 自己的 control RPC socket**——core 不在路径上，是被操作的对象（`ServiceSet` 命名 core 一律被拒）。写入 `services.json` 的因此是 watchdog 自己：并入 `settings.db` 违反「一个文件一个写入者」，且会让监督者的记忆依赖被监督者存活。它的性质是**操作员配置**（与 `config.toml` 同类），越界问题由「从状态目录派生 + 可显式覆盖」解决。详见 design D6。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `state-store`: 「Database location and single-writer ownership」由「单个 SQLite 数据库」改为「一个状态目录下按用途分层的多个数据库，各自单写者」；WAL、单写者、序列化变更、路径展开等既有语义逐条保留，并新增分层规则（有界系统配置 vs 增长的用户数据）与「分层让一个库的重置不牵连另一个」的要求。
- `cli-service`: 新增「状态目录」要求——单一变量派生全部落点、逐文件变量作为显式覆盖、退休 `SEBAS_STATE_DB`、任何状态写入都必须落在派生目录内。

## Impact

- **改动**：路径解析集中一处（逻辑名映射表：逻辑名 → 所属库 → 文件名 → 覆盖变量）；`src/run.rs`、`src/sebas_state/*`（两个库各自的注册表与打开）、`src/watchdog.rs`、`src/node_link/registry.rs`、`sebas-webui/src/{archive,projects,user_store}.rs`、`tasks.py`、`AGENTS.md`。
- **依赖**：`extract-sebas-db` 提供「每库一次 open + 重置策略 + 单写执行」的运行时（本 change 是它的第一个消费者：core 开两个库）；`add-domain-layer` 提供 tilde 展开等原语的单一实现。
- **数据**：无发布版，故**不做任何迁移**——`sebas.db` 只是不再被打开，文件留在盘上由操作员处置；开发机首次启动即创建 `settings.db` 与 `projects.db`。
- **验收**：钉住状态目录后全部落点都在目录内（枚举逻辑名的机械断言）＋ 两个库各自的单写者与独立重置（重置 `projects.db` 不影响 `settings.db`）＋ 沙箱旅程复核真实 `~/.sebas` 未被触碰。

## Non-goals

- **不拆到业务域粒度**：`settings.db` 内不再按 provider / model / card 分文件。分层依据是**增长特征与写入进程**，不是业务域粒度——`providers`/`models` 归 `settings.db` 正是这条规则的应用，而不是对它的违背。
- **不做跨库事务**：若发现某个写事务需要横跨两个库，必须把边界调整为「同一库内」或「最终一致」，而不是引入跨库两阶段提交（见 design D4 的前置核对）。
- **不迁 `services.json` 到 SQLite**（显式例外，见上）。
- **不动 `auth.db`**：它已是独立文件、写入者是 webui；按「一个文件一个写入者」它不应并入 core 的库。
- **不改配置文件的发现顺序**与 `[service.core] channel_path` 等既有语义。
- **不引入 XDG 重排**：默认值收敛到既有的主状态位置，不额外重排目录结构。

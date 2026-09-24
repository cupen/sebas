## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **spec 自相矛盾**（本 change 要消除的正是它）：`session-persistence` 要求「persisted in the state store and written per mutation」；`state-store`「Runtime state boundaries」仍说「currently persisted by the core as a shutdown-only JSON snapshot… the `session_map` table is a reserved placeholder… Migrating the session map into the state store is a deferred design step」。
2. **落点已存在但形状不对**：`session_map` 表已注册（`repo.rs:604-615`）——`chat_id, thread_id, session_id, last_active_unix, project_dir`，主键 `(chat_id, thread_id)`；`load_session_map` / `save_session_map`（`repo.rs:468-514`）已实现且**无生产调用方**。而映射实际携带约 11 个字段（`sebas-dispatch/src/state.rs:1430` 的 `MappingDto`）。
3. **今天的写入最脆**：关停时截断式 `std::fs::write`，无 fsync、无 chmod、非原子（`src/run.rs:590-601`）；读取与文件隔离在 `src/session_boot.rs:304-328`（`<path>.corrupt-<unix>`）。
4. **迁移词表只支持加列**（`migration.rs`）——但**产品尚未发布**，所以本 change 不必迁就它：表按目标形状重建，开发机旧库重置一次即可。
5. **配置是 deny_unknown_fields**：删掉 `[dispatch] state_file` 键会让仍带该键的配置文件解析失败。无发布版时这是**可接受且更诚实**的选择（见 D5）。

## Goals / Non-Goals

**Goals:**

- 让会话映射在 SIGKILL 下不丢：这是本 change 唯一的行为收益，也是 spec 早已要求的。
- 消除两条 spec 之间的矛盾，让 `session-lifecycle` / `session-persistence` / `state-store` 三者一致。
- 表结构一次做对，不背「只能加列」留下的历史包袱。

**Non-Goals:**

- 不改映射语义（字段含义、Dormant 恢复、pending 语义）。
- 不做遗留导入、不保留旧文件、不为兼容把新列做成可空。
- 不做保留期或清理。

## Decisions

### D1 持久性粒度 = 按变更（spec 要求），实现复用单写 actor

会话创建 / 模式或模型变更 / 标签变更 / 关闭时，把映射变更作为一个事务提交。**代价**：写次数上升（每个生命周期事件一条）。**为何可接受**：状态库是 WAL + 单写线程，一次提交是一条 SQLite 事务，不涉及新进程或新文件；而收益是关停不再是持久化的必要环节。**被否备选**：保留关停快照并额外加周期性快照——半吊子，仍会丢窗口内的变更。

### D2 表按目标形状重建为 ActiveRecord struct，`SessionMapRow` 与 `MappingDto` 合一

`session_map` 按映射的完整形状重建：需要的列可声明为 `NOT NULL` 并带合适默认值，主键仍是 `(chat_id, thread_id)`（它是「按会话键寻址」的既有语义，本 change 不重画）。struct 挂 `#[derive(ActiveRecord)]`（复合主键生成 `find_by` / `delete_by`，见 `extract-sebas-db` D3），`SessionMapRow` 与 `MappingDto` **合一**——一行即一个实例，「按变更落盘」的实现就是生命周期事件处一次 `entry.save(&store)`（经 store 闭包保持单写串行），不再有「组装 DTO → 自由函数序列化」的中间层。

**理由**：产品尚未发布，旧库重置一次是可接受的成本；而「为兼容把每个新列都做成可空」会把表结构永久钉在历史包袱上，并在每次读取处引入「None 代表旧行」的隐性分支。**被否备选**：只加可空列——无发布版时为兼容付出的代价买不到任何东西。

### D3 **不做遗留导入**（取消原方案）

原计划：旧 `sessions.json` 存在且库中会话表为空时导入一次并标记。**取消**：无发布版，没有需要搬运的既有安装；而导入本身要引入标记、幂等性、损坏文件处理与四条测试——为不存在的数据付成本。旧文件**不再被读取**，留在盘上可随手删除。

**被否备选**：保留导入（好处仅是「升级后会话列表不为空」，但那些映射指向的会话子进程在无发布版语境下没有保留价值）。

### D4 损坏语义与状态库对齐

- 库**无法打开**（损坏）→ 由 `state-store`「Corrupt store is not silently reset」管辖：拒绝启动、指名路径、绝不重置。
- 会话**映射条目**不可读 → 空表 + 诚实日志，**绝不因为会话映射而拒绝启动**。
- 旧的 `<path>.corrupt-<unix>` 文件隔离退休（不再有那个文件）。
- 若因 schema 形状变化触发重置，交由 `quarantine-database-reset` 的隔离语义保留旧库痕迹。

### D5 直接删除 `[dispatch] state_file` 键，不留过渡期

**理由**：无发布版，没有需要平滑的部署。配置里残留该键会以「未知键」报错——这比留一个「能解析但不生效」的假键**更诚实**：后者会让操作员以为配置生效了。代价是文档与沙箱菜谱必须同步改（`AGENTS.md` 把它列为必配项），否则本机沙箱会以配置错误启动；这一步写进任务。
**被否备选**：保留键一个版本 + warn（原计划）——为不存在的部署付复杂度。

### D6 依赖：不再需要 schema 加固，仅建议排在 `extract-sebas-db` 之后

原计划建议排在 `harden-schema-migration` 之后，理由是「今天不可调和即删库」有风险。既然本 change 主动接受一次重置，该依赖消失。仍建议排在 `extract-sebas-db` 之后，纯为复用共享运行时（连接配方与单写 actor）而非自建。

## Risks / Trade-offs

- **[开发机上的旧库被重置一次（会话映射清空）]** → 接受的成本（无发布版）；重置会隔离旧文件、日志给出路径，需要时可手工恢复。
- **[删配置键导致带该键的配置启动失败]** → 明确写进任务：`tasks.py`、`AGENTS.md` 与沙箱菜谱同步更新；错误信息是标准未知键报错，指向性强。
- **[per-mutation 写入拖慢会话创建]** → 复用单写 actor，不新增线程/文件；用既有 e2e 的时长基线对照，出现明显回退则记录并评估批量提交（但不得退回关停快照）。
- **[`SessionMapRow` 与 `MappingDto` 合一改动面]** → 编译器兜底（两处结构合一后，未更新的构造点直接编译失败）；分步提交：先建新形状与新读写，再切调用点，最后删旧类型。
- **[测试裸读 `sessions.json`]** → `tests/restart_recovery_test.rs`（断言 `.corrupt-*` 隔离）、`tests/sigterm_cleanup_test.rs`（预置并断言重序列化）、`tests/support/mod.rs`（路径钉）必须改写，且**不得只是删断言**：改为经状态库断言同样的语义（映射存活、不阻塞启动）。

## Migration Plan

1. `session_map` 按目标形状重建，`SessionMapRow` 与 `MappingDto` 合一；开发机旧库重置一次（隔离保留痕迹）。
2. 接上引擎：`load_session_map` / `save_session_map` 变为生产路径，会话生命周期事件按变更提交。
3. 退休关停 dump 与 `session_boot` 的文件读取/隔离；删除 `[dispatch] state_file` 键。
4. 改测试、`tasks.py`、`AGENTS.md`；更新 `session-persistence` 的 Purpose（去掉「being migrated」的进行时表述）。
5. 全量回归。

**回滚**：分步提交。第 1 步（表重建）回滚后旧库已隔离，可手工恢复；第 2–3 步可各自 revert。无遗留导入，因此不涉及「旧文件被改坏」的风险。

## Open Questions

- 是否需要为 `last_active_unix` 建索引以支撑未来的「按活跃时间清理」：今日无此查询需求，不加索引。
- `MappingDto` 合一后是否需要保留一个 serde 兼容层：不需要——无发布版，且该类型不出现在任何线格式里（它是磁盘/内存形状）。

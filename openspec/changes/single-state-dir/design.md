## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **逐文件变量共 7 个**：`SEBAS_STATE_DB`、`SEBAS_ARCHIVE_PATH`、`SEBAS_WEBUI_AUTH_DB`、`SEBAS_PROJECTS_PATH`、`SEBAS_HOME`（+ `HOME` 兜底），另有已退休的 `SEBAS_STATE_FILE` 与 `SEBAS_ROUTER_PROVIDER_OVERLAY`（见 `retire-legacy-state-json`）。`AGENTS.md:92-107` 称其中五个为 all mandatory；`tasks.py:_sandbox_env` 实钉 8 个。
2. **两个文件今天钉不住**：`services.json` 硬编码 `~/.sebas/services.json`（`src/watchdog.rs:45-51`）；`nodes.json` 只有配置键 `[node_link] registry_file`。
3. **单库现状与增长事实**：`sebas.db` 注册 6 张表（`providers`、`model_aliases`、`settings`、`projects`、`session_map`、`schema_meta`）。其中 `session_map` 在 `persist-session-map` 落地后**按变更写入**；`archive.json` 已含**完整转录**（体积随会话增长，且已被 `webui` spec 要求独立存放）；`router-usage.jsonl` 目前**无限增长**。也就是说「会增长的用户数据」这一侧是真实存在的，且方向明确。
4. **`archive.json` 的默认值已经是从 `SEBAS_STATE_DB` 的目录派生**（`sebas-webui/src/archive.rs:93-98`）——本 change 把这个既有的局部思路推广为单一目录规则。
5. **原子写的域边界**：`save_persisted_state` 在**一个事务**里写 `providers` + `model_aliases` + `settings.runtime_state`（`repo.rs:139-141`）——三者都是「有界系统配置」，因此全部落在 `settings.db`，事务完整，不跨库。
6. **`extract-sebas-db` 已把「每库一次 open + 重置策略 + 单写执行」做成运行时**，本 change 是它的第一个多库消费者。

## Goals / Non-Goals

**Goals:**

- 让沙箱只需钉一个变量就覆盖全部落盘，消除「漏钉即越界」。
- 让**有界的系统配置**与**会增长的用户数据**分处两个文件，各自备份、各自维护、各自重置。
- 让「有什么落点、各归哪个库」成为一张可枚举、可机械断言的表。

**Non-Goals:**

- 不拆到业务域粒度；不引入跨库事务；不动 `auth.db`；不迁 `services.json`。

## Decisions

### D1 优先级：逐文件显式覆盖 > 状态目录 > 默认

**理由**：既有部署与沙箱菜谱都用逐文件变量，降级会让既有设置静默失效；目录变量是新入口，未设置时行为确定。**被否备选**：目录变量压过逐文件变量（会让既有 `SEBAS_ARCHIVE_PATH=…` 之类的设置静默失效）。

### D2 **分层采用两级规则：先写入进程，后增长特征**

```
第一级（必须）：一个文件一个写入者
  core   → settings.db, projects.db
  webui  → auth.db
  router → usage.db
  node   → 无库（跨机器；且迁库会把 bundled C SQLite 拖进节点产物）

第二级（core 内）：按增长特征分
  settings.db  有界：providers / model_aliases / settings —— 行数由手写配置决定，不随运行时间增长
  projects.db  增长：projects / session_map（+ 后续会话与消息）—— 随使用持续膨胀、频繁写
```

**第一级的理由**：单写者所有权是 `state-store` 的既有要求，也是「谁能开这个库」的唯一可行判据；`auth.db` 不能并入 `settings.db` 正因为它由 webui 写。

**第二级的理由（本 change 的核心判断）**：两类数据的**增长特征相反**——把「不膨胀的设置」与「会膨胀的用户数据」放同一个文件，会让备份、VACUUM/checkpoint、以及 per-mutation 写入的 WAL 抖动全部混在一起，且重置用户数据会连设置一起丢。分层让这三件事各自独立。

**被否备选**：单库（原计划）——省一次 open，但代价是上面三件事永远混在一起，而且**现在改最便宜**（无发布版、无数据）；按业务域细分（provider.db / model.db / card.db）——分层依据不是业务域粒度，过度拆分只会增加 open 次数与协调面。

### D3 `providers` / `models` 归 `settings.db`，并扁平化为类型化列——它们本就是设置项

按 D2 第二级规则：provider 与 model 的行数由**人工配置**决定（几个到几十个），不随运行时间增长，与 card 配置、default selection 同类。因此它们与 `settings` 同库。

同时，借两库重建立 schema 之机，把 `providers` 从 `config TEXT (JSON blob)`（读写作 `Item = Map<String, Value>`、键名靠约定）**扁平化为类型化列**（name / preset / `base_url_anthropic` / `base_url_openai` / `base_url_responses` / `api_key_env` / `api_key` 等），一行即一个 provider 实例，由 `#[derive(ActiveRecord)]` 生成 CRUD（机制见 `extract-sebas-db`）。这终结了 workspace 里最后一处「表存 JSON、键名靠约定」的存储。**为什么现在做**：无发布版 + 反正要建新库，重塑的边际成本最低；列名与既有 JSON 键名对齐，channel 上的 provider JSON 形状经 serde 保持不变（webui/router 的线形状零变化）。

**顺带解掉一处我原以为的冲突**：`save_persisted_state` 在一个事务里写 `providers` + `model_aliases` + `settings.runtime_state`（Context 5）。三者同属「有界系统配置」→ 同落 `settings.db` → 事务完整（经 store 闭包），**不需要任何跨库机制**。

### D4 拆库前必须先枚举跨域原子性（前置检查项）

拆库的唯一真实风险是**把原本在同一个事务/同一快照里的不变量拆到两个库上**。因此实现前的第一步是**枚举所有写事务与快照读的域边界**：

- 已知：`save_persisted_state`（settings 域内，安全）、`import_defaults_once`（settings 域内）、projects CRUD（projects 域内）、session_map 写入（projects 域内）。
- 需核对：channel 的 state 快照是否要求「providers + projects + session_map」在同一快照内自洽。**若要求**，则它今天已经是多次查询（多个 state 方法），拆库不会让它更差，但**必须明确写出这一点**；若发现某个**写**事务横跨两库 → 调整边界（并入同一库，或改为最终一致），**不得引入跨库两阶段提交**。

这一项写进任务，且是「先核对、后拆」的顺序。

### D5 每个库独立：独立 open、独立策略、独立版本与隔离

两个库各走一次 `sebas-db` 的 open（各自 WAL、各自 `busy_timeout`、各自 schema 注册表与版本戳）。**收益**：重置 `projects.db`（例如 schema 演进）不影响 `settings.db`；反之亦然。**代价**：两次 open 与两份注册表——由 `sebas-db` 承担，不新增自建配方。

### D6 `services.json` 保留为文件：watchdog 是守护进程，不引入持久层

**先纠正一个容易误会的事实——开关的控制路径不经过 core。** 发起方确实是 webui 服务页 / CLI / feishu（经 im），但它们的终点是 **watchdog 自己的 control RPC socket**（`SEBAS_CONTROL_SOCKET`），不是 core channel：server 在 watchdog 进程内（`src/watchdog/control_rpc.rs:201`），客户端是 standalone webui 的 `ControlRpcAdminAdapter`（`src/webui_cmd.rs:433`）、CLI（`src/cli.rs:364`）与 im 的 `WatchdogControl`（`src/im_cmd.rs:100`）。core 在这条路径上是**被操作的对象**，不是中转：`ServiceSet` 命名 core 一律被拒（enable-core-by-default，webui 服务页对 core 只提供 restart）。收到 `ServiceSet` 后，**写入 `services.json` 的就是 watchdog 进程自己**（`src/watchdog/services.rs`）；webui 只是通过 control RPC 遥控它，自己既不读也不写这个文件。

**为什么不能并入 `settings.db`**（两级规则的第一级直接否决）：

1. **写入者不同。** `settings.db` 的写入者是 **core**，且 `state-store` 明文「Only the core process SHALL open the database」。`services.json` 的写入者是 watchdog。并入即二选一：要么 watchdog 直接开 core 的库（违反单写者所有权），要么经 core channel 写——后者把「切换服务」这个动作的落盘挂在被监督者的心跳上。provider/models 能并入 settings 正因为它们的写入者就是 core；本条不满足这个前提。
2. **存活依赖倒置。** watchdog 读覆盖层决定拉起哪些服务时，core 尚不存在；用户切换服务最常见的原因恰是 core 出问题，此时 core channel 不可用，「记录 router off」却要求 core 活着才能落盘。监督者的记忆不能依赖被监督者的心跳。
3. **watchdog 的定位是守护进程，不引入持久层。** `services.json` 的性质因此是**操作员配置**，与 `config.toml` 同类：它是三层解析（config → 覆盖层 → runtime）的中间层，记录的是操作员经 `ServiceSet` 做出的显式决定；watchdog 对它只做两件事——启动时照念、收到 `ServiceSet` 时改写。没有数据库、没有 ActiveRecord、没有 state store，也没有将来引入它们的计划。

**沙箱越界问题不靠挪库解决**：`services.json` 已在本次改为从状态目录派生 + 可显式覆盖，漏钉风险已经消除。

**结论不设触发条件**：watchdog 不引入持久层是本次记录的定位（守护进程形态），日后即使监督状态真的长出复杂需求（操作历史、更新/回滚记录等），也必须作为新 change 显式重新评估，默认答案是不做——而不是默认长出一个 `watchdog.db`。「core 不可用时覆盖层照常工作」与「runtime 覆盖在 watchdog 重启后仍生效」两个场景已写进 spec，把这条定位变成可测试的要求。

### D7 默认落点一并收敛到状态目录

全部逻辑名的默认值收敛为「默认状态目录 + 固定文件名」。实际几乎不改变现状（`auth.db`、`archive.json`、`projects.json`、`services.json` 今天的默认已在 `~/.sebas`；`archive.json` 甚至已是派生）；**唯一真正移动的是 `nodes.json`**（配置目录 → 状态目录）。**理由**：无发布版，不必分两次改动以保持「升级行为可归因」；一条规则比「规则 + 例外表」简单，也让机械断言可以写成无条件形式。

### D8 退休 `SEBAS_STATE_DB`，逐库覆盖变量取而代之

`sebas.db` 不再存在，故 `SEBAS_STATE_DB` 退休；取而代之的是状态目录变量 + 逐库覆盖（`settings.db` / `projects.db` / `auth.db` / `usage.db` 各一个）。**理由**：无发布版，无需为旧文件名保留别名；留着一个指向「已不存在的库」的变量比删掉更危险。同步更新 `tasks.py`、`AGENTS.md` 与沙箱菜谱（那是漏钉风险的来源）。

## Risks / Trade-offs

- **[把不变量拆到两个库]**（本 change 最大风险）→ D4 的前置枚举；已知的写事务都在单库内；快照读本来就跨多次查询，拆库不使其更差，但要显式记录。
- **[两次 open 的顺序与失败语义]**：一个库能开、另一个失败时怎么办？→ 明确为「任一库不可用即该域降级并如实报告」，不假装成功（对齐 `state-store`「Unavailable store degrades honestly」）。
- **[目录变量与逐文件变量同时设置时的歧义]** → D1 的优先级 + 两种组合各一条测试。
- **[退休 `SEBAS_STATE_DB` 造成本机沙箱启动异常]** → 文档与 `tasks.py` 同步（任务里列），且启动日志对未知/退休变量给出明确提示。
- **[`nodes.json` 默认位置变化]** → 唯一移动的落点，D7 已点名；测试里用期望清单钉住新位置。
- **[测试大量硬编码路径]** → `tests/support/mod.rs` 的路径钉改为经映射表取值，避免新增硬编码。

## Migration Plan

无发布版，故**无数据迁移**：

1. 落地逻辑名映射表（逻辑名 → 所属库 → 文件名 → 覆盖变量）与统一解析（复用 `sebas-domain` 的 tilde 展开）。
2. 用 `sebas-db` 把 core 的两个库各开一次，注册表按域拆分（settings 三表 / projects 两表）。
3. 完成 D4 的跨域原子性枚举，结论写进本文件。
4. 各落点改走映射表；`services.json` 与 `nodes.json` 变为可派生 + 可显式覆盖（修越界点）。
5. 退休 `SEBAS_STATE_DB`；更新 `tasks.py` / `AGENTS.md`。
6. 加两项机械断言（派生覆盖、默认收敛）。
7. 全量回归 + 沙箱越界复核。

**回滚**：分步提交。`sebas.db` 不被删除（只是不再打开），如需回退，把注册表合回一个库即可；两库拆分本身不涉及数据搬迁，因此回滚无需数据恢复。

## Open Questions

- 文件名：`projects.db` 将来还会承载会话与消息，名字是否需要更中性的（如 `data.db` / `sessions.db`）？今天按 `proposal.md` 的命名执行，改名成本在发布前始终很低。
- `session_map` 是否应随会话/消息一起留在 `projects.db`（今日方案）还是随使用频率另立一库：取决于 per-mutation 写入的频率实测，不必现在定。

## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **三处双存的位置都已存在**：`settings.key='runtime_state'`（mode + default_selection）、`providers` + `model_aliases` 表、`settings.key='card_config'`。**无需新表**。
2. **spec 已先于实现**：`router-admin-api`「Configuration source」已要求 legacy JSON 不导入不写入，并把 router 热更新定为 core channel 订阅；`router-admin-api`「External change hot reload」也只描述 channel，不要求文件监视。
3. **唯一相反的 spec**：`feishu-cards`「Card theme configuration」要求 card 设置持久化为 JSON 文件。
4. **权限缺口**：`settings.json` 写入时 `chmod 0600`（`sebas-dispatch/src/settings.rs:46-62`），而状态库连接配方（`src/sebas_state/db.rs:18-34`）**没有任何 chmod**。库里今天已含 provider 的 `api_key`（`providers.config` 是 JSON blob），因此这是**既有的**权限缺口，迁入 card 配置只是让它更严重。
5. **一次性导入的既有范式**：`src/sebas_state/defaults_import.rs` 用 `settings` 表里 `key='defaults_imported'` 做标记，**在同一个事务里**写标记与数据；文件缺失或损坏也落标记，保证恰好导入一次，并有回归测试（`imports_once_then_never_reads_again`）。
6. **读取方清单**：`settings.json` 由 core 回退（`src/run.rs:167-190`、`670-684`）、standalone webui（`src/webui_cmd.rs:662-684`）、im（`src/im_cmd.rs:301-313`）读取；`state.json` 由 core 读取（`sebas-dispatch/src/state_store.rs:335-340`、`935`）；`providers.json` 由 router 读取（`sebas-router/src/config.rs:751`、`773`）与监视（`hot_reload.rs:137-144`）。

## Goals / Non-Goals

**Goals:**

- 让「文件回退」在代码里彻底消失，而不是留着偶尔生效。
- 让 spec 与实现一致：把已要求的事做完，把唯一相反的一条改正。
- 顺手补上状态库的 0600，使「敏感配置只归库」这件事不带来权限回退。

**Non-Goals:**

- 不新增表/列，不拆库，不改线格式。
- 不为 provider 数据做遗留导入（spec 明文禁止）。
- 不引入新的路径环境变量。

## Decisions

### D1 三个文件同 change 同修

同一个根因（DB 已是权威、文件是回退）、同一个修法（删路径）、同一次验证（同一批测试裸读这些文件）。拆开做只会让「回退还在」的状态跨多个 change 继续存在。

### D2 `settings.json` 落到既有的 `settings.key='card_config'`，不新增表

`repo.rs:293-325` 的 `load_settings`/`save_settings` 已经读写这个键。core 的读取回退（先库后文件）改为**只读库**；standalone webui 与 im 改为经 state 方法（它们本来就有 channel 客户端）。**被否备选**：为 card 设置建独立表——无收益，键值表就是它的形状。

### D3 **不做任何遗留导入**——无发布版，没有需要搬运的安装

原计划按类三分（`card_config` 缺值导入一次 / provider 不导入 / `runtime_state` 不导入）。**取消导入这一支**：导入要引入标记键、幂等性、文件缺失与损坏分支、以及「恰好一次」的回归测试——全部是**为既有安装的数据**付的成本，而产品尚未发布、不存在既有安装。

后果要如实说明：库为空时 card 设置**回到默认主题**，遗留文件里的值不会被搬进来。这是无发布版语境下可接受的取舍，而不是遗漏。**被否备选**：仅对 `card_config` 保留导入（用户可观测的配置值得保住）——若真要保住，正确做法是「发布前不做 schema 破坏性变更」，而不是为实现期的一次性导入写一套机制。**触发条件**：首个正式发布前重新评估（那时才存在需要保护的数据）。

逐类结论（供查阅）：provider / alias **不导入**——`router-admin-api` 明文禁止，本 change 不推翻该既有决定；`runtime_state` **不导入**——selection 的遗留导入已由 `defaults.json` 的一次性导入承担，mode 由载入期修复。

### D4 状态库文件补 0600，并把「敏感值只归库」当作契约

`sebas-db` 的 open 在创建文件后 `chmod 0600`（Unix），并在启动期校验既有文件权限、不符时收紧。**理由**：库里今天已有 provider `api_key`，迁入 card 配置后敏感面更大，而 0600 是 `settings.json` 原本就有的保证——迁入不该成为权限回退。Windows 由 ACL 语义承担（与既有 `core.secret` 的处理一致）。**被否备选**：把敏感值加密入库——引入密钥管理，超出本 change。

### D5 环境变量**退休**而非保留为空操作

`SEBAS_STATE_FILE` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 删除。**理由**：留着不生效比删掉更危险——操作员会以为自己配对了目录。同步更新 `tasks.py` 的沙箱 env 集合与 `AGENTS.md` 的菜谱（那里把它们列为「必钉」）。**代价**：既有脚本可能带着这些变量运行；行为等同未设（因为不再读取），不会报错。

### D6 router 的 overlay 文件监视一并退休

`sebas-router/src/hot_reload.rs` 的文件监视路径删除。**理由**：`router-admin-api` 两处都已把 channel 订阅定为机制，文件监视是实现残留；而文件本身不再被写入，监视一个永不变化的文件没有意义。**风险**：若真的有部署依赖「手动编辑 providers.json 触发重载」，该能力消失——但 spec 已明确要求 router 不写 provider 数据、且以 channel 为源，`scripts/e2e_gateway_admin.sh` 里那条外部改写用例正是要改掉的旧行为。

## Risks / Trade-offs

- **[用户可观测的配置丢失]** → D3 的 `card_config` 一次性导入 + 专项用例（文件在、库空 → 导入后卡面一致）。
- **[测试大量裸读文件而失败]** → 明确列出改造清单：`tests/state_persistence_test.rs`、`tests/spawn_env_store_authority_test.rs`、`tests/testsuite_e2e_test.rs`（providers.json 缺席断言保留但改为断言「不创建」）、`scripts/e2e_gateway_admin.sh`（外部改写热更新用例删除或改写为 channel 通知）。逐个改，不删断言。
- **[权限收紧导致既有部署启动失败]**（文件权限比 0600 宽时为收紧，不是失败；只在无法收紧时报错）→ 实现为「尝试收紧 + 失败则告警但不中止」，避免把一个安全加固变成无法启动。
- **[删掉文件后 operator 以为数据丢了]** → 文件**留在盘上不动**（spec 场景已钉），并在启动日志与文档里说明可安全删除。

## Migration Plan

无 schema 变更，故无表迁移；只有数据的一次性导入（仅 `card_config`）。

1. `sebas-db` 补 0600 与启动期权限校验。
2. `card_config` 一次性导入落地（含「文件缺、库空」与「库已有值」两条用例）。
3. 删 `state.json` / `providers.json` 的写入与读取回退；退休两个环境变量。
4. 三个读取方（core 回退 / standalone webui / im）改走 state 方法；删 router 的 overlay 读取与监视。
5. 改测试与脚本、更新 `tasks.py` / `AGENTS.md`。
6. 全量回归。

**回滚**：分步提交，每步可 revert。无表结构变更、无数据删除（文件原样留在盘上），因此回滚不涉及数据恢复。

## Open Questions

- 遗留文件是否需要在若干版本后**主动提示删除或自动归档**：今日只留不动 + 日志说明。若操作员反馈盘上残留困扰，另立 change。
- `card_config` 的导入标记是否需要与 `defaults_imported` 共用同一张表的命名空间：实现便利问题，不影响 spec 与任务分解。

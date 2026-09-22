# Proposal: retire-legacy-state-json

## Why

`state.json`、`providers.json`、`settings.json` 三个文件**已经是遗留回退**，不是活的存储：状态库已是权威，三者的值也早已在库里有对应位置（`settings.runtime_state`、`providers`/`model_aliases`、`settings.card_config`）。`state.json` 的 `save()` 在 engine 就绪后直接委托给库，文件实质冻结；`providers.json` 已**没有生产写入方**（router 的 admin 变更端点已退休，router 变成只读消费者，`tests/testsuite_e2e_test.rs:2056` 已断言该文件从不被创建）。

关键在于 **spec 已经先于实现**：`router-admin-api`「Configuration source」已明文要求「Legacy JSON files SHALL NOT be imported and SHALL NOT be written: the state store starts empty and remains the only authority」，而同一条也把 router 的热更新定为**走 core channel 订阅**，所以 `sebas-router/src/hot_reload.rs` 的文件监视同样是多余机制。反过来 `feishu-cards`「Card theme configuration」还明文要求 card 设置「persisted as a full-snapshot JSON file」——这是唯一一处 spec 与目标相反的表述。

于是本 change 不是新设计，而是**把 spec 已经要求的事做完，并修正一处 spec 与实现相反之处**：删掉三个文件的写入路径、读取回退与两个环境变量。

## What Changes

- **退休 `state.json` 与 `providers.json`**：删写入路径与读取回退（含 `src/provider.rs` 的 corrupt-overlay 隔离、router 的 overlay 文件监视），删除 `SEBAS_STATE_FILE` 与 `SEBAS_ROUTER_PROVIDER_OVERLAY`。
- **退休 `settings.json`**：CardConfig 快照改由状态库承载（`settings.db` 的 `settings` 表 KV 行，键 `card_config` 已存在；经 `SettingRow` 的 ActiveRecord CRUD 读写），core / standalone webui / im 三处读取改走 state 方法；同步修正 `feishu-cards` 的持久化要求。
- **不做任何遗留导入**：三处的值已在库中，库是唯一权威。产品尚未发布，没有需要搬运的既有安装，因此不为任何一类数据实现导入与标记。
- **补上状态库文件权限 0600**：`settings.json` 今天 `chmod 0600`，而状态库文件**没有**任何 chmod——库里今天就有 provider `api_key`，迁入后还多一份 card 配置。权限回退必须在本 change 内补齐。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `feishu-cards`: 「Card theme configuration」的持久化表述由「full-snapshot JSON file」改为「状态库中的快照」，并明确状态库是 card 设置的唯一权威；严格解析与默认主题语义不变。
- `cli-service`: 「Config precedence and environment variables」的 override 集合移除已退休的 `SEBAS_ROUTER_PROVIDER_OVERLAY`，并说明退休变量不再生效。
- `session-persistence`: 新增「遗留 JSON 状态文件已退休」要求——不再写入也不再读取 `state.json` / `providers.json`，状态库是唯一权威；相应环境变量退休；读取方一律走 state 方法。

## Impact

- **改动**：`src/provider.rs`（overlay 写入与损坏隔离）、`src/run.rs`（启动读取回退）、`sebas-dispatch/src/state_store.rs`（文件回退与 `load_at`）、`sebas-dispatch/src/settings.rs`（文件读写）、`src/webui_cmd.rs` / `src/im_cmd.rs`（settings.json 读取）、`sebas-router/src/{config,hot_reload}.rs`（overlay 文件读取与监视）、`sebas-db`（库文件 0600）、`tasks.py` 与 `AGENTS.md`（退休的环境变量与沙箱菜谱）。
- **测试面**：`tests/state_persistence_test.rs`、`tests/spawn_env_store_authority_test.rs`、`tests/testsuite_e2e_test.rs`（providers.json 缺席断言）、`scripts/e2e_gateway_admin.sh`（外部改写 overlay 测热更新）都要改——它们今天**裸读/裸写这些文件**。
- **不变**：所有表结构与线格式不变；不新增表、不新增列。验收 = 既有 `state_persistence_test` / `state_subscription_test` 改写后全绿 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。
- **用户可观测**：无发布版，故**不保证**遗留文件里的值被搬入库；库为空时 card 设置回到默认（见 design D3）。遗留文件本身不被删除也不被读取。

## Non-goals

- **不做遗留数据导入**（三类数据一个都不导）——无发布版，没有需要搬的安装。
- **不动 `sessions.json`**（会话映射）——它今天仍在被写入且是 rest 恢复的唯一来源，迁移是 `persist-session-map`。
- **不动 `projects.json` 与 `archive.json`**（webui 自有，含远程项目与转录）——分别属 `migrate-project-registry` 与后续 change。
- **不动 `router-usage.jsonl`**——属 `persist-router-usage`。
- **不新增表或列，也不决定表所属的库**——本 change 只删路径，不碰 schema；分层（`settings.db` / `projects.db`）由 `single-state-dir` 决定。
- **不为 provider 数据做遗留导入**——spec 已明文禁止，本 change 不推翻该决定。
- **不引入新的路径环境变量**——单目录派生是 `single-state-dir` 的事。

# Proposal: persist-router-usage

## Why

`router-usage.jsonl` 是**唯一由非 core 进程自持的运行时持久化**。它的现状有三处欠缺：**无限增长**（无保留期、无轮转）、**每条记录都重新 `open(O_CREATE|O_APPEND)` + `write_all` + flush**（`sebas-router/src/usage.rs:92-121`）、以及**无查询能力**（要按模型/上游/状态聚合只能自己写脚本解析）。同一文件里的 `key` 字段**恒为空**（`sebas-router/src/auth.rs` 侧的归属信息没有落进去），说明这份数据本身还没被真正用起来。

它是追加型遥测而非状态，所以 SQLite 的事务与约束带来的收益有限；真正的收益是**可查询**与**能被保留期管住**。代价也要说清：失去 NDJSON 逐行可读（`scripts/e2e_router.sh` 与 e2e 直接解析它），且 router 产物会新增 SQLite 依赖。因此本 change 独立立项——它是这批里唯一价值可疑的边界，可以单独取消而不牵连其它四个。

**关键约束**：`state-store`「Database location and single-writer ownership」要求「Only the core process SHALL open the database」。所以 router 的用量数据**不能**写进 core 的 `sebas.db`——要么 router 拥有自己的库文件，要么每记录经 channel 上报（热路径上一次往返，否决）。

## What Changes

- **router 拥有自己的用量库**（状态目录下的 `usage.db`），单写者即 router 自己；**不打开 core 的状态库**。用量记录落为 ActiveRecord struct `UsageRecord`（一行即一条记录，`record.save(&store)` 即写入；机制见 `extract-sebas-db`——模式统一，struct 留在 `sebas-router`，归属按写入者）。
- **保留期**：按时间与行数上限定期后台清理——把今天「无限增长」这个既有缺陷一并修掉，而不是把它带进库。
- **异步 sink 语义不变**：容量 256 的有界通道、满则丢弃并 warn、**绝不阻塞或失败在途响应**。
- **记录字段与语义不变**（时间戳、协议、模型、provider、上游模型、状态、延迟、ttft、四类 token 计数、error，`key` 仍恒空）。
- **直接删除 `[router] usage_file` 键**：新增 `[router] usage_db`，旧键不留过渡期——产品尚未发布，配置里残留该键会以未知键报错，这比留一个「能解析但不生效」的假键更诚实。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `router-auth-rate-limit`: 「Usage record pipeline」更新——落点由 JSONL 文件改为 router 自有用量库，新增保留期要求（时间 + 行数上限、后台清理），异步 sink 的溢出与不阻塞语义不变。
- `router-auth-rate-limit`: 「Usage record content」更新——记录字段集不变，表述由「JSONL record」改为「记录」，并明确记录可被查询。

## Impact

- **改动**：`sebas-router/src/usage.rs`（sink 改为库写入 + 后台清理）、`sebas-router/src/config.rs`（`usage_db` 键、`usage_file` 两步退休）、`sebas-router` 的依赖（新增 `sebas-db`）、`tasks.py` / `AGENTS.md`（`[router] usage_file` 说明）。
- **测试面**：`tests/testsuite_e2e_test.rs:928-957` 的 `read_jsonl` 辅助与断言、`scripts/e2e_router.sh:225-237`、`tests/support/mod.rs:361` 都要改为查询库。
- **依赖**：已满足——`extract-sebas-db` 与 `single-state-dir` 均已实现归档（`sebas-db` 的 `Record` trait + `StateHandle` 门面、状态目录逻辑名映射表均已落地），本 change 可直接实现。
- **验收**：新增「保留期生效（超期与超行数各一条）」+「sink 溢出仍丢弃且响应不受影响」+「router 不打开 core 的状态库」三道断言；e2e 的用量断言改为查库后全绿。

## Non-goals

- **不写 core 的状态库**——`state-store` 明文禁止非 core 打开它。
- **不做跨进程用量聚合**、不做用量 UI、不做按 key 归属（`key` 恒空是既有决定，本 change 不改）。
- **不新增导出 CLI**：保留可查询性靠 `sqlite3` 直查 + 文档示例，不为此扩展 `cli-service`。
- **不动 SSE usage tee 的提取逻辑**（`sebas-router/src/sse.rs`）——只换落盘。
- **不动 node 的会话日志与 meta.json**：那在另一台机器上，迁库会把 bundled C SQLite 拖进节点产物（与 `extract-sebas-db` D1 的节点隔离理由相悖）。

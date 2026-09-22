## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **硬约束**：`state-store`「Database location and single-writer ownership」——「Only the core process SHALL open the database; all other processes access state exclusively through the core channel state methods」。router 用量历史不属于 domain state，但它也**不能**写 core 的库。
2. **router 今天没有任何 DB**：其依赖图里没有 `rusqlite`；`sebas-ipc` 提供了 socket 传输，`sebas-db` 提供连接配方与单写 actor（见 `extract-sebas-db`）。router 是控制面角色，依赖中立的 `sebas-db` 不触犯任何隔离纪律（那是 `sebas-node` 的约束）。
3. **sink 语义是既有要求**：容量 256 的有界通道、满则丢弃 + warn、绝不阻塞/失败响应（`router-auth-rate-limit`「Usage record pipeline」的两个场景）。
4. **无保留期是既有缺陷**：`usage.rs` 只追加，`[router] usage_file` 无轮转配置；今天靠操作员手工清理。
5. **裸读面**：`tests/testsuite_e2e_test.rs:928-957` 用 `read_jsonl` 解析它并断言 `model` / `upstream_model` / token 数；`scripts/e2e_router.sh:225-237` 检查非空；`tests/support/mod.rs:361` 钉路径。
6. **配置键的事实**：router 配置用 `deny_unknown_fields`，删键会让仍带该键的配置**解析失败**。

## Goals / Non-Goals

**Goals:**

- 用量数据可查询、可被保留期管住——把「无限增长」这个既有缺陷一次修掉。
- 保持 sink 的全部既有语义（异步、有界、可丢弃、不阻塞）。
- 让 router 的用量库自动落在状态目录内，从而被沙箱覆盖（依赖 `single-state-dir`）。

**Non-Goals:**

- 不写 core 的库；不做跨进程聚合；不做 UI；不做导出 CLI。
- 不动 SSE 提取逻辑；不动 node 侧日志。

## Decisions

### D1 router 拥有自己的库文件，绝不开 core 的库

**理由**：D 段的硬约束（Context 1）。**被否备选**：每记录经 core channel 上报——热路径上一次往返，且用量必须「绝不阻塞响应」，两者直接冲突；**被否备选**：写进 core 的库——违反明文要求且需要放宽单写者语义。

### D2 保留期是必备项，不是可选项

今天的 JSONL 无限增长**本身就是缺陷**；若不设保留期，把无限增长搬进带 WAL 的库会更糟（WAL 无界增长 + 需要 vacuum + 查询随行数退化）。因此 spec 把保留期写成要求：**按时间**（配置的保留窗口）与**按行数上限**双闸，后台定期间隔清理。**被否备选**：入库但不设保留期——把缺陷换个容器装。**取舍**：保留期意味着历史会被删；默认值取保守（足够长），且清理动作写日志，避免「数据悄悄消失」。

### D3 异步 sink 语义逐条保留，落盘为 ActiveRecord 记录

容量 256、满则丢弃并 warn、绝不阻塞或失败在途响应、写失败不影响路由。实现上复用 `sebas-db` 的单写执行模型（一个自有线程/连接，命令串行），从而**不引入自建的一套连接管理**。`UsageRecord` struct 挂 `#[derive(ActiveRecord)]`（表 `usage_records`，时间戳为主键或含自增 id——实现时定），sink 的写入就是 `record.save(&store)`；批量提交（一次闭包写若干条）可降低提交次数，但**不得**改变「记录丢失只发生在通道满/关闭时」这一语义。struct 留在 `sebas-router`——模式靠共享 trait + derive 统一，归属按写入者（`User` 在 webui、core 表在 `sebas-models`，同理）。

### D4 记录字段与语义一字不改

含 `key` 恒空这一既有决定（`router-auth-rate-limit`「Usage record content」的场景已钉）。本 change 只换存储与增加可查询性，不改归属语义——改归属是独立的产品决定。

### D5 直接删除 `usage_file` 键，不留过渡期

新增 `[router] usage_db`，删除 `usage_file`。**理由**：无发布版，没有需要平滑的部署；而保留一个「能解析但不生效」的假键会让操作员以为配置还在起作用——**报未知键是更诚实的失败**。**代价**：文档与脚本必须同步（任务里已列）。**被否备选**：保留键一个版本 + warn（为不存在的部署付复杂度）。

### D6 明确取舍：失去裸 grep，换来查询与保留期

NDJSON 逐行可读是当前操作员的实际用法（`AGENTS.md` 与脚本都直接解析）。迁库后需要 `sqlite3` 查询；本 change 提供文档示例，**不新增导出 CLI**（避免为此扩展 `cli-service`，也避免一个半成品导出面）。**这是一处有意的降级**，写进 proposal 与变更说明。

### D7 独立立项以便单独取消

用量是**遥测**而非状态：SQLite 的事务与约束对它收益有限。若这批 change 需要砍一个，就是它——所以它单独成 change、且不阻塞其它四个。**触发条件（重启价值）**：出现跨机器/多 router 实例的用量聚合需求，或需要按 key 归属做成本分摊时，本 change 的价值才真正兑现。

## Risks / Trade-offs

- **[测试与脚本裸读 JSONL 而失败]** → 明确清单（Context 5）改为查库断言；**不删覆盖点**（token 计数、上游模型等断言要逐条保留）。
- **[router 产物变大 + 新增依赖]** → 依赖 `sebas-db` 而非直接 `rusqlite`，与 core 共用同一份配方与版本；在 PR 里给出二进制体积变化。
- **[热路径延迟]** → sink 保持异步 + 批量提交；对比改造前后 e2e 的代理延迟基线，出现回退则调整批量大小（不得改为同步写）。
- **[保留期误删有用历史]** → 默认窗口取保守值、清理写日志、并允许通过配置关闭时间闸只留行数闸。
- **[router 库损坏影响代理]** → 用量写失败**不得**影响路由（既有 sink 语义）；库不可用时降级为「丢弃 + warn」，不阻塞响应。实现须用测试钉住这条。

## Migration Plan

1. `[router] usage_db` 键 + `usage_file` 两步退休（保留但忽略 + warn）。
2. 用 `sebas-db` 落 sink（自有库、单写执行、批量提交），保持异步与丢弃语义；加「router 不开 core 库」断言。
3. 加保留期（时间 + 行数）与后台清理；清理写日志。
4. 改测试与脚本（查库断言）、`tasks.py` / `AGENTS.md`。
5. 全量回归 + 体积/延迟对照。

**回滚**：分步提交。第 2 步可 revert 回 JSONL sink（届时需一并恢复 `usage_file` 键）。**不做遗留导入，也不要求保留既有 `router-usage.jsonl`**：无发布版，用量历史非关键状态，导入的收益不抵一次性迁移的复杂度——这是**有意不做**，不是遗漏。

## Open Questions

- 是否需要把历史 JSONL 一次性导入 `usage.db`：默认不做（见 Migration Plan 的说明）。若操作员需要连续的历史，可另立小 change。
- 保留期默认值取多少：实现时按典型部署的请求量取保守值，并在文档写明可配。

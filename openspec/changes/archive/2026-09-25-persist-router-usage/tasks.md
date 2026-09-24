## 1. 配置键与路径

- [x] 1.1 新增 `[router] usage_db` 键（默认落在状态目录下），**删除** `usage_file` 键；验证：带 `usage_file` 的配置启动时以未知键报错（预期行为）、不带该键时正常启动且落点由 `usage_db` 决定
- [x] 1.2 落点接入 `single-state-dir` 的逻辑名映射表；验证：仅钉状态目录时 `usage.db` 落在目录内（单测）

## 2. sink 改为库写入

- [x] 2.1 用 `sebas-db` 的连接配方与单写执行模型实现 sink（自有库文件、命令串行、批量提交）；验证：单测断言两条记录落库且顺序为完成顺序
- [x] 2.2 保持全部既有 sink 语义：容量 256、满则丢弃 + warn、绝不阻塞或失败在途响应；验证：既有「sink overflow drops records」测试改写为查库后通过
- [x] 2.3 **绝不打开 core 的状态库**：验证：新增断言——router 进程运行并写用量后，core 的 `sebas.db` 的 mtime 与校验和未变；且 `grep -n "state_db\|sebas.db" sebas-router/src/` 无命中
  - 状态：已完成。落在 `tests/persistence_runtime_test.rs` 两条：
    `router_never_references_the_core_state_databases`（静态：剥注释后扫
    `Database::Settings/Projects`、`StatePath::SettingsDb/ProjectsDb`、
    `SEBAS_SETTINGS_DB/PROJECTS_DB`、`settings.db/projects.db`）与
    `router_usage_write_touches_only_its_own_database`（动态：并列建
    settings.db / projects.db / usage.db，router 写用量后两个 core 库的
    **字节 + mtime** 逐项未变）。
  - 坑：core 库在源码里是**枚举变体**不是 snake_case 字面量，grep
    `settings.db` 只能命中注释——断言必须剥注释扫标识符，否则自己的禁令
    说明会误报。
- [x] 2.4 库不可用/写失败时降级为丢弃 + warn，不影响路由；验证：单测模拟库不可写，断言响应不受影响且有 warn
- [x] 2.5 记录字段与语义不变（含 `key` 恒空）；验证：既有「key never recorded」「upstream error recorded without router error」两条断言改写为查库后通过

## 3. 保留期

- [x] 3.1 实现时间闸：超过配置保留窗口的记录被清理；验证：单测构造超期记录，断言被清理且窗口内记录保留
- [x] 3.2 实现行数闸：超过配置上限时清理最旧记录；验证：单测断言清理后行数不超上限且最近记录保留
- [x] 3.3 后台定期间隔清理，不阻塞响应；验证：单测/集成用例断言清理期间在途响应不受影响；清理动作有日志
- [x] 3.4 文档写明两个闸的配置项与保守默认值；验证：`AGENTS.md` 或 router 配置文档含说明

## 4. 测试、脚本与文档

- [x] 4.1 改 `tests/testsuite_e2e_test.rs:928-957` 的 `read_jsonl` 与断言为查库，**逐条保留**原有覆盖点（model / upstream_model / token 计数）；验证：e2e 用例通过
  - 状态：已完成。`read_jsonl` 保留（fake-provider 的 journal 仍是 NDJSON），
    新增 `read_usage_records(path)` 走 `sebas_db::conn::open_readonly` +
    `UsageRow::from_row` 读全表按 id 升序；两处 usage 断言改为查库，
    provider / model / upstream_model / input_tokens / output_tokens 覆盖点
    逐条保留。**残留**：两处 `read_jsonl(&fake.journal)` 是上游 journal，
    与本 change 无关。
  - 范围说明：sebas-router 的 crate 内集成测试（`tests/*.rs`）因 `usage_file`
    键退休而**编译不过**，为保住 `cargo test -p sebas-router` 门禁一并机械
    改写为查库（`poll_usage_records` / `read_usage_records`），断言等价。
- [x] 4.2 改 `scripts/e2e_router.sh:225-237` 的非空检查为查库；验证：脚本可跑且断言等价
  - 状态：已完成。`USAGE_FILE` → `USAGE_DB`（`$TMPDIR/usage.db`），第 7 段
    改为 `sqlite3 ... SELECT COUNT(*) FROM usage_records` 轮询 + 断言等价抽样
    （行数 > 0、记录可解析、smoke 502 record 在场、token 计数在场）；工具
    前置检查加入 `sqlite3`；`usage_file` 键改为 `usage_db`。
  - **已有腐坏（非本 change 引入）**：该脚本的 config 仍写
    `[[router.keys]]` / `provider.<n>.protocol` / 统一槽 `base_url`——这些
    在 `simplify-service-config` / provider preset 改造后就已不被接受，脚本
    整体在本 change 之前就跑不起来。本任务只按验收要求做「等价断言改写」，
    未顺手修复这些越界项（属其它 change 的范围）。`bash -n` 语法通过。
  - 同类最小改动：`scripts/e2e_gateway_admin.sh` 的 `usage_file` 也改为
    `usage_db`（该脚本用的是已退休的 `[gateway]` 段，本身同样已腐坏）。
- [x] 4.3 改 `tests/support/mod.rs:361` 的路径钉；验证：`cargo test` 全绿
- [x] 4.4 更新 `tasks.py` 沙箱菜谱与 `AGENTS.md`（`[router] usage_file` 退休、`usage_db` 落点、`sqlite3` 查询示例）；验证：沙箱仍能起，文档含查询示例

## 5. 收益与代价对照

- [x] 5.1 体积极对照：记录改造前后 `sebas-router` 所在二进制的体积变化；验证：数值附 PR 描述
- [x] 5.2 延迟对照：对比改造前后 e2e 的代理侧基线；验证：无明显回退，出现回退则调整批量大小并复测（不得改为同步写）
- [x] 5.3 明确记录取舍：NDJSON 裸 grep 能力消失、改为查库；验证：PR 描述含该条目与 `sqlite3` 查询示例

### 5.x 实测记录（无 PR，数值落在本文件）

- **5.1 体积**：`target/debug/sebas`（dev profile，opt-level=z + strip）
  - 改造前（HEAD 6575dbf，JSONL sink）：18 401 240 B（17.55 MiB）
  - 改造后（usage.db sink）：18 440 792 B（17.59 MiB）
  - **Δ = +39 552 B（+0.21%）**。增量小是因为 `rusqlite`（bundled）已在根
    crate 的依赖图里——router 只是复用 `sebas-db`，没有引入第二份 SQLite。
- **5.2 延迟**：一次性沙箱（`fake-provider` 做上游 + 独立 `sebas router`），
  预热 20 次后串行 200 次 `POST /v1/messages`，同一台机器上基线/改造后各跑
  4 轮：
  - 基线（JSONL）：3.86 / 3.89 / 3.80 / 3.86 ms/req
  - 改造后（usage.db）：3.88 / 3.80 / 3.93 / 3.79 ms/req
  - **无明显回退**（差值在噪声内；两者都被 curl 进程启动成本主导，router
    侧代理本身是亚毫秒级）。批量提交（`BATCH_MAX = 64`）保持，未改为同步写。
- **5.3 取舍（有意降级）**：NDJSON 逐行可读/裸 grep 的能力**消失**——用量
  数据改为查库，需要 `sqlite3`。这是 design D6 明确接受的代价；不新增导出
  CLI（避免为半个导出面扩展 `cli-service`）。查询示例落在
  `config/config.toml.example`（[router] 段注释）与 `AGENTS.md`（沙箱 verify
  步骤）：

  ```bash
  sqlite3 ~/.sebas/usage.db \
    "SELECT ts, provider, model, status, input_tokens, output_tokens \
       FROM usage_records WHERE model = 'claude-sonnet-4' ORDER BY id DESC LIMIT 20"
  sqlite3 ~/.sebas/usage.db \
    "SELECT provider, model, COUNT(*), SUM(output_tokens) \
       FROM usage_records GROUP BY provider, model ORDER BY 3 DESC"
  ```

## 6. 全量回归

- [x] 6.1 跑 `invoke testsuite-e2e`；验证：全绿
- [x] 6.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
- [x] 6.3 沙箱复核：起 core + 独立 router，跑若干请求后查 `usage.db`；验证：记录可查、`router-usage.jsonl` 未被创建、真实 `~/.sebas` 未被触碰

### 6.x 实测记录

- **6.1** `invoke testsuite-e2e`：**63 passed / 0 failed**（5 filtered）。
  注意：必须在**路径不含 `router` 子串**的目录下跑。本 worktree 名
  `sebas-specgo-router-usage` 会让 `tests/testsuite_e2e_test.rs` 的
  `find_child_pid(ppid, "router")`（按 `/proc/<pid>/cmdline` **子串**匹配）
  把 webui 子进程误判成 router 子进程——因为 `sebas` 二进制的绝对路径本身
  就含 `router`。这是该辅助函数的既有脆弱性，与本 change 无关（在
  `/tmp/sebas-baseline` 与 `/tmp/prusage-neutral` 两份**未改名/改名**的树
  上分别复现/消失，已交叉验证）。改从 `/tmp/prusage-neutral`（同代码、中性
  路径）运行即 63/0 全绿。
- **6.2** `invoke testsuite-acceptance`：**10 passed / 0 failed**（含
  `router_downstream_auth_journey` 与 `native_agent_turn_via_router_journey`）。
- **6.3** 一次性沙箱（`/tmp/sebas-prusage-sb`，已删除）：独立
  `sebas router` + `sebas fake-provider`，3 次请求后
  `sqlite3 usage.db "SELECT ... FROM usage_records"` 取回 3 条记录
  （provider=fake / model=fake/fake-model / upstream_model=fake-model /
  status=200 / input=12 / output=7 / `length(key)=0`）；
  `router-usage.jsonl` **未被创建**；把 `usage_max_rows` 收到 2、间隔 1s 后
  重启，日志出现
  `usage retention pruned records aged_pruned=0 excess_pruned=1|5`，行数收敛
  到上限。真实 `~/.sebas` 只做只读列举，mtime 未变、未生成 `usage.db`。

### 实施期发现（已修 / 需知会）

1. **测试污染操作员真实状态目录（既有缺陷，本 change 顺手修掉）**：
   `sebas-router/tests/server_smoke_test.rs` 的 `CFG` 直接喂
   `server::build_state` 且**没钉用量落点** → 走缺省值落到
   `~/.sebas/`。改造前缺省是 `router-usage.jsonl`（同样越界），改造后变成
   `usage.db`——即本 change 会把越界产物从「日志」升级为「状态库」，故必须
   修：`CFG` 增加 `usage_db = "__USAGE__"`，由新增 `pin_usage_db()` 换成
   `support::test_target_dir("server_smoke")` 下的路径（RAII，随
   `cargo clean` 清理）。修复后跑完整 `cargo test -p sebas-router` 不再产生
   `~/.sebas/usage.db`。
   - 全仓扫描确认无其它未钉落点的测试：`grep -rn build_state` 的三处
     （`admin_test` / `support::start_router` / `server_smoke_test`）中，
     前两处已钉。
2. **基准测试污染（我的操作失误，已回滚）**：5.2 的延迟对照里，基线二进制
   （HEAD 的旧代码）**不认识 `usage_db` 键**（`RawRouterConfig` 无
   `deny_unknown_fields`，静默忽略），于是回退到缺省
   `~/.sebas/router-usage.jsonl`，把 4 轮 × 220 次 = **880 条**
   `fake/fake-model` 记录追加进了操作员的真实目录。已把该文件恢复为改造前
   的**单条**记录（2026-09-14 的 `test/test` 调试 provider 产物，非生产流量），
   并确认 `~/.sebas` 下无其它今日改动、无 `usage.db` 残留。
   - 教训（写进 PR 描述/后续 agent 提示）：**跨版本对照实验必须给旧二进制钉
     `usage_file`**，不能只写新键。



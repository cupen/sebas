## 1. 配置键与路径

- [ ] 1.1 新增 `[router] usage_db` 键（默认落在状态目录下），**删除** `usage_file` 键；验证：带 `usage_file` 的配置启动时以未知键报错（预期行为）、不带该键时正常启动且落点由 `usage_db` 决定
- [ ] 1.2 落点接入 `single-state-dir` 的逻辑名映射表；验证：仅钉状态目录时 `usage.db` 落在目录内（单测）

## 2. sink 改为库写入

- [ ] 2.1 用 `sebas-db` 的连接配方与单写执行模型实现 sink（自有库文件、命令串行、批量提交）；验证：单测断言两条记录落库且顺序为完成顺序
- [ ] 2.2 保持全部既有 sink 语义：容量 256、满则丢弃 + warn、绝不阻塞或失败在途响应；验证：既有「sink overflow drops records」测试改写为查库后通过
- [ ] 2.3 **绝不打开 core 的状态库**：验证：新增断言——router 进程运行并写用量后，core 的 `sebas.db` 的 mtime 与校验和未变；且 `grep -n "state_db\|sebas.db" sebas-router/src/` 无命中
- [ ] 2.4 库不可用/写失败时降级为丢弃 + warn，不影响路由；验证：单测模拟库不可写，断言响应不受影响且有 warn
- [ ] 2.5 记录字段与语义不变（含 `key` 恒空）；验证：既有「key never recorded」「upstream error recorded without router error」两条断言改写为查库后通过

## 3. 保留期

- [ ] 3.1 实现时间闸：超过配置保留窗口的记录被清理；验证：单测构造超期记录，断言被清理且窗口内记录保留
- [ ] 3.2 实现行数闸：超过配置上限时清理最旧记录；验证：单测断言清理后行数不超上限且最近记录保留
- [ ] 3.3 后台定期间隔清理，不阻塞响应；验证：单测/集成用例断言清理期间在途响应不受影响；清理动作有日志
- [ ] 3.4 文档写明两个闸的配置项与保守默认值；验证：`AGENTS.md` 或 router 配置文档含说明

## 4. 测试、脚本与文档

- [ ] 4.1 改 `tests/testsuite_e2e_test.rs:928-957` 的 `read_jsonl` 与断言为查库，**逐条保留**原有覆盖点（model / upstream_model / token 计数）；验证：e2e 用例通过
- [ ] 4.2 改 `scripts/e2e_router.sh:225-237` 的非空检查为查库；验证：脚本可跑且断言等价
- [ ] 4.3 改 `tests/support/mod.rs:361` 的路径钉；验证：`cargo test` 全绿
- [ ] 4.4 更新 `tasks.py` 沙箱菜谱与 `AGENTS.md`（`[router] usage_file` 退休、`usage_db` 落点、`sqlite3` 查询示例）；验证：沙箱仍能起，文档含查询示例

## 5. 收益与代价对照

- [ ] 5.1 体积极对照：记录改造前后 `sebas-router` 所在二进制的体积变化；验证：数值附 PR 描述
- [ ] 5.2 延迟对照：对比改造前后 e2e 的代理侧基线；验证：无明显回退，出现回退则调整批量大小并复测（不得改为同步写）
- [ ] 5.3 明确记录取舍：NDJSON 裸 grep 能力消失、改为查库；验证：PR 描述含该条目与 `sqlite3` 查询示例

## 6. 全量回归

- [ ] 6.1 跑 `invoke testsuite-e2e`；验证：全绿
- [ ] 6.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
- [ ] 6.3 沙箱复核：起 core + 独立 router，跑若干请求后查 `usage.db`；验证：记录可查、`router-usage.jsonl` 未被创建、真实 `~/.sebas` 未被触碰

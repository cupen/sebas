## 1. 状态库文件权限

- [ ] 1.1 `sebas-db` 的 open 在创建后把库文件设为 0600（Unix），并在打开既有库时校验权限、过宽则收紧；验证：新增单测断言新建库权限为 0600，且把一个 0644 的既有库打开后变为 0600
- [ ] 1.2 `-wal` / `-shm` 与目录权限一并核对；验证：单测断言 WAL 模式下 `-wal` 文件权限不宽于库文件
- [ ] 1.3 权限收紧失败时告警但不中止启动；验证：单测模拟不可 chmod 的场景，断言启动继续且日志有 warn

## 2. 不做遗留导入（核对项）

- [ ] 2.1 确认三处遗留文件的值**不被导入**：`card_config` / `runtime_state` / providers 一律以库为唯一权威，库为空即取默认；验证：单测构造「文件有值 + 库空」，断言启动后库仍为空、行为取默认值（**不**出现导入标记）
- [ ] 2.2 确认没有引入任何导入标记键；验证：`grep -rn "imported\|_import" src/sebas_state/` 只剩既有 `defaults_imported`（`defaults.json` 那条与本次无关）

## 3. 退休 state.json 与 providers.json

- [ ] 3.1 删 `provider` overlay 的写入路径与损坏隔离（`src/provider.rs`）；验证：`cargo test -p sebas` 全绿，且 `grep -rn "providers.json" src/` 只在注释或测试夹具中残留
- [ ] 3.2 删 `state_store` 的文件回退（`load_at` / `save_at` 的调用路径改为仅库），保留 legacy v0/v1 的**读入拒绝**语义；验证：`cargo test -p sebas-dispatch` 全绿，且「库不可用」时呈现 unavailable 而非文件派生值
- [ ] 3.3 删 `src/run.rs` 的 `state.json` / `providers.json` 启动读取回退；验证：删除两个文件后启动行为不变（库为权威）
- [ ] 3.4 退休 `SEBAS_STATE_FILE` 与 `SEBAS_ROUTER_PROVIDER_OVERLAY`；验证：设置这两个变量后启动，路径解析与不设置时**完全一致**（单测覆盖）
- [ ] 3.5 删 router 的 overlay 文件读取（`sebas-router/src/config.rs:751,773`）与文件监视（`hot_reload.rs`）；验证：`cargo test -p sebas-router` 全绿，且 provider 变更经 channel 通知仍热生效
- [ ] 3.6 从 `cli-service`「Config precedence and environment variables」描述的 override 集合中移除 `SEBAS_ROUTER_PROVIDER_OVERLAY`（该要求已在本 change 的 delta 中更新）；验证：该 spec 文本不再把它列为生效变量，且实现里无读取点

## 4. settings.json 退休

- [ ] 4.1 core 读 card 设置改为只读库（删 `src/run.rs` 的文件回退分支）；验证：删除 `settings.json` 后卡面渲染配置取库中默认值（库为空时即默认主题）
- [ ] 4.2 standalone webui 与 im 改为经 state 方法取 card 设置；验证：两个进程的既有用例全绿，且不可达时呈现 unavailable 状态
- [ ] 4.3 删 `sebas-dispatch/src/settings.rs` 的文件读写；验证：`grep -rn "settings.json" src/ sebas-*/src/` 无生产代码命中

## 5. 测试、脚本与文档

- [ ] 5.1 改 `tests/state_persistence_test.rs` / `tests/spawn_env_store_authority_test.rs`：不再裸写这些文件驱动状态；验证：两个测试文件全绿，且其中不再有 `std::fs::write` 到 legacy 路径
- [ ] 5.2 改 `scripts/e2e_gateway_admin.sh` 的「外部改写 providers.json 测热更新」用例（改为经 channel 通知，或删除该用例并说明理由）；验证：脚本可跑且断言与新的权威来源一致
- [ ] 5.3 保留并复核 `tests/testsuite_e2e_test.rs` 的「providers.json 从不被创建」断言；验证：断言通过，且新增对 `state.json` / `settings.json` 同款缺席断言
- [ ] 5.4 更新 `tasks.py` 的沙箱 env 集合与 `AGENTS.md` 的沙箱菜谱（移除已退休的两个变量与对应「必钉」说明）；验证：沙箱仍能起、且 `AGENTS.md` 不再声称这两个变量会生效
- [ ] 5.5 复核并更新 `openspec/specs/session-persistence/spec.md` 的 Purpose（它还声称拥有 `state.json` / `providers.json` 的磁盘布局）；验证：Purpose 与 `specs/session-persistence` 的新增要求一致

## 6. 全量回归

- [ ] 6.1 跑 `invoke testsuite-e2e`；验证：全绿
- [ ] 6.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
- [ ] 6.3 遗留文件不被读取也不被改动：在含三个遗留文件的机器（沙箱模拟）上启动并跑完整流程；验证：三个文件逐字节未变，且库中不出现它们的值（无导入）

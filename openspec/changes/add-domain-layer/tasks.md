## 1. 建共享域层骨架

- [ ] 1.1 新建 `sebas-domain` 目录与 `Cargo.toml`（`name = "sebas-domain"`，依赖仅 serde / serde_json / thiserror），加入根 `Cargo.toml` 的 `[workspace] members`；验证 `cargo build -p sebas-domain` 通过
- [ ] 1.2 写 `src/lib.rs` 模块骨架（`session` / `project` / `provider` / `prim`）与 crate 级文档注释，说明准入规则（≥2 个 crate 需要且角色中立方可入）；验证 `cargo doc -p sebas-domain` 无警告且模块清单与 design D5 一致
- [ ] 1.3 在根 `Cargo.toml` 与 `sebas-webui` / `sebas-dispatch` / `sebas-router` / `sebas-im` / `sebas-node` 各 `Cargo.toml` 加 `sebas-domain = { path = ... }` 依赖；验证 `cargo build --workspace` 通过

## 2. 中立原语收敛

- [ ] 2.1 **先审计再收敛**：逐点核对 6 份会话键实现的语义（`sebas-dispatch/src/engine/mod.rs:2559`、`sebas-webui/src/routes.rs:170-181`、`src/node_link/projection.rs:1035`、`sebas-im/src/frontend.rs:1205`、`src/agent_backend.rs:331`、`sebas-channels/src/key.rs:77-86`），逐处判定「跨进程 / 持久化 / 参与 map key 比较」，产出一张等价性结论表；验证：表中每一行都有「等价 / 不等价 + 理由」，且 `src/agent_backend.rs:331` 的 `serde_json` 编码与 `projection.rs:58` 的 `node\0` 嵌套两种非标准形态均已单独判定
- [ ] 2.2 在 `sebas-channels::key` 落地唯一编解码实现（`encode_session_key` / `decode_session_key`，`urlencoded` 形态 + `node\0` 嵌套形态），用 2.1 收集的现有输出做黄金样本；验证：单元测试逐字节比对通过，且对每个非标准形态的保留/收敛决定有测试或注释钉住
- [ ] 2.3 替换全部调用点并删除 6 份重复实现；验证：`cargo test -p sebas-channels -p sebas-dispatch -p sebas-webui -p sebas-im` 全绿，且 `grep -rn "percent_encode\|urlencoding::encode" --include=*.rs src/ sebas-*/src/` 只剩 `sebas-channels` 一处
- [ ] 2.4 `expand_tilde` 移入 `sebas-domain::prim`，替换 `src/config.rs` / `sebas-router/src/config.rs` / `sebas-dispatch/src/state_store.rs` 三处；验证：三处调用方测试全绿，`grep -rn "fn expand_tilde"` 只剩一处定义
- [ ] 2.5 `now_unix` 等时间戳原语移入 `sebas-domain::prim`，替换 `sebas-webui/src/user_store.rs:237` 等本地实现；验证：相关 crate 测试全绿且无第二份 `fn now_unix` 定义

## 3. 中立契约类型迁移（原位再导出）

- [ ] 3.1 把 `sebas_dispatch` 的 `SessionInfo` / `RemoteSessionView` / `SessionEvent` / `TurnEntry` / `TurnStreamEvent` / `SessionIdentity` / `PendingApproval` / `PendingSubmission` 的定义移入 `sebas-domain::session`，`sebas-dispatch` 对应模块改为 `pub use`；验证：`cargo test -p sebas-dispatch` 全绿，且 `sebas_dispatch::SessionInfo` 等路径在现有调用点仍可解析
- [ ] 3.2 把 `sebas-webui::session_backend` 的 `SessionRejection` / `PermissionNotice` / `PermissionDecision` 定义移入 `sebas-domain::session`，webui 原位再导出（`SessionBackend` trait 与 `Reachability` 不动）；验证：`cargo test -p sebas-webui` 全绿，且 `sebas-webui/tests/ws_rpc_contract_test.rs` 无改动通过
- [ ] 3.3 把 provider 状态词表中仍是词表的部分（`DefaultSelection` / `ProviderMode`）与 providers.json overlay 读取器移入 `sebas-domain::provider`，`sebas-dispatch::state_store` 与 `sebas-router` 原位再导出，删除 `sebas-router/src/config.rs:947` 的 `ProviderOverlay` 与 `:960` 的 `ModelAliasEntry` 副本；验证：`cargo test -p sebas-router -p sebas-dispatch` 全绿，且 router 的 provider 热重载用例（`sebas-router/tests/*`）全绿
- [ ] 3.4 把 `sebas-webui::models::SessionRow` / `ConversationEntryView` 的手写字段罗列改为 `From<&SessionInfo>` / `From<&TurnEntry>` 显式转换（类型与字段本身不动）；验证：`cargo test -p sebas-webui` 全绿且 `sebas-webui/tests/api_endpoints_test.rs` 的响应形状断言无改动通过
- [ ] 3.5 补形状钉测试：`ProjectRow` ↔ `ProjectEntry` 双向转换与两侧序列化形状各钉一个测试；验证：新增测试通过，且人为给一侧加字段时测试失败（在 PR 描述里附一次失败演示）

## 4. 显式重声明收口

- [ ] 4.1 `NodeView` 与 `NodeInfo` 合一（保留 webui 的 `local: bool` 为独立字段），`sebas-webui/src/session_backend.rs:161` 改为引用共享定义；验证：`cargo test -p sebas-webui` 与节点列表相关用例全绿
- [ ] 4.2 复核并删净本 change 触及的复制点；验证：`grep -rn` 清单（会话键、`expand_tilde`、`now_unix`、`NodeInfo`、`ModelAliasEntry`、`ProviderOverlay`）各只剩一处定义，结果附在 PR 描述
- [ ] 4.3 加叶子属性机械断言测试（放在根 crate 的集成测试里）：解析 `cargo tree -p sebas-domain` 输出，断言不含 core/webui/router/im 与 sebas-node；同时断言 `cargo tree -p sebas-node` 仍不含角色实现；验证：新增测试通过，且临时给 `sebas-domain` 加一条 `sebas-webui` 依赖时该测试失败（附一次失败演示）

## 5. 全量回归与收口

- [ ] 5.1 线格式快照比对：对 core channel / node link / webui WS+HTTP 的代表性载荷做序列化比对，与重构前逐字节一致；验证：快照测试通过且无任何 diff
- [ ] 5.2 持久化兼容验证：用一份重构前生成的 `sebas.db` 与 state/providers/projects/settings JSON 启动重构后二进制；验证：无 schema 差异告警、无行丢失或重置（对照 `tests/state_persistence_test.rs` 与沙箱菜谱）
- [ ] 5.3 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
- [ ] 5.4 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位并记录
- [ ] 5.5 更新 `AGENTS.md` / `CLAUDE.md` 的 workspace crate 速查表，加入 `sebas-domain` 的定位与准入规则；验证：速查表含新 crate 且描述与 `specs/shared-domain-layer/spec.md` 一致

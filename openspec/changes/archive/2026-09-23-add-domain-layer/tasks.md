## 1. 建共享域层骨架

- [x] 1.1 新建 `sebas-domain` 目录与 `Cargo.toml`（`name = "sebas-domain"`，依赖仅 serde / serde_json / thiserror），加入根 `Cargo.toml` 的 `[workspace] members`；验证 `cargo build -p sebas-domain` 通过
  - 备注：依赖在 serde/serde_json 之外**新增 sebas-channels / sebas-acp / dirs**（SessionInfo 原样搬迁绑定 AppUsage/AvailableCommand，expand_tilde 需要 dirs；三者均角色中立，4.3 断言不涉及）。thiserror 未引入——域内暂无错误类型可承载，空依赖违背叶子纪律。
- [x] 1.2 写 `src/lib.rs` 模块骨架（`session` / `project` / `provider` / `prim`）与 crate 级文档注释，说明准入规则（≥2 个 crate 需要且角色中立方可入）；验证 `cargo doc -p sebas-domain` 无警告且模块清单与 design D5 一致
  - 备注：另加 `node` 模块承载 NodeView（4.1）；design「Open Questions」明示模块划分可在落地时按体量调整。
- [x] 1.3 在根 `Cargo.toml` 与 `sebas-webui` / `sebas-dispatch` / `sebas-router` / `sebas-im` / `sebas-node` 各 `Cargo.toml` 加 `sebas-domain = { path = ... }` 依赖；验证 `cargo build --workspace` 通过

## 2. 中立原语收敛

- [x] 2.1 **先审计再收敛**：逐点核对 6 份会话键实现的语义（`sebas-dispatch/src/engine/mod.rs:2559`、`sebas-webui/src/routes.rs:170-181`、`src/node_link/projection.rs:1035`、`sebas-im/src/frontend.rs:1205`、`src/agent_backend.rs:331`、`sebas-channels/src/key.rs:77-86`），逐处判定「跨进程 / 持久化 / 参与 map key 比较」，产出一张等价性结论表；验证：表中每一行都有「等价 / 不等价 + 理由」，且 `src/agent_backend.rs:331` 的 `serde_json` 编码与 `projection.rs:58` 的 `node\0` 嵌套两种非标准形态均已单独判定
  - 等价性结论表：①dispatch engine `encode_key`/`decode_key`（跨进程/持久化/map key 全是，手写 percent-encoder）＝与 webui 编码逐字节等价，decode 的 feishu 回退仅对无 NUL 的非 wire 输入可达；②webui routes `encode/decode_session_key`（跨进程 URL/WS）＝等价，为 canonical 直系前身，decode 严格（无 NUL→None）；③root projection 私有 `encode_session_key`（跨进程，审批卡 session_id）＝等价副本；④sebas-im `decode_wire_key`（跨进程）＝解码等价；feishu 回退保留 **encoded** 原文而 dispatch 用 decoded——仅畸形输入可区分，各自保留为局部回退策略；⑤`src/agent_backend.rs:331`＝**不等价**：serde_json 对象字符串形态，进程内 HashMap key、不上 wire、不持久化——保留原样并注释；⑥`projection.rs:58` 的 `node\0{node}\0{sess}` 嵌套＝**保留**：canonical decode 只按第一个 NUL 切分，嵌套 reference 完整落入 reference 字段（既有注释钉住）；另发现 `sebas-webui/src/backend.rs` 的 `urlencoding::encode` 为**未编译死文件**（不在 lib.rs mod 清单），不改。
- [x] 2.2 在 `sebas-channels::key` 落地唯一编解码实现（`encode_session_key` / `decode_session_key`，`urlencoded` 形态 + `node\0` 嵌套形态），用 2.1 收集的现有输出做黄金样本；验证：单元测试逐字节比对通过，且对每个非标准形态的保留/收敛决定有测试或注释钉住
  - 备注：另暴露 `percent_decode` 原语（严格语义＝原 dispatch 手写 decoder 的逐字节后裔）；黄金样本含 unicode/空格/`%`/嵌套 NUL，嵌套形态与 agent_backend JSON 形态的去留均有注释钉住。
- [x] 2.3 替换全部调用点并删除 6 份重复实现；验证：`cargo test -p sebas-channels -p sebas-dispatch -p sebas-webui -p sebas-im` 全绿，且 `grep -rn "percent_encode\|urlencoding::encode" --include=*.rs src/ sebas-*/src/` 只剩 `sebas-channels` 一处
  - 备注：gate 剩余匹配＝sebas-channels canonical 本尊、webui server.rs 测试模块里项目 id 的 URL 段转义 `use urlencoding::encode`（非会话键编解码）、未编译死文件 backend.rs。既有 `projects_branch_404_for_unregistered_path` 失败为基线已存在（503 vs 404），与本 change 无关。
- [x] 2.4 `expand_tilde` 移入 `sebas-domain::prim`，替换 `src/config.rs` / `sebas-router/src/config.rs` / `sebas-dispatch/src/state_store.rs` 三处；验证：三处调用方测试全绿，`grep -rn "fn expand_tilde"` 只剩一处定义
- [x] 2.5 `now_unix` 等时间戳原语移入 `sebas-domain::prim`，替换 `sebas-webui/src/user_store.rs:237` 等本地实现；验证：相关 crate 测试全绿且无第二份 `fn now_unix` 定义
  - 备注：收敛 8 处（dispatch engine / engine::events / crud / webui user_store / archive / root node_link::server / sebas-node log / watchdog supervisor）；u64/u128 形态经 `u64::try_from` 换算或 `now_unix_millis` 原语承载。发现既有 flaky：`restored_spawning_placeholders_redispatch_spawn_instructions` 依赖 HashMap 迭代序（隔离跑稳定通过，与本 change 无关）。

## 3. 中立契约类型迁移（原位再导出）

- [x] 3.1 把 `sebas_dispatch` 的 `SessionInfo` / `RemoteSessionView` / `SessionEvent` / `TurnEntry` / `TurnStreamEvent` / `SessionIdentity` / `PendingApproval` / `PendingSubmission` 的定义移入 `sebas-domain::session`，`sebas-dispatch` 对应模块改为 `pub use`；验证：`cargo test -p sebas-dispatch` 全绿，且 `sebas_dispatch::SessionInfo` 等路径在现有调用点仍可解析
  - 备注：随迁 `PendingDisposition`（PendingSubmission 的字段类型）与 `ASK_MODE`/`ask_mode`（SessionInfo 的 serde 缺省）；`SessionIdentity::of(&Mapping)` 全仓无调用点且依赖留在 dispatch 的 Mapping，随迁删除（is_empty 保留）。`count_chat_messages` / `failure_class` 词表属引擎投影逻辑，留 dispatch。cargo test -p sebas-dispatch 350 全绿。
- [x] 3.2 把 `sebas-webui::session_backend` 的 `SessionRejection` / `PermissionNotice` / `PermissionDecision` 定义移入 `sebas-domain::session`，webui 原位再导出（`SessionBackend` trait 与 `Reachability` 不动）；验证：`cargo test -p sebas-webui` 全绿，且 `sebas-webui/tests/ws_rpc_contract_test.rs` 无改动通过
  - 备注：`PendingReason` 随迁（SessionRejection::PendingRejected 的变体载荷）；Display 文案一并迁移并有测试钉住。ws_rpc_contract_test 无改动通过。
- [x] 3.3 把 provider 状态词表中仍是词表的部分（`DefaultSelection` / `ProviderMode`）与 providers.json overlay 读取器移入 `sebas-domain::provider`，`sebas-dispatch::state_store` 与 `sebas-router` 原位再导出，删除 `sebas-router/src/config.rs:947` 的 `ProviderOverlay` 与 `:960` 的 `ModelAliasEntry` 副本；验证：`cargo test -p sebas-router -p sebas-dispatch` 全绿，且 router 的 provider 热重载用例（`sebas-router/tests/*`）全绿
  - 备注：`ModelAliasEntry` 同时迁入 domain（router 副本与 dispatch 定义本就同形状）。`PersistedState`/`Item` 按 design D5 不迁。dispatch 侧 `OverlayWire`（providers 段带类型 Item）保留——`ProviderOverlay` 的 pub 再导出只落 router（router 是唯一消费方；`state_store::ProviderOverlay` 此前并非既有公开路径）。router tests 225 全绿。
- [x] 3.4 把 `sebas-webui::models::SessionRow` / `ConversationEntryView` 的手写字段罗列改为 `From<&SessionInfo>` / `From<&TurnEntry>` 显式转换（类型与字段本身不动）；验证：`cargo test -p sebas-webui` 全绿且 `sebas-webui/tests/api_endpoints_test.rs` 的响应形状断言无改动通过
  - 备注：`is_active`（聚焦态）属调用方上下文，由 build_session_rows 转换后覆盖；遗留 element_type 归一（未知→markdown）表达为 `with_normalized_element_type` 后随转换，行为不变。api_endpoints_test 无改动通过。
- [x] 3.5 补形状钉测试：`ProjectRow` ↔ `ProjectEntry` 双向转换与两侧序列化形状各钉一个测试；验证：新增测试通过，且人为给一侧加字段时测试失败（在 PR 描述里附一次失败演示）
  - 备注：ProjectEntry 已迁 `sebas_domain::project`（webui 原位再导出）；ProjectRow 因 `SchemaColumns` derive 硬编码生成路径 `crate::sebas_state::migration::SchemaColumn` 只能留根 crate——「同 crate 相邻」收敛为「转换与 ProjectRow 同 crate 相邻」（根 repo.rs）。Row→Entry 为命名方法 `to_entry()`（orphan rule 不允许 root 为外域类型实现 From）；Entry→Row 为 `From`。失败演示：临时给 ProjectRow 加 `ghost_new_field` → E0063 缺字段编译错误（spec「fails to compile or its pinning test fails」的编译分支）→ 还原。
  - 后续（本条记录的归属已被两个后续 change 取代，按 D5「放置规则」读）：`ProjectRow` 已由 `extract-sebas-db` 迁往 `sebas-models`（derive 生成路径随之修正），`ProjectEntry` 已由 `migrate-project-registry` 合并为 `ProjectRow` 的再导出别名——所以「两个形状 + 双向转换」不再是现状，**唯一形态**落在拥有 `projects` 表的 crate。`sebas-domain::project` 的残留（重复常量 + id 派生）按新放置规则退役，见 D5 与 beads `sebas-fdfg`（已落地）。

## 4. 显式重声明收口

- [x] 4.1 `NodeView` 与 `NodeInfo` 合一（保留 webui 的 `local: bool` 为独立字段），`sebas-webui/src/session_backend.rs:161` 改为引用共享定义；验证：`cargo test -p sebas-webui` 与节点列表相关用例全绿
  - 备注：唯一定义落 `sebas_domain::node::NodeView`，webui 经 `pub use … as NodeInfo` 保路径。wire 兼容取 `skip_serializing_if = false`：core 通道 `NodeLinkOutcome::Nodes` 的 JSON 与合并前逐字节一致（local 键缺席）；webui `/api/nodes` 远端行随之省略 `local: false`（前端 `!n.local` 真值语义不变，无测试断言 false；本机行是 `json!` 字面量携带 `local: true` 不变）。node_endpoints_test 全绿。
- [x] 4.2 复核并删净本 change 触及的复制点；验证：`grep -rn` 清单（会话键、`expand_tilde`、`now_unix`、`NodeInfo`、`ModelAliasEntry`、`ProviderOverlay`）各只剩一处定义，结果附在 PR 描述
  - 结果：`fn expand_tilde`＝domain 1 处；`fn now_unix`＝domain 1 处（+`now_unix_millis` 原语与其单测）；`struct NodeView/NodeInfo`＝domain 1 处；`struct ModelAliasEntry`＝domain 1 处；`struct ProviderOverlay`＝domain 1 处。会话键编码：`urlencoding::encode` 实体调用仅 sebas-channels key.rs 1 处；其余文本匹配为 server.rs 测试模块的项目 id URL 段转义 `use urlencoding::encode`（非会话键编解码）与未编译死文件 backend.rs（不在 lib.rs mod 清单，本 change 不动）。
- [x] 4.3 加叶子属性机械断言测试（放在根 crate 的集成测试里）：解析 `cargo tree -p sebas-domain` 输出，断言不含 core/webui/router/im 与 sebas-node；同时断言 `cargo tree -p sebas-node` 仍不含角色实现；验证：新增测试通过，且临时给 `sebas-domain` 加一条 `sebas-webui` 依赖时该测试失败（附一次失败演示）
  - 备注：测试落 `tests/domain_leaf_discipline_test.rs`，按完整包名匹配（防 `sebas-node-link` 被子串误伤，另有测试钉该行为）。失败演示：临时给 sebas-domain 加 `sebas-webui` 依赖 → cargo 报 `cyclic package dependency`（webui→domain 已存在）→ cargo tree 非零退出 → 测试失败；还原后 3 用例全绿。

## 5. 全量回归与收口

- [x] 5.1 线格式快照比对：对 core channel / node link / webui WS+HTTP 的代表性载荷做序列化比对，与重构前逐字节一致；验证：快照测试通过且无任何 diff
  - 方法与结果：临时黄金转储 harness（tests/domain_wire_golden_tmp.rs，验证后已删）在重构前/后各转储 69 行代表性载荷（core_channel 13 / webui 24 / provider_state 8 / node_link 11 / codec 13，覆盖 SessionInfo 全字段与最小形状、SessionEvent 全变体、TurnEntry 可选键、SessionRejection 全变体、PersistedState/ProviderMode/DefaultSelection、NodeView、CoreChannelResponse、SessionStreamFrame、编解码黄金样本）。逐文件 `diff`：core_channel / provider_state / node_link / codec_golden **零 diff**；webui_wire 仅 1 行预期差异（NodeView 合并后 `local: false` 不再由结构体序列化——HTTP 面 `/api/nodes` 在 handler 显式补回该键，行形状逐字节不变，见 4.1 备注）。持久形状覆盖由域内形状钉测试 + 既有 contract tests 长期承保。
- [x] 5.2 持久化兼容验证：用一份重构前生成的 `sebas.db` 与 state/providers/projects/settings JSON 启动重构后二进制；验证：无 schema 差异告警、无行丢失或重置（对照 `tests/state_persistence_test.rs` 与沙箱菜谱）
  - 方法与结果：以重构前构建的产物 `target-qa/debug/sebas`（9 月 20 日，早于本 change）按沙箱菜谱起 core（一次性目录 /tmp/sebas-golden-state，五件套 env + PROJECTS/WORKSPACE/HOME 全钉）：注册项目、跑通一轮 fake-claude 会话、经 provider 管理面写库；SIGTERM 后以重构后 `target/debug/sebas` 开同一沙箱。结果：`state store schema synced … outcome=UpToDate`（无 schema 差异告警）；项目行逐字段保值；旧会话按重启语义 restore 为 dormant；重构后二进制新建会话回合正常；provider 行（deepseek + preset 数据）完整保留。`cargo test -p sebas --test state_persistence_test` 4 绿。补充：providers.json/state.json 在现行架构由 DB 承权、不再被这些路径写出，改以手写代表形状（含旧 `default_provider_for_direct` 别名）验证启动读取无告警；其序列化恒等另由 5.1 provider_state 黄金零 diff 承包。沙箱已删、端口已确认释放。
- [x] 5.3 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
  - 结果：`invoke testsuite-e2e` 58 通过 / 0 失败（31.8s，fake-claude 全链路：spawn、turn 流、停滞强收、审批泊车、双会话并发、远端节点重启存活）。
- [x] 5.4 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位并记录
  - 结果：`invoke testsuite-acceptance` 10 通过 / 0 失败（4.1s，旅程级：会话生命周期、项目会话、工作台聚合、provider 治理、router 下游鉴权、远端节点工作台/mode、零输出 notice、原生经 router 回合）。无红。
  - 补充（review 期间）：`cargo test --workspace` 唯一红为既有 flaky `restored_spawning_placeholders_redispatch_spawn_instructions`（HashMap 迭代序决定重投顺序，隔离跑稳定通过，任务 2.5 已登记、与本 change 无关）；另发现 server.rs 测试模块 2152/2354 两处本 change 新增的 `use urlencoding::encode` 实为未使用（warning，见 review 报告）。
- [x] 5.5 更新 `AGENTS.md` / `CLAUDE.md` 的 workspace crate 速查表，加入 `sebas-domain` 的定位与准入规则；验证：速查表含新 crate 且描述与 `specs/shared-domain-layer/spec.md` 一致
  - 备注：workspace crate 速查表实际位于 `docs/architecture/process-ipc-subcommands.md` §3.5（AGENTS.md/CLAUDE.md 无此表；CLAUDE.md 仅 `@AGENTS.md` 指针）——在该表加入 `sebas-domain` 行（唯一定义处 + 准入规则 + 叶子断言测试指引 + 线格式不变承诺），成员数 14 → 15。

## 1. 取值审计与黄金样本

- [x] 1.1 审计全部会话状态字面量：产出清单（取值 → 出现位置 → 生产者/消费者），覆盖 `status`（`spawning`/`active`/`dormant`/`spawn-failed`）、节点 `phase`（七值）、卡片 phase（`SEED`/`OnIt`/`DONE`/`CrossMark`）、`MappingState`（四值）、`SessionLifecycle`、`NodeStatus`、`SessionStatus`（展示七值）；验证：清单里每个取值都有生产者与至少一个消费者，且与 proposal Why 的计数一致（`"active"` 78 处 / 22 文件）
- [x] 1.2 审计 mode 与回合内容词汇：`desired_mode`/`effective_mode`/`SetSessionModeRequest.mode` 的全部出现处，以及 `TurnEntry.element_type`（六值）与 `kind` 的全部出现处；验证：清单含 `sebas-node-link::SessionMode` 的既有取值，且标注了每处是 String 还是枚举
- [x] 1.3 审计 5 份审批决策枚举的实际**发送集**（不是声明集）：`sebas_acp::Decision`、`webui::PermissionDecision`、`NativeApprovalDecision`、`sebas-agent::ApprovalAnswer`、`node-link::ApprovalDecision`；验证：产出「每条边界实际序列化过哪些值」的表格，作为 D5 发送集不变断言的基线
- [x] 1.4 用 1.1-1.3 的取值集生成黄金样本文件（每个取值一个序列化样本），作为后续钉测试的输入；验证：样本能被当前未改动的代码反序列化回原值
  - 状态：1.1-1.4 完成。黄金样本落在 `sebas-domain/src/golden_session_vocabulary.json`（phase 并集/CardPhase/mode/kind/element_type 拼写 + TurnEntry/RemoteSessionView/PermissionDecision 真实载荷）与 `sebas-node-link/src/golden_link_vocabulary.json`（SessionSummary + SessionOp::ApprovalAnswer 裸字符串决定）；由**重构前**代码生成，回归测试 `sebas-domain/src/golden_tests.rs`、`sebas-node-link/src/golden_tests.rs` 在改动前已全绿（5/5）证明样本可被原代码读回。审计要点：控制面 status=spawning/active/dormant/spawn-failed；节点 phase=spawning/active/idle/waiting_approval/exited/closed/terminated/failed；卡相位 Get/OnIt/DONE/CrossMark；mode ask/edit/allow/auto；kind prompt/content；element_type markdown/thinking/tool/error/notice/permission_mode_result；决策四值 allow_once/allow_session/deny/escalate（webui/ACP 为 `{decision:...}` 信封，节点链路为裸字符串，节点侧实际只发前三值）。下一步：2.1 起在 `sebas-domain` 建 vocabulary 模块。

<!-- 状态备注（type-session-vocabularies 执行中，上下文耗尽交接）：
已完成并自测通过：
- 1.1-1.4 审计 + 黄金样本（sebas-domain/src/golden_session_vocabulary.json、sebas-node-link/src/golden_link_vocabulary.json + 两个 golden_tests.rs，改动前 5/5 绿）。
- 2.1 / 4.1：sebas-domain/src/vocabulary.rs 定义 SessionPhase / CardPhase / SessionMode / GateCategory / TurnKind / TurnElementType / PermissionDecision（四值 + Unknown，自定义 serde 保住 {decision:…} 信封），单测齐全；`cargo test -p sebas-domain` 47/47 绿。
- 3.1：sebas-node-link 改为 `pub use` 再导出 + `bare_decision` 适配器保住裸字符串线形；`cargo test -p sebas-node-link` 32/32 绿。
- 依赖边调整（为满足「唯一定义在 domain」）：AvailableCommand 从 sebas-acp 移入 sebas-domain，删除 domain→acp 边、新增 acp→domain 与 node-link→domain。sebas-acp 的 Decision 已删除、改用共享类型，escalate 降级 + 未知决定 fail-closed 已落地。
- sebas-dispatch 与 sebas-node 已迁移并通过 `cargo build`（MappingState::phase()、Mapping.desired_mode/effective_mode 类型化、element_type/kind 类型化 match、inbound 未知决定「不解除泊车审批」守卫）。
进度更新（交接第二轮）：sebas-acp / sebas-dispatch / sebas-node / sebas-webui / sebas-im 均已 `cargo build` 通过；**只剩根 crate**。`cargo build` 剩 26 条错误，分布：src/node_link/projection.rs(17)、src/node_link/fleet.rs(8)、src/node_link/driver.rs(5)、src/agent_backend.rs(2)、src/core_channel/server.rs(1)、src/native_dispatch_bridge.rs(1)。典型修法：`resume_mode.as_ref().map(|m| m.as_str())`（dispatch.rs 已修）、`summary.phase.is_terminal()` 取代 `is_terminal_phase(&str)`、`fleet.set_live/note_phase` 收 &str 故传 `phase.as_str()`、MetaEntry 的 desired/effective_mode 是 String 需 `from_wire`/`as_str`、driver.rs:483 decision 需 clone（且未知决定要在调用点挡下）。
未完成（下一步）：
- sebas-webui：约 16 处错误待修。SessionStatus::derive 需改为 `derive(&SessionPhase, Option<&CardPhase>)` 并加 SessionPhase/CardPhase 导入（models.rs:316/317/349/357/392/393、call sites routes.rs:62、api.rs:251/2412）；SessionRow.status 建议改 String；ConversationEntryView.kind/element_type 转换（models.rs）；session_backend.rs:536 map_permission_decision 降级为同类型降级、:697 set_desired_mode、:911 NativeApprovalDecision 桥接、:1219/1220 push_entry、:1394、api.rs:2230 desired_mode。
- sebas-im：frontend.rs 的 element_type/kind 字符串匹配改 typed match（含 turn_to_card_input 未知取值用例）。
- 根 crate：src/core_channel/*、agent_backend.rs、node_link/*（如 :938 unwrap_or_else(ask_mode)）、sebas_state 持久行转换（ask_mode() 现返回 SessionMode，行结构仍是 String）。
- 2.3 / 2.4（webui 部分）/ 2.5 / 3.2（webui 部分）/ 3.3 / 3.4 / 4.2（webui+im 部分）/ 4.3 / 5.1-5.4。
注：`ask_mode()` 现返回 `SessionMode`；`ASK_MODE: &str` 留给磁盘行站岗。构建必须用 `cargo build --workspace`（根 crate 或显式 -p 逐个）。-->

<!-- 状态备注（type-session-vocabularies 收口轮，全部 task 完成）：
本轮（接手 26 条根 crate 编译错误之后）：
- 修完根 crate 全部类型错误 → `cargo build --workspace` 绿；`cargo test --workspace --no-run` 绿。
- 3.4 归零复核：`grep -rn "enum Decision|enum PermissionDecision|enum ApprovalDecision|enum ApprovalAnswer|enum NativeApprovalDecision"` 只剩 `sebas-domain/src/vocabulary.rs:320` 一处（四处并行枚举全部改为 `pub use` 再导出）。
- 2.5 清理：`routes.rs` 的计数桶改为按**类型化** `SessionPhase` 分类（不再拿派生 raw status 词做字面量比较）；`sebas-node/src/session.rs` 的 state 日志行、`sebas-im/src/frontend.rs` 的占位 id、`sebas-dispatch/src/engine/inbound.rs` 的展示标签改为从共享定义取拼写。残余字面量属**其它词表**：systemd `is-active`（service.rs）、日志条目 kind（"error"/"audit"）、agent/provider kind（"echo"/"gemini"）、NodeStatus（"online"）、`SessionRow.status` 的 legacy raw 词表（改它会改 wire，故保留）。
- 4.2：`sebas-im::turn_to_card_input` 由 `(kind.as_str(), element_type.as_str())` 字符串匹配改为**类型化匹配**（未知取值 → 通用文本块；词汇表之外的 `image`/`text` 历史值行为逐字保留，判别只在 `Unknown` 回退分支）；新增单测覆盖六值 + 未知值 + 历史值。webui 侧 `with_normalized_element_type` 早已是 `from_wire` + 类型化 match。
- 4.3：新增 `sebas-webui/src/models.rs::golden_transcript_renders_unchanged`——用重构前黄金转录（六种 element_type + prompt）反序列化并逐条渲染，断言渲染结果与重构前一致，并打印比对输出。
- 3.3：新增三条**边界发送集不变**测试：`core_channel/protocol.rs::approval_answer_send_set_is_unchanged`（webui → 后端：四值原样过线 + escalate 的 reason 不丢）、`sebas-webui/src/session_backend.rs::acp_boundary_send_set_is_unchanged`（core channel → ACP：三值，escalate → allow_once）、`src/node_link/projection.rs::node_boundary_send_set_is_unchanged`（core channel → 节点：三值，escalate → deny，未知 → 不投递）。**失败演示已做**：临时把 `map_permission_decision` 改成原样透传（等于给 ACP 多发 `escalate`），测试立刻失败——`left: {allow_once, allow_session, deny, escalate}` vs `right: {allow_once, allow_session, deny}`，随后已还原。
- 5.1：新增 `golden_spelling_serialization_is_byte_identical`——对 phase/card/mode/kind/element_type 五张拼写表逐值做**逐字节**序列化断言（必须恰好是 `"<拼写>"`，裸字符串、大小写与下划线逐字），并逐字节钉住决定信封（`{"decision":"allow_once"}` / `{"decision":"escalate","reason":"why"}` / 未知值 `{"decision":"yolo"}`）；连同既有两个 `golden_tests.rs` 的结构体往返（phase/mode/decision/element_type 四类载荷）构成 5.1 快照闸门。
- 5.4：三边界抽查证据——core channel：协议帧发送集测试 + 黄金 TurnEntry/RemoteSessionView 载荷往返 + e2e `permission_loop_{allow_once,deny,allow_session}_over_core_channel` 全绿；node link：`golden_link_vocabulary.json`（SessionSummary ×11 + ApprovalAnswer 裸字符串 ×3）往返 + 节点发送集测试 + e2e `remote_node_*` 全绿；webui WS：`sebas-webui/tests/ws_rpc_contract_test.rs` **零改动**通过 + e2e `turn_appends_stream_over_ws` / `native_turn_streams_deltas_to_the_webui` / `claude_turn_streams_multiple_frames_to_the_webui` 全绿。

门禁结果（worktree `/data/workbench/repos-ai/sebas-sg-w3a`）：
- `cargo build --workspace` ✅
- `cargo test -p sebas-domain -p sebas-node-link -p sebas-dispatch -p sebas-node -p sebas-webui -p sebas-im -p sebas-acp -p sebas-agent` ✅ 全绿（sebas-domain 48/48）
- 根 crate `cargo test --no-run` ✅
- 根 crate `cargo test` ⚠️ 14 条 `spawn_env` 失败——**环境性、与本次改动无关**：`src/spawn_env.rs` 未被本 change 触碰（`git diff HEAD` 为空），失败原因是操作员 shell 里导出了 `ANTHROPIC_MODEL`/`ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`/`ANTHROPIC_SMALL_FAST_MODEL`，该测试断言「Off 形态下漏网变量只有 2 个」；把这 6 个变量 unset 后 `cargo test` 全绿（其余 415 条全过）。
- `cargo build -p sebas-node` ✅（e2e 需要这个 bin；裸 `cargo build` 不构建它）
- `invoke testsuite-e2e` ✅ 35 用例全绿
- `invoke testsuite-acceptance` ✅ 10/10 全绿

偏离设计的决策（本轮新增，前轮已列的不重复）：
1. `wire_string_enum!` 新增 `From<&str>` / `From<String>` / `From<&String>`（语义等同既有的不可失败 `from_wire`，已在宏内文档说明）——为消除约 75 处测试夹具的 `.into()` 噪音。**未**新增 `PartialEq<&str>`（那会允许生产代码里出现 spec 禁止的字符串比较，已 grep 确认无此 impl）。
2. `sebas-dispatch/src/engine/inbound.rs` 未知决定的呈现方式由 `Out::HelpText` 改为 `Out::PlainText`：`HelpText` 在 dispatch 层是 no-op（等于静默丢弃），与 spec「unknown decision is surfaced」冲突；改为 PlainText 后未知决定对操作者可见，且仍**不**解除泊车审批。
3. 既有测试 `routing_paths_test.rs::button_cb_unknown_decision_fails_closed_to_deny` 断言「未知词 → PermissionReply{Deny}」，与 spec「no parked approval is silently resolved by a decision that could not be interpreted」直接冲突；按已裁定的偏差 4 改写为 `button_cb_unknown_decision_is_surfaced_without_resolving_the_approval`（断言：被呈现、不发任何 PermissionReply、不翻卡结案、泊车记录未被消费）。
4. 节点边界 `escalate → deny` 保持不变（spec 禁止任何边界开始发送它以前没发过的值）；`sebas-domain/src/vocabulary.rs` 中「ACP、节点链路都降级为 allow_once」的文档措辞与节点侧实际行为（deny）不符，属文档滞后，未改代码语义。

风险：
- `SessionRow.status` 仍保留 legacy raw 词表（`&'static str`，含 `"active"/"dormant"/"spawn-failed"/"spawning"` 的折叠规则）。把它类型化会改变 Idle/WaitingApproval/Exited/Closed/Terminated/Failed 等相位在 JSON 里的取值（今天一律折叠成 `"spawning"`），属 wire 变化，故**刻意保留**；2.5 的 grep 因此在该文件仍会命中这几处**值构造**（非比较）。
- `sebas-im::turn_to_card_input` 对词汇表之外的 `image`/`text` 仍按原串判别（在 `Unknown` 分支内）；这是为保住既有行为，若将来把这两值纳入词表需同步改这里。
- `spawn_env` 的 14 条失败在操作员默认环境下会持续红；本 change 未引入也未修复。-->


## 2. 会话状态词汇类型化

- [x] 2.1 在 `sebas-domain` 定义 `SessionPhase`（覆盖控制面与节点取值并集，`#[serde(rename = "...")]` 逐值保拼写，含未知值路径）；验证：单测覆盖每个取值的往返序列化 + 一个未知值被接受且标记为未知
- [x] 2.2 `SessionInfo.status`、节点 `SessionSummary.phase`、`HostedSession.phase`、`MappingState` 改用 `SessionPhase`，原处 `pub use` 或字段类型替换；验证：`cargo test -p sebas-dispatch -p sebas-node` 全绿，且黄金样本回归通过
- [x] 2.3 webui `SessionStatus::derive` 改为对 `SessionPhase` 的类型化映射（展示 enum 取值集与文案不动）；验证：`cargo test -p sebas-webui` 全绿且 `sebas-webui/tests/api_endpoints_test.rs` 的 `status_label`/`status_slug`/`status_glyph` 断言无改动通过
- [x] 2.4 `SessionMode` 定义移入 `sebas-domain`，`sebas-node-link` 原位再导出，`desired_mode`/`effective_mode`/`SetSessionModeRequest.mode` 等 `String` 字段改用该类型（含未知值容错）；验证：`cargo test -p sebas-node-link -p sebas-node -p sebas-webui` 全绿，且 `sebas-webui/tests/ws_rpc_contract_test.rs` 无改动通过
- [x] 2.5 清理 `"active"` / `"dormant"` / `"spawning"` / `"spawn-failed"` 的散落字面量比较；验证：`grep -rn '"active"\|"dormant"\|"spawning"\|"spawn-failed"' --include=*.rs src/ sebas-*/src/` 在非测试代码区域只剩 enum 定义处的 rename 属性

## 3. 决策词汇合一

- [x] 3.1 在 `sebas-domain` 定义单一决策类型（四值 + 未知值路径），`sebas-node-link` 的原类型改为再导出并保留其公有 API；验证：单测覆盖四值往返 + 未知值接受；`cargo test -p sebas-node-link` 全绿
  - 状态：决定词汇唯一定义落在 `sebas-domain/src/vocabulary.rs`（`PermissionDecision` 四值 + `Unknown`，自定义 serde 保住 `{decision:…}` 信封与未知值原样回吐）；`sebas-node-link` 已改为再导出并加 `bare_decision` 适配器保住裸字符串线形；`cargo test -p sebas-node-link` 32/32 绿。
- [x] 3.2 切换 `sebas_acp::Decision`、webui `PermissionDecision`、`NativeApprovalDecision`、`sebas-agent::ApprovalAnswer` 四处到共享类型，删掉 `sebas-webui/src/session_backend.rs:654 map_permission_decision` 之类的桥接；验证：`cargo test -p sebas-acp -p sebas-webui -p sebas-agent -p sebas-dispatch` 全绿
- [x] 3.3 加「各边界发送集不变」测试：对每条边界（core channel → ACP、core channel → 节点、webui → 后端）断言序列化输出集合与 1.3 的基线一致，且 `escalate` 到 ACP 仍降级为 `allow_once`；验证：测试通过，且人为让某边界多发一个值（如给 ACP 送 escalate）时测试失败（附一次失败演示）
- [x] 3.4 复核 5 份并行枚举已归零；验证：`grep -rn "enum Decision\|enum PermissionDecision\|enum ApprovalDecision\|enum ApprovalAnswer\|enum NativeApprovalDecision"` 只剩共享定义一处

## 4. 回合内容词汇类型化

- [x] 4.1 在 `sebas-domain` 定义回合内容的 `element_type` 与 `kind` 类型（六值 + kind 取值 + 未知值路径，逐值保拼写含 `"permission_mode_result"`）；验证：单测覆盖每值往返 + 未知值
- [x] 4.2 `TurnEntry` 字段改用新类型，webui / im 的字符串匹配改为 `match`，未知取值渲染为通用块；验证：`cargo test -p sebas-webui -p sebas-im -p sebas-dispatch` 全绿，且 `sebas-im` 的 `turn_to_card_input` 路径有未知取值用例
- [x] 4.3 回归既有转录内容的读取：用重构前写下的转录（含全部六种 element type）反序列化并渲染；验证：每条渲染结果与重构前一致，附比对输出

## 5. 全量回归与收口

- [x] 5.1 线格式快照比对（覆盖 phase/mode/decision/element_type 四类取值的序列化载荷）；验证：与重构前逐字节一致，无 diff
- [x] 5.2 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
- [x] 5.3 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应 enum 定位并记录
- [x] 5.4 复核零变化基线：抽查三个边界（core channel、node link、webui WS）的实际 JSON 载荷与重构前对比；验证：字段名与取值拼写零差异，结论附在 PR 描述

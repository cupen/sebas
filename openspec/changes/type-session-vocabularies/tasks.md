## 1. 取值审计与黄金样本

- [ ] 1.1 审计全部会话状态字面量：产出清单（取值 → 出现位置 → 生产者/消费者），覆盖 `status`（`spawning`/`active`/`dormant`/`spawn-failed`）、节点 `phase`（七值）、卡片 phase（`SEED`/`OnIt`/`DONE`/`CrossMark`）、`MappingState`（四值）、`SessionLifecycle`、`NodeStatus`、`SessionStatus`（展示七值）；验证：清单里每个取值都有生产者与至少一个消费者，且与 proposal Why 的计数一致（`"active"` 78 处 / 22 文件）
- [ ] 1.2 审计 mode 与回合内容词汇：`desired_mode`/`effective_mode`/`SetSessionModeRequest.mode` 的全部出现处，以及 `TurnEntry.element_type`（六值）与 `kind` 的全部出现处；验证：清单含 `sebas-node-link::SessionMode` 的既有取值，且标注了每处是 String 还是枚举
- [ ] 1.3 审计 5 份审批决策枚举的实际**发送集**（不是声明集）：`sebas_acp::Decision`、`webui::PermissionDecision`、`NativeApprovalDecision`、`sebas-agent::ApprovalAnswer`、`node-link::ApprovalDecision`；验证：产出「每条边界实际序列化过哪些值」的表格，作为 D5 发送集不变断言的基线
- [ ] 1.4 用 1.1-1.3 的取值集生成黄金样本文件（每个取值一个序列化样本），作为后续钉测试的输入；验证：样本能被当前未改动的代码反序列化回原值

## 2. 会话状态词汇类型化

- [ ] 2.1 在 `sebas-domain` 定义 `SessionPhase`（覆盖控制面与节点取值并集，`#[serde(rename = "...")]` 逐值保拼写，含未知值路径）；验证：单测覆盖每个取值的往返序列化 + 一个未知值被接受且标记为未知
- [ ] 2.2 `SessionInfo.status`、节点 `SessionSummary.phase`、`HostedSession.phase`、`MappingState` 改用 `SessionPhase`，原处 `pub use` 或字段类型替换；验证：`cargo test -p sebas-dispatch -p sebas-node` 全绿，且黄金样本回归通过
- [ ] 2.3 webui `SessionStatus::derive` 改为对 `SessionPhase` 的类型化映射（展示 enum 取值集与文案不动）；验证：`cargo test -p sebas-webui` 全绿且 `sebas-webui/tests/api_endpoints_test.rs` 的 `status_label`/`status_slug`/`status_glyph` 断言无改动通过
- [ ] 2.4 `SessionMode` 定义移入 `sebas-domain`，`sebas-node-link` 原位再导出，`desired_mode`/`effective_mode`/`SetSessionModeRequest.mode` 等 `String` 字段改用该类型（含未知值容错）；验证：`cargo test -p sebas-node-link -p sebas-node -p sebas-webui` 全绿，且 `sebas-webui/tests/ws_rpc_contract_test.rs` 无改动通过
- [ ] 2.5 清理 `"active"` / `"dormant"` / `"spawning"` / `"spawn-failed"` 的散落字面量比较；验证：`grep -rn '"active"\|"dormant"\|"spawning"\|"spawn-failed"' --include=*.rs src/ sebas-*/src/` 在非测试代码区域只剩 enum 定义处的 rename 属性

## 3. 决策词汇合一

- [ ] 3.1 在 `sebas-domain` 定义单一决策类型（四值 + 未知值路径），`sebas-node-link` 的原类型改为再导出并保留其公有 API；验证：单测覆盖四值往返 + 未知值接受；`cargo test -p sebas-node-link` 全绿
- [ ] 3.2 切换 `sebas_acp::Decision`、webui `PermissionDecision`、`NativeApprovalDecision`、`sebas-agent::ApprovalAnswer` 四处到共享类型，删掉 `sebas-webui/src/session_backend.rs:654 map_permission_decision` 之类的桥接；验证：`cargo test -p sebas-acp -p sebas-webui -p sebas-agent -p sebas-dispatch` 全绿
- [ ] 3.3 加「各边界发送集不变」测试：对每条边界（core channel → ACP、core channel → 节点、webui → 后端）断言序列化输出集合与 1.3 的基线一致，且 `escalate` 到 ACP 仍降级为 `allow_once`；验证：测试通过，且人为让某边界多发一个值（如给 ACP 送 escalate）时测试失败（附一次失败演示）
- [ ] 3.4 复核 5 份并行枚举已归零；验证：`grep -rn "enum Decision\|enum PermissionDecision\|enum ApprovalDecision\|enum ApprovalAnswer\|enum NativeApprovalDecision"` 只剩共享定义一处

## 4. 回合内容词汇类型化

- [ ] 4.1 在 `sebas-domain` 定义回合内容的 `element_type` 与 `kind` 类型（六值 + kind 取值 + 未知值路径，逐值保拼写含 `"permission_mode_result"`）；验证：单测覆盖每值往返 + 未知值
- [ ] 4.2 `TurnEntry` 字段改用新类型，webui / im 的字符串匹配改为 `match`，未知取值渲染为通用块；验证：`cargo test -p sebas-webui -p sebas-im -p sebas-dispatch` 全绿，且 `sebas-im` 的 `turn_to_card_input` 路径有未知取值用例
- [ ] 4.3 回归既有转录内容的读取：用重构前写下的转录（含全部六种 element type）反序列化并渲染；验证：每条渲染结果与重构前一致，附比对输出

## 5. 全量回归与收口

- [ ] 5.1 线格式快照比对（覆盖 phase/mode/decision/element_type 四类取值的序列化载荷）；验证：与重构前逐字节一致，无 diff
- [ ] 5.2 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
- [ ] 5.3 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应 enum 定位并记录
- [ ] 5.4 复核零变化基线：抽查三个边界（core channel、node link、webui WS）的实际 JSON 载荷与重构前对比；验证：字段名与取值拼写零差异，结论附在 PR 描述

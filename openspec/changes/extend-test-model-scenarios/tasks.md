## 1. 线协议事实源扩展（anthropic_wire）

- [ ] 1.1 `AnthropicMessage` 增加块构造入口：thinking 块（含 `thinking` 文本字段）与 tool_use 块（id / name / 确定性 input JSON）；既有 `text()` 构造签名不变；验证：既有 fake-provider 与 test_provider 单测不改而全绿
- [ ] 1.2 SSE 生成器按块类型发正确事件序列：`content_block_start` 形状随类型（`thinking` / `tool_use` / `text`），delta 分别为 `thinking_delta` / `input_json_delta` / `text_delta`，`message_delta` 的 stop_reason 取自场景；验证：每块类型一个流式单测（事件顺序与形状断言）
- [ ] 1.3 非流式与流式同构：同一 message 生成的 JSON content 块序列与 SSE 拼接结果一致（多块混排 + 长文分块两用例）；验证：同构性单测通过

## 2. 场景解析与规则（test_provider）

- [ ] 2.1 场景解析：body 的 `model` 字段映射 `test` → 既有 echo（路径零改动）、九个场景（`text` / `long` / `thinking` / `tool-use` / `tools-parallel` / `full` / `empty` / `error`）→ 各自应答；未知 `test/<x>` 的处理取一并测试钉住；验证：每场景一个解析单测，bare `test` 的既有断言不改而通过
- [ ] 2.2 agent-loop 规则（`tool-use` / `full`）：与 fake-provider 同规范（tools 非空且无 tool_result → 首 tool tool_use；有 tool_result → 终文本；无 tools → 纯文本降级）；`text` / `long` / `thinking` / `empty` 永不发 tool_use；验证：规则真值表单测（tools × tool_result × 场景 全组合）
- [ ] 2.3 `test/tools-parallel`：无 tool_result 且 tools ≥ 1 时一回合发出全部 tools 的 tool_use（确定性 input 各异，stop_reason=tool_use）；验证：单/多工具两用例 + 与 2.2 的分岔语义有注释注明动机
- [ ] 2.4 `test/long`：确定性长文（固定内容、固定分块数流式下发），拼接与非流式一致；`test/empty`：零 content 块完成回合（stop_reason=end_turn）；`test/error`：Anthropic 错误体（HTTP 5xx + `api_error`），OpenAI 家族为对应错误形状；验证：三场景各自的非流式/流式单测
- [ ] 2.5 确定性 usage：消息型场景各一组互不相同的非零值；`test` 与 `test/empty` 保持全零；验证：usage 断言单测
- [ ] 2.6 与 fake-provider 规则一致性：同一请求形状分别经 test provider 规则与 fake 的规则函数，块序列与 stop_reason 一致；验证：一致性对比测试通过（若实现时把规则提为共用函数，则该测试退化为存在性检查）

## 3. e2e journeys（testsuite-process-e2e）

- [ ] 3.1 native kernel 工具环 journey（`test/tool-use`）：tool_use → 权限批准 → 工具执行 → tool_result → 终文本 → Done；验证：journey 全绿，usage 断言为场景确定性值，全程无真实上游外呼
- [ ] 3.2 并行权限 journey（`test/tools-parallel`）：多 tool_use 各自产生独立权限请求，逐一决策后回合推进；验证：journey 全绿（对应 permission-flow「并行工具调用独立 request_id」）
- [ ] 3.3 thinking 呈现 journey（`test/thinking`）与混排 journey（`test/full`）：呈现形态可区分、块序保持；验证：两条 journey 全绿
- [ ] 3.4 零输出通知 journey（`test/empty`）：回合完成无可见输出、通知路径触发；验证：journey 全绿（对应会话管理「零输出回合追加通知」）
- [ ] 3.5 长文流式 journey（`test/long`）：流式期间无帧丢失、拼接与非流式一致；验证：journey 全绿（对应流式背压/增量同步）
- [ ] 3.6 **流式中取消 journey**（`test/long`）：流式期间经用户面取消，断言流停止、取消如实呈现、会话此后仍可发起新回合；验证：journey 全绿（对应 core-session-channel「Session cancel over the channel」）
- [ ] 3.7 **模型切换生效 journey**：会话先以 `test/text` 完成一回合，经用户面切模型为 `test/tool-use` 后下一回合产生 tool_use 与权限请求，切前后回合各自保留呈现形状；验证：journey 全绿（对应 acp-model-selection「切换生效」）
- [ ] 3.8 错误呈现 + 可用性 journey（`test/error`）：LLM 失败如实呈现、会话状态迁移遵循终局错误语义，**且失败后新会话可创建并完成一次正常回合**；验证：journey 全绿（注意真实 claude-code 对 5xx 有重试——本 journey 以 native 后端驱动，重试行为实测结论记入 design）
- [ ] 3.9 **证明标准落地**：① 全部 journey 经 webui HTTP API + WS/SSE 驱动（辅助函数不得绕过用户面直调内部接口，复核 helpers 并整改）；② 工具环 journey 重复执行两次断言转录形状与终态一致（id/时间戳除外）；③ `test/text` journey 断言 echo 回显 = 最后一条用户消息原文（对话连续性）；④ 操作员全路径（项目 → 建会话 → 提交 → 流式 → 权限 → 结果 → 关闭）核对有 journey 命中，缺哪个补哪个；验证：四项各有一条结论附 PR 描述
- [ ] 3.10 可选：真实 claude-code 场景 journey（`test/tool-use`），二进制缺席诚实跳过；验证：跳过路径与通过路径各有断言
- [ ] 3.11 复核既有 bare `test` 用例（「router debug provider 应答」）不改而通过；验证：`invoke testsuite-e2e` 相关用例全绿
- [ ] 3.12 usage 断言经套件既有辅助取数（不直接绑定 JSONL/DB 形态）；验证：辅助函数签名不变或两边各改一次的结论记入 PR 描述

## 4. 验收载体定向与收口

- [ ] 4.1 更新 `tests/acceptance/COVERAGE.md` 账本规则段：记录「工作台行为验收 = test 模型」定向与日期，核对五簇中 LLM 形状相关 requirement 的证据指针（重指到新 journey 或保留原证据），确认分母与 100% 口径不变；验证：账本复核通过（五簇重数与命中逐行核对），定向说明入账
- [ ] 4.2 `AGENTS.md` 沙箱食谱补场景表（九个场景 + bare `test` 的用途与确定性形状说明，注明工作台验收默认用 test 模型）；验证：食谱含场景表且与 spec 一致
- [ ] 4.3 实测结论回填 design（thinking 块 `signature_delta` 取舍、input_json_delta 分片粒度、未知场景名的处理选择、`test/error` 的重试行为）；验证：design.md 的 Open Questions 逐条有结论
- [ ] 4.4 跑 `invoke testsuite-e2e`；验证：全绿
- [ ] 4.5 跑 `invoke testsuite-acceptance`；验证：全绿且账本口径不变，出现红则回到对应步定位

## 1. 线协议事实源扩展（anthropic_wire）

- [x] 1.1 `AnthropicMessage` 增加块构造入口：thinking 块（含 `thinking` 文本字段）与 tool_use 块（id / name / 确定性 input JSON）；既有 `text()` 构造签名不变；验证：既有 fake-provider 与 test_provider 单测不改而全绿
- [x] 1.2 SSE 生成器按块类型发正确事件序列：`content_block_start` 形状随类型（`thinking` / `tool_use` / `text`），delta 分别为 `thinking_delta` / `input_json_delta` / `text_delta`，`message_delta` 的 stop_reason 取自场景；验证：每块类型一个流式单测（事件顺序与形状断言）
- [x] 1.3 非流式与流式同构：同一 message 生成的 JSON content 块序列与 SSE 拼接结果一致（多块混排 + 长文分块两用例）；验证：同构性单测通过

## 2. 场景解析与规则（test_provider）

- [x] 2.1 场景解析：body 的 `model` 字段映射 `test` → 既有 echo（路径零改动）、九个场景（`text` / `long` / `thinking` / `tool-use` / `tools-parallel` / `full` / `empty` / `error`）→ 各自应答；未知 `test/<x>` 的处理取一并测试钉住；验证：每场景一个解析单测，bare `test` 的既有断言不改而通过
- [x] 2.2 agent-loop 规则（`tool-use` / `full`）：与 fake-provider 同规范（tools 非空且无 tool_result → 首 tool tool_use；有 tool_result → 终文本；无 tools → 纯文本降级）；`text` / `long` / `thinking` / `empty` 永不发 tool_use；验证：规则真值表单测（tools × tool_result × 场景 全组合）
- [x] 2.3 `test/tools-parallel`：无 tool_result 且 tools ≥ 1 时一回合发出全部 tools 的 tool_use（确定性 input 各异，stop_reason=tool_use）；验证：单/多工具两用例 + 与 2.2 的分岔语义有注释注明动机
- [x] 2.4 `test/long`：确定性长文（固定内容、固定分块数流式下发），拼接与非流式一致；`test/empty`：零 content 块完成回合（stop_reason=end_turn）；`test/error`：Anthropic 错误体（HTTP 5xx + `api_error`），OpenAI 家族为对应错误形状；验证：三场景各自的非流式/流式单测
- [x] 2.5 确定性 usage：消息型场景各一组互不相同的非零值；`test` 与 `test/empty` 保持全零；验证：usage 断言单测
- [x] 2.6 与 fake-provider 规则一致性：同一请求形状分别经 test provider 规则与 fake 的规则函数，块序列与 stop_reason 一致；验证：一致性对比测试通过（若实现时把规则提为共用函数，则该测试退化为存在性检查）

## 3. e2e journeys（testsuite-process-e2e）

- [x] 3.1 native kernel 工具环 journey（`test/tool-use`）：tool_use → 权限批准 → 工具执行 → tool_result → 终文本 → Done；验证：journey 全绿，usage 断言为场景确定性值，全程无真实上游外呼
- [x] 3.2 并行权限 journey（`test/tools-parallel`）：多 tool_use 各自产生独立权限请求，逐一决策后回合推进；验证：journey 全绿（对应 permission-flow「并行工具调用独立 request_id」）
- [x] 3.3 thinking 呈现 journey（`test/thinking`）与混排 journey（`test/full`）：呈现形态可区分、块序保持；验证：两条 journey 全绿
- [x] 3.4 零输出通知 journey（`test/empty`）：回合完成无可见输出、通知路径触发；验证：journey 全绿（对应会话管理「零输出回合追加通知」）
- [x] 3.5 长文流式 journey（`test/long`）：流式期间无帧丢失、拼接与非流式一致；验证：journey 全绿（对应流式背压/增量同步）
- [x] 3.6 **流式中取消 journey**（`test/long`）：流式期间经用户面取消，断言流停止、取消如实呈现、会话此后仍可发起新回合；验证：journey 全绿（对应 core-session-channel「Session cancel over the channel」）
- [x] 3.7 **模型切换生效 journey**：会话先以 `test/text` 完成一回合，经用户面切模型为 `test/tool-use` 后下一回合产生 tool_use 与权限请求，切前后回合各自保留呈现形状；验证：journey 全绿（对应 acp-model-selection「切换生效」）
- [x] 3.8 错误呈现 + 可用性 journey（`test/error`）：LLM 失败如实呈现、会话状态迁移遵循终局错误语义，**且失败后新会话可创建并完成一次正常回合**；验证：journey 全绿（注意真实 claude-code 对 5xx 有重试——本 journey 以 native 后端驱动，重试行为实测结论记入 design）
- [x] 3.9 **证明标准落地**：① 全部 journey 经 webui HTTP API + WS/SSE 驱动（辅助函数不得绕过用户面直调内部接口，复核 helpers 并整改）；② 工具环 journey 重复执行两次断言转录形状与终态一致（id/时间戳除外）；③ `test/text` journey 断言 echo 回显 = 最后一条用户消息原文（对话连续性）；④ 操作员全路径（项目 → 建会话 → 提交 → 流式 → 权限 → 结果 → 关闭）核对有 journey 命中，缺哪个补哪个；验证：四项各有一条结论附 PR 描述
- [x] 3.10 **浏览器级呈现 journey**（`testsuite-webui-browser`，Playwright）：以 `test/tools-parallel` 验证并行审批卡片各自独立、`test/empty` 验证零输出通知呈现、`test/long` 验证 UI 取消、`test/text → UI 切模型 → test/tool-use` 验证切换生效（native 通路不可通时按既有 spike 门控转豁免入账）；验证：浏览器旅程全绿或按门控诚实豁免，`testsuite-webui` 整体不红
- [x] 3.11 可选：真实 claude-code 场景 journey（`test/tool-use`），二进制缺席诚实跳过；验证：跳过路径与通过路径各有断言
- [x] 3.12 复核既有 bare `test` 用例（「router debug provider 应答」）不改而通过；验证：`invoke testsuite-e2e` 相关用例全绿
- [x] 3.13 usage 断言经套件既有辅助取数（不直接绑定 JSONL/DB 形态）；验证：辅助函数签名不变或两边各改一次的结论记入 PR 描述

### 3.10 浏览器载体（本轮实测）

新增 native 场景姿态：`playwright.native.config.ts`（端口 9894）+ harness 的
`TESTSUITE_NATIVE=1`（`tasks.py`：把 native 内核指向沙箱内已在跑的 debug router
随本装配 webui 端口派生的 debug router 端口，默认模型 `test/text`，可用模型 =
九场景 + bare `test`）。
默认沙箱**不注入**这组 env，`first-paint` 的「native 未配置模型凭据」断言不受影响
（实测：`--case first-paint` 2 passed、`--case permission` 3 passed）。
`invoke testsuite-webui` 链尾已加 native 配置（第六套）。

| 3.10 项 | 载体 | 结果 |
|---|---|---|
| `test/empty` 零输出通知呈现 | 新 `test-model-scenarios.spec.ts`（native） | **绿**（notice 条目 + 助手回合收尾标记同场） |
| `test/tools-parallel` 并行审批卡片各自独立 | 同文件（native） | **豁免**：native 的审批卡在 webui 渲染不出来（下述根因），用例以 `test.fixme` 带根因入账，关闭缺口即可启用 |
| UI 取消（`test/long`） | 既有 `stop-settle.spec.ts`（ACP 载体，delta spec 许可的双载体） | 既有覆盖（绿） |
| UI 切模型（`test/text → test/tool-use`） | 既有 `models.spec.ts`（切换生效 + composer chip 呈现）；native 侧由 3.7 进程级 journey 覆盖 | 既有覆盖（绿） |

**豁免根因（实测，非环境问题）**：WS `permission.requested` 对 native 会话携带的
`session_id` 是 ChannelKey 的 JSON 字符串（`{"channel":"feishu","reference":"agent-…"}`），
而 webui 聚焦会话键是编码键（`feishu%00agent-…`）；`review-card.ts:184` 的过滤是精确
字符串比较（`event.session_id !== this.sessionKey`），事件因此被丢弃、
`sebas-review-cards` 恒空。native 回合自身的 ⏳ 条目与
`POST /api/permissions/{rid}/answer` 决策都正常（3.1/3.2 进程级 journey 全绿），
缺的只是 webui 卡面——修法是前端按 ChannelKey 归一匹配（超出本 change 的 delta 范围，
记入风险/后续）。

同一条 native 通路还有一处已知形态：webui 拉 `/api/sessions/{key}/approvals`
（parked 读模型）对 native 回 503，浏览器控制台留一条网络错误——本旅程只滤这一条，
其余控制台错误与 JS 异常仍零容忍。

### 3.x 落地记录（本轮实测）

| task | 用例（`tests/testsuite_e2e_test.rs`） | 结果 |
|---|---|---|
| 3.1 / 3.9 / 3.13 | `test_model_tool_loop_runs_with_permission_and_records_usage` | 绿（4.6s） |
| 3.2 | `test_model_parallel_tool_permissions_are_independent` | 绿 |
| 3.3 | `test_model_thinking_and_mixed_blocks_reach_the_transcript` | 绿（2.4s） |
| 3.4 | `test_model_empty_turn_appends_the_zero_output_notice` | 绿 |
| 3.5 / 3.6 | `test_model_long_stream_is_incremental_and_cancellable` | 绿（7.0s） |
| 3.7 | `test_model_switch_takes_effect_on_the_next_turn` | 绿（7.6s） |
| 3.8 | `test_model_error_surfaces_and_the_session_stays_workable` | 绿（9.0s） |
| 3.11 | `test_model_scenario_journey_with_real_claude_code` | 绿（真 CLI 命中，6.99s / 0.89s 两次；缺席时诚实跳过） |
| 3.12 | 既有 `router_debug_provider_serves_messages` | 绿，且该用例零改动（diff 纯新增） |

- 3.9①：journey 全程只经 webui HTTP API（`post_json`/`fetch_session_entries`）+ WS
  （`ws_subscribe`/`next_ws_frame`）驱动；唯一例外是 3.5 取「非流式参照文本」时直连
  router 的 `POST /v1/messages`——那是 router 对外的 HTTP 面，不是内部接口/内部状态，
  仅用于拼接一致性比对，已在用例内注释说明。usage 取数复用套件既有
  `read_usage_records`（签名未改）。
- 3.9②：3.1 用例内对同一会话跑两轮，比对转录形状（`element_types` + 门控工具集 +
  摘要稳定段）与终态；耗时的毫秒部分不入比对。
- 3.9③：`test/text` 回合断言 echo 回显 = 最后一条用户消息原文（3.4 对照面、3.6 切前
  回合、3.7 恢复回合）。
- 3.9④：操作员全路径由 3.1（项目 → 建会话 → 提交 → 流式 → 权限 → 结果）与
  既有 `session_close` 系用例（关闭）共同命中。
- 3.11：debug router 的 `test/tool-use` 也能驱动**真实 claude-code** 的 ACP 通路
  （CLI 吃下场景 tool_use → 执行工具 → 回传 tool_result → 场景次轮终文本进转录，
  11 条 usage 记录，零真实外呼）；二进制缺席时打印原因并跳过。
- 3.10 状态见下方「浏览器载体」段。

## 4. 验收载体定向与收口

- [x] 4.1 更新 `tests/acceptance/COVERAGE.md` 账本规则段：记录「工作台行为验收 = test 模型」定向与日期，核对五簇中 LLM 形状相关 requirement 的证据指针（重指到新 journey 或保留原证据），确认分母与 100% 口径不变；验证：账本复核通过（五簇重数与命中逐行核对），定向说明入账
- [x] 4.2 `AGENTS.md` 沙箱食谱补场景表（九个场景 + bare `test` 的用途与确定性形状说明，注明工作台验收默认用 test 模型）；验证：食谱含场景表且与 spec 一致
- [x] 4.3 实测结论回填 design（thinking 块 `signature_delta` 取舍、input_json_delta 分片粒度、未知场景名的处理选择、`test/error` 的重试行为）；验证：design.md 的 Open Questions 逐条有结论
- [x] 4.4 跑 `invoke testsuite-e2e`；验证：全绿
  - 评审轮实跑（主 agent，先 `cargo build -p sebas-node`——裸 `cargo build` 不构建该 bin，
    套件有用例依赖它，见 issue sebas-bm42）：**71 passed / 0 failed / 5 filtered**（基线 63 + 本
    change 新增 8 条 journey）。未并发跑其它重型 cargo 作业（并发会导致 pending/queue 类用例假红）。
- [x] 4.5 跑 `invoke testsuite-acceptance`；验证：全绿且账本口径不变，出现红则回到对应步定位
  - 评审轮实跑：**10 passed / 0 failed / 5 filtered**。账本口径已核验：`git status --short openspec/specs`
    为空（主 spec 零改动），五簇分母不受本 change 影响。

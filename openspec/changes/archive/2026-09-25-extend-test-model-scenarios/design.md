## Context

动机见 `proposal.md` — Why。设计相关的事实：

1. **注入机制已存在且无需改动**：`--debug` / `[router] debug` 注入 provider `test`（base_url 为 `router://self` 哨兵，proxy 短路应答）+ `test → test` 优先路由（`sebas-router/src/debug.rs`）。**关键事实：`test/<anything>` 形式的模型已经路由到该 provider**（路由按模型前缀匹配），且请求 body 原样到达——即 body 的 `model` 字段带着完整场景名。场景选型因此零新增配置。
2. **响应形状已有共用事实源**：`AnthropicMessage`（`anthropic_wire.rs`）由 fake-provider-upstream 1.1 从 test_provider 下沉而来，`test_provider.rs` 与 `fake_provider.rs` 共用；`content` 本就是 `Vec<Value>` 块数组，SSE 生成器遍历块——但当前只处理 `text` 一种块（`content_block_start` 硬编码 `"type": "text"`）。
3. **bare `test` 是已文档化契约**：AGENTS.md 沙箱食谱明写「`POST /v1/messages` with `{"model":"test"}` → 200 `msg_test_debug`」，`tests/testsuite_e2e_test.rs` 有对应断言。
4. **agent-loop 规则的既有规范**：`fake-provider-upstream`「内置 agent-loop 确定性规则」——tools 非空且无 tool_result → 首个 tool 的 tool_use；有 tool_result → 终文本。本 change 的场景规则与它**同规范**（同一套语义，两处实现分别服务自答与拨号两种拓扑）。
5. **e2e 基建**：`testsuite-process-e2e` 已有「router debug provider 应答」（bare test）与「agent-loop journey」（真实 claude-code + fake 上游，缺席诚实跳过）的先例；native 后端会话的进程级驱动有既有 helper。
6. **usage 断言的现状**：既有确定性用量用例读 `router-usage.jsonl`；`persist-router-usage` 计划把它迁到 `usage.db`（落地顺序未定）。

## Goals / Non-Goals

**Goals:**

- 让 e2e 能在零进程、零凭据、零网络的条件下验收：thinking 呈现、tool_use 权限流、多块混排、usage 结算。
- 场景与 fake-provider 的 agent-loop 规则同规范，避免两处语义漂移。

**Non-Goals:**

- 不动 fake-provider 子命令；不做场景文件编排；不让 OpenAI 家族完整对齐（降级即可）。

## Decisions

### D1 场景选型走 model 名：`test/<scenario>`，bare `test` 逐字保留

`test/<anything>` 路由已存在、body 原样到达 provider——场景名搭既有便车，零配置、零新键，任何客户端（agent 的模型配置、curl、Playwright）都能用同一个机制选剧本。**bare `test` 的 echo 行为逐字不变**：它是 AGENTS.md 食谱与既有 e2e 的已文档化契约，改它等于破坏自己的验收工具。**被否备选**：场景放请求 header 或 body 扩展字段——agent 客户端（真实 claude-code、native kernel）不会替你发这个字段，而模型名是所有客户端都会传的。

场景集：`test/text`（纯文本，含 echo 句式）、`test/thinking`（thinking + text）、`test/tool-use`（tool 环）、`test/full`（thinking + text + tool 环）。bare `test` 不参与 agent-loop 规则（它不是场景，是回显器）。

### D2 agent-loop 规则只在含 tool_use 的场景生效，语义与 fake-provider 同规范——并行场景是**有意的分岔**

`test/tool-use` 与 `test/full`：tools 非空且无 tool_result → 首个 tool 的 `tool_use`（stop_reason=tool_use，input 为确定性对象）；有 tool_result → 终文本（stop_reason=end_turn）；无 tools → 纯文本降级（不报错）。`test/text` / `test/thinking` / `test/long` / `test/empty` **永不发 tool_use**——每个场景职责单一，journey 断言才立得住（thinking journey 不会突然冒出权限卡）。**与 fake-provider 的一致性用测试钉住**：同一请求形状分别喂 test provider 与 fake 的规则函数，块序列与 stop_reason 必须一致（两处实现共享 `AnthropicMessage` 事实源后，漂移面只剩规则判断本身）。

**`test/tools-parallel` 是有意的分岔**：一回合发出请求中**全部** tools 的 `tool_use`（确定性 input 各异）。它存在的原因是覆盖账本：`permission-flow` 有「并行工具调用各自独立 request_id」的核心 requirement，fake-provider 的「只回首个 tool」规则驱动不了它。分岔只加不改——fake-provider 的语义与 spec 逐字不动，分岔写在 test 模型侧并注明动机。

### D2b 场景集由核心五簇的 LLM 形状相关 requirement 反推（覆盖矩阵）

「100% 不打折扣」的可执行形式：每个场景对应账本里一类明确的 requirement，不存在「为酷而加」的场景；反过来说，凡 LLM 响应形状决定其行为的核心 requirement，都必须有一个场景能驱动它。当前映射：

| 场景 | 驱动的核心 requirement（簇） |
|---|---|
| `test/text` | 回合正文呈现、composer 投递（③）、**对话连续性证明**（echo 回显 = 历史送达） |
| `test/long` | 流式背压与队列（①）、增量同步/折叠呈现（③）、**会话取消在流中生效**（⑤ core-session-channel） |
| `test/thinking` | thinking 呈现与转录区分（③） |
| `test/tool-use` | 权限三值流、gated call approval、工具环到 Done（①③） |
| `test/tools-parallel` | 并行工具调用独立 request_id（③ permission-flow） |
| `test/full` | 混排顺序、turn 身份/折叠标题（③） |
| `test/empty` | 零输出回合追加通知（①） |
| `test/error` | 终局错误如实呈现（①）、诚实降级面 |
| bare `test` | 既有 debug 应答契约（⑤通道，不动） |

**场景组合驱动的 journey**（行为类 requirement 常需要两个场景接力）：模型切换生效（② acp-model-selection）= `test/text` → 用户面切模型 → `test/tool-use` 下一回合行为改变；稳定性证明 = 工具环 journey 重复执行一致性。

**不进矩阵的**（显式豁免，非 LLM 形状决定）：model **目录与可选列表**（来自 agent catalog 探测，非 LLM 响应——注意「切换是否生效」不在豁免内，它由场景组合证明）、容量上限/双开竞争/休眠恢复（状态机行为，既有非 LLM 测试覆盖）、provider 治理（state methods）、拨号透传（fake 上游 journey 的既有职责，见 D7）。**矩阵随账本复核更新**：每次五簇重数变化时，新 LLM 形状 requirement 若无场景可驱动，就是场景集的缺口（⚠️），与账本缺口同等级对待。

### D3 块类型扩展落在 `AnthropicMessage`（共用事实源），fake-provider 同受益

`anthropic_wire.rs` 增加块构造入口（thinking 块、tool_use 块）与 SSE 生成器的分块类型处理（`content_block_start` 按 `thinking` / `tool_use` / `text` 分别发正确形状；delta 分别为 `thinking_delta` / `input_json_delta` / `text_delta`）。**理由**：test provider 与 fake_provider 共用这个事实源正是当初下沉的目的——在旁边另写一套块知识会重新制造漂移。fake-provider 的既有 text 行为不受影响（新增入口不改旧签名）。**注意**：thinking 块的 SSE 事件里 `content_block_start` 的形状与 text 不同（含 `thinking` 字段），`signature_delta` 是否需要由真实 claude-code 的实测决定——实现时以「真实客户端能消费」为准，实测结论记入本文件。

### D4 流式保真：每块正确的事件序列 + 确定性分块

文本按固定长度切块（例如每 8 字符一个 `text_delta`），thinking 同理；`tool_use` 的 input 经 `input_json_delta` 分片下发；`message_delta` 的 stop_reason 随场景（tool_use / end_turn）。确定性分块让「流式卡片更新次数」「增量拼接结果」可精确断言——这正是 debug provider 相对真上游的价值。非流式与流式必须同构（同一 `AnthropicMessage` 事实源生成，spec「identical requests produce identical responses」）。

### D5 OpenAI 家族降级：thinking → 纯文本，文档写明

chat 协议没有 thinking 的对应物；Responses 档的 debug 应答本来就只承诺回显级形状。降级规则：OpenAI 家族上所有场景都只产出 text（+ tool_calls 映射，chat 协议的 tool_use 对应物），thinking 场景不报错、静默降为纯文本。**理由**：agent 的实际拨号面是 Anthropic（`ANTHROPIC_BASE_URL` 指向 router）；OpenAI 家族在 debug 拓扑里没有真实消费者，完整对齐是无收益成本。

### D6 确定性非零 usage：每场景固定值

bare `test` 今天 usage 全零——保留（既有契约）。四个场景各给一组固定非零值（input/output/cache_read/cache_creation），使 router 用量记录链路（usage sink → 记录字段）在 journey 里有真实形状可断言。数值选取以「各字段互不相同」为准，断言时能区分字段错位。

### D7 e2e journeys 的断言面与排序依赖

三个 journey（native 工具环 + 权限流、thinking 呈现、混排）走既有 `testsuite-process-e2e` 的沙箱与 helper；真实 claude-code 的场景 journey 为可选（缺席诚实跳过，与 agent-loop journey 同法）。**usage 断言不直接绑定存储形态**：经套件既有的用量断言辅助取数——`persist-router-usage` 落地前读 JSONL、落地后查库，辅助函数换实现、journey 断言不动。因此本 change 与 `persist-router-usage` 互不阻塞（软依赖，仅辅助函数可能需要两边各改一次）。

### D8 载体定向：工作台行为验收 = test 模型；fake 上游只管拨号路径

本条是用户指令的规范化：**今后涉及 agent 回合的工作台验收，LLM 一律用内置 `test` 模型**。边界划在「验收对象」上：验收**工作台行为**（回合呈现、权限流、会话状态迁移）→ test 模型；验收**拨号透传链路**（header 过滤、key 注入、SSE 透传、限流/用量结算的传输面）→ fake 上游 / fake-provider 保持既有职责。这条边界与两个机制的设计初衷一致：test 模型零进程零凭据（适合行为矩阵的快速全覆盖），fake-provider 走真实 HTTP 拨号（适合传输面）。

**不打折扣 = 口径不降**：核心五簇账本保持 100% 硬指标。载体切换只改变**证据指针**（某 requirement 的命中证据从旧 journey 重指到 test 模型 journey），requirement 分母与命中率口径不变；`COVERAGE.md` 账本规则段记录本次定向（该更新是本 change 的一个 task，planning 阶段不动 `tests/`）。既有 `native_agent_turn_via_router_journey` 使用的 fake 上游在其透传职责内保留，其工作台行为断言部分由 test 模型 journey 接管后，证据指针相应拆分。

### D9 命名：场景叫 `test/*` 而不是新 provider 名

用户提议「可以叫 test」——正好就是既有 provider 名。不新造 `fake` / `mock` provider 名：`test` 已被 AGENTS.md、e2e、`/admin/stats` 的路由计数引用，改名是无收益的破坏。

### D10 旅程证明标准：绿即「工作台正常、稳定、可用」

journey 的设计标尺不是「跑通」，是**证明力**——全绿必须等价于工作台可用的证据。四个可断言的维度，全部写进 spec（`testsuite-process-e2e` 的「旅程证明标准」要求）：

1. **用户面保真**：journey 一律经工作台用户面驱动（webui HTTP API + WS/SSE 事件），绕过用户面直调内部接口的绿不算数——那证明的是 dispatch 而不是工作台。断言对象是操作员可见行为。
2. **路径完整**：journey 集合计覆盖操作员全路径（项目 → 建会话 → 提交 → 流式 → 权限 → 结果 → 关闭/归档）；任一环节无 journey 命中即缺口。既有 `workbench_aggregate_journey` 等已覆盖大半，新 journey 补 test 模型驱动的段落。
3. **可重复即稳定**：确定性场景的价值要被 journey 消费——代表性 journey（工具环）重复执行断言转录形状与终态一致（id/时间戳类字段除外）。单次绿不构成稳定性证明。
4. **失败也诚实**：error journey 双断言——失败如实呈现 + 失败后工作台仍可操作（新会话可建可用）。单点失败不得拖垮工作台，这本身就是可用性的一部分。

**echo 即连续性证明**：`test/text` 的回显句式（「我收到了你的消息 X」）使「完整对话历史送达 LLM」从隐含假设变成被断言的事实——journey 显式断言回显=最后一条用户消息。这是白拿的证明力，不写进断言就浪费了。

## Risks / Trade-offs

- **[块类型扩展破坏 fake-provider 的既有行为]** → `anthropic_wire` 的新入口不改旧签名；fake-provider 的既有单测与 e2e（agent-loop journey）不改而通过作为闸门。
- **[thinking 块的线协议细节与真实客户端不兼容]**（`signature_delta`、`content_block_start` 形状）→ 以真实 claude-code 与 native kernel 实测为准（D3）；实测结论记入本文件；native kernel 的解析在仓库内，可直接对齐。
- **[场景名与未来模型名冲突]**（`test/text` 作为模型被下游记录/展示）→ 模型名只影响路由与响应内容，debug 拓扑无计费与配额语义；AGENTS.md 食谱写明场景表即可。
- **[规则两处实现漂移]**（test provider vs fake-provider 的 agent-loop 判断）→ D2 的一致性测试；更进一步可把规则判断提为共用函数——实现时若顺路则做，不顺路靠测试钉住。
- **[journey 对权限流的依赖使其变慢/变脆]** → 复用既有 native 后端 helper 与权限批准 helper（`tests/support/`）；超时沿用套件既有有界时间策略。

## Migration Plan

1. `anthropic_wire.rs` 扩展块构造与 SSE 分块类型（thinking / tool_use），既有 text 路径回归。
2. `test_provider.rs`：场景解析（model 名）+ 各场景块序列 + agent-loop 规则 + 确定性 usage；bare `test` 路径零改动。
3. 一致性测试：与 fake-provider 规则同形状（D2）。
4. e2e journeys 三个 + 可选真实 claude-code journey；`AGENTS.md` 食谱补场景表。
5. 全量回归：`invoke testsuite-e2e` + `invoke testsuite-acceptance`。

**回滚**：分步提交。场景解析是纯新增（bare `test` 路径不动），回滚 = revert 对应提交；`anthropic_wire` 的扩展若需回滚，fake-provider 的既有测试先行的做法保证它独立可退。

## Open Questions

（2026-09-24 实测结论回填；每条都有对应测试钉住，见 `tests/testsuite_e2e_test.rs` 的
3.x journey 与 `sebas-router` 单测。）

- **thinking 块的 `signature_delta`**：按需发送（块带 `signature` 时才发
  `signature_delta`）。实测 native 内核对未知 delta 直接忽略，真实 claude-code
  （3.11 journey）也照常吃下该 thinking 块并跑完工具环——**结论：保留「有则发」，
  不强制、不阻塞任何客户端。**
- **`input_json_delta` 分片粒度**：固定 8 字符窗口（`SCENARIO_CHUNK`）。非流式/流式
  同构性由 1.3 的同构性单测钉住；native 侧按累加解析，拼接结果与非流式逐字一致
  （3.5 journey 的 `answer_text == 非流式 text` 断言）。**结论：固定窗口，不随场景变。**
- **未知 `test/<x>` 的处理**：回落到 bare `test` 的 echo 语义（`scenario_from_name`
  未知 → Echo），单测钉住。**结论：echo 回落**（新增场景只加名字，老客户端不炸）。
- **`test/full` 第二轮是否带 thinking**：终文本轮**不带** thinking（与真实模型典型
  行为一致）；3.3 journey 断言混排回合块序 = thinking → 工具 → 终文本。**结论：不带。**
- **`test/error` 的重试行为**：native 后端对 5xx **不重试**——3.8 journey 断言一次
  失败回合只留一条用量失败记录（`failed_rows.len() == 1`）、转录只有一条 `⚠ HTTP 500`，
  且会话非终局（terminal=false，其后换场景仍能完成正常回合）。真实 claude-code 的
  5xx 重试不在本 change 覆盖范围（3.11 只走 200 路径）。**结论：native 单次尝试。**

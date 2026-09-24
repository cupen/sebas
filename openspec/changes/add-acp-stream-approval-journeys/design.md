# add-acp-stream-approval-journeys — Design

## Context

见 proposal.md。当前事实基线（已核实）：

- fake-claude 桩（`tests/bin/fake-claude.rs`）已有两层剧本机制：触发词（perm / stream / drip，按用户消息文本选择，零装配改动）与顶层 `--scenario` 枚举（装配时固定）。hook_callback 审批环由 `wait_hook_decision(stdin_rx, req_id, io)` 实现：发 `control_request{subtype:hook_callback}` 后阻塞读 stdin 等应答。
- ACP 驱动层 `pending_perms` 是按 request_id 的 HashMap（`sebas-acp/src/claude/driver.rs`），结构上支持多个权限请求并发泊车；webui 读模型 `GET /api/sessions/{key}/approvals` 按 request_id 幂等合并枚举，前端 review-card 按请求渲染卡片。
- 浏览器沙箱装配（tasks.py）按 agent 惯例扩展：claude（默认档 `--slow-ms 800`）、claude-empty、claude-stream（`--delta-gap-ms 500`）。
- 关键既有覆盖：conversation-streaming.spec.ts 已钉「running 时 DOM 增量正文」（中途断言纪律，retries:0）；stop-settle.spec.ts 已钉「流式中 UI 取消」；permission-flow spec 已有「并行工具调用各自独立 request id」契约场景。

## Goals / Non-Goals

- Goals：给三条缺口各补一个确定性驱动 + 至少一条旅程（进程级 / 浏览器级），COVERAGE.md 入账。
- Non-Goals：见 proposal Non-goals（不碰 router/native 通路，不重复 stop-settle，不改生产行为）。

## Decisions

### D1 范围：互补小 change（用户拍板）

与 extend-test-model-scenarios 各管一条驱动通路：本 change 管 ACP/桩侧，router test provider 九场景归它。桩保留为「驱动器专属契约」权威（其 delta spec 的原话），故桩侧缺口必须在本 change 补。

### D2 剧本形态：触发词 `parallel`（默认推荐，用户未答，实施前可改）

与 perm / stream / drip 同机制：用户消息含触发词即选用剧本。理由：进程级与浏览器级都零装配即可达；不新增顶层场景枚举、不动 `SCENARIOS` 校验。备选「顶层 --scenario」被否：浏览器沙箱要为此新加装配段或改参数，收益只是「会话固定回放」，而触发词已足够；testsuite 惯例（drip/perm）都是触发词驱动浏览器用例。

### D3 并行泊车形状：重叠式（默认推荐，用户未答）

两个 tool_use 帧连发、两个 hook_callback 连发——第二个发出时第一个仍在等决定。理由：契约要钉的是「两张卡同时在场、各自独立」，顺序式（等第一个决定再发第二个）退化成既有 tool-loop 的两次循环，钉不住「同时待批」。重叠式对驱动层的要求已核实满足（pending_perms 是 HashMap，不串行化）。收尾语义：两个决定都收到后才发环后正文 + result，与 tool-loop 的「环后正文」形态一致。

### D4 thinking 浏览器旅程纳入本 change（默认推荐，用户未答）

桩 thinking scenario 已存在（两段 thinking/正文交替），只缺装配段与用例——补齐成本极低且是账本空白。装配段按惯例命名 `claude-thinking`（`args = ["--scenario", "thinking", "--slow-ms", "800"]`）。备选「留给场景扩展 change」被否：那是 router test provider 的 thinking 场景，通不同路。

### D5 流式「显示正确」的验收口径：结算切换断言，不重建中途断言

「实时性」已有 conversation-streaming.spec.ts 的中途断言（running 时 DOM 已见增量），本 change 不重复；补的是该用例没钉的**结算时刻**：turn 结束后 live-tail 纯文本切回全量 markdown 渲染。断言口径：同一回合的正文在结算后以 markdown 容器呈现（不再是 live-tail 形态）、内容与流式期间拼接一致、无重复条目。挂在既有 drip 装配（claude-stream）上，无需新装配段。

### D6 journey 驱动纪律：沿用项目既有标准

进程级经 webui HTTP API + WS 驱动、不绕用户面（extend-test-model-scenarios 3.9 的「证明标准」同款）；浏览器级零固定 sleep（expect.poll / 自动等待）、对「实现缺陷不得被 retry 掩盖」的用例设 retries:0（对齐 conversation-streaming）。

## Risks / Trade-offs

- [重叠泊车卡死：桩在等两个决定，测试侧若只应答一个，回合挂起] → 桩的 wait 循环在任一决定到达后继续等剩余请求；journey 的轮询超时与停滞看门狗（turn_stall_timeout）兜底，用例内先断言两请求都在读模型里再逐一决策。
- [两卡片竞速：前端合并窗把两次 hook_callback 帧抖成一张卡] → 账本行锚定 permission-flow「并行独立 request id」；断言以 request_id 为粒度（两个不同 id 各自可决策），不依赖两卡出现的时间间隔。
- [thinking 折叠的渲染细节与 markdown 渲染器耦合] → 浏览器断言只钉契约层（折叠在场、正文独立、顺序保持），像素级/滚动行为不进断言；细节已由前端单测 transcript-view.test.ts 覆盖。
- [剧本形态默认决策未经用户确认] → 已在 proposal/design/tasks 记录为可改点；若实施时用户改选顶层 --scenario，只动装配段与用例入口参数，specs 不变。

## Migration Plan

纯测试与桩扩展，无部署/回滚问题。落地顺序：桩剧本 → 进程级 journey → 浏览器装配与用例 → 账本加行，每步独立可验证（cargo test / invoke testsuite-e2e / invoke testsuite-webui）。

## Open Questions

无（剧本形态与泊车形状以默认推荐执行，已在 D2/D3 记录为可改点，不阻塞 spec 与任务分解）。

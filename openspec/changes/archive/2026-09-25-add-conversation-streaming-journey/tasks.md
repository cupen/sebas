## 1. 桩能力：按时间间隔发出 delta

- [x] 1.1 在 `tests/bin/fake-claude.rs` 增加 `--delta-gap-ms N` 解析（缺省 `0` = 完全现行行为），并在默认场景相邻文本 delta 之间 `sleep N` 毫秒
- [x] 1.2 补桩单测：`N=0` 时帧序与现行一致；`N>0` 时相邻文本帧的时间差 `>= N`
- [x] 1.3 在桩的模块文档注明时限约束：driver 看门狗的**控制探针应答超时 1.5s**（`sebas-acp/src/claude/driver.rs:477`）约束「先 sleep 再 pump」的 `drip` 路径；默认场景 gap 内每 50ms pump，不受该约束；**挂起探测**默认 5 分钟（`driver.rs:498-502`）。并把约束写进 argparse 说明

## 2. 专用对话流 e2e 用例

- [x] 2.1 新建 `tests/testsuite-webui/tests/conversation-streaming.spec.ts`：建 placeholder 会话 → 打开深链 → composer 输入并提交（design D4，确保观测起点早于回合开始）
- [x] 2.2 以 50ms 粒度轮询 focused conversation 的 DOM 与 `status_slug`，断言「增量正文已上屏」发生在会话进入终态之前；随后断言收敛 `done`；零固定 sleep
- [x] 2.3 该 spec 局部配置 `retries: 0`（design D5），确保实现缺陷不被 retry 掩盖
- [x] 2.4 断言只使用既有选择器（`sebas-dashboard sebas-transcript-view .turn-block.is-assistant` 与 composer/status 既有句柄），不新增测试专用 DOM 钩子
- [x] 2.5 （3c review 补强）中途采到的文本必须是最终回复的**更短真前缀**，排除「结束时整块渲染 + 服务端状态滞后」的 TOCTOU 假绿

## 3. 红→绿：实时渲染（条件执行）

- [x] 3.1 首次运行该 spec：若绿，记录「当前实现已具备实时上屏」作为回归护栏（实测 +22ms 上屏 / done +1310ms，上屏为部分正文），并跳至第 4 组
- [x] 3.2 若红：定位 `sebas-webui/frontend/src/views/transcript-view.ts` 的 `turn.append` 应用路径（订阅 / position 去重 / `willUpdate` 裁剪）并修到绿（条件未触发）
- [x] 3.3 若红且根因在推送时机：修 `sebas-webui` 侧 `turn.append` 的合并与推送，使增量在回合进行中到达而非收尾一次性到达（条件未触发）

## 4. 账本与口径同步

- [x] 4.1 更新 `tests/testsuite-webui/README.md` 旅程账本：新增「对话流实时上屏」用例行与锚点
- [x] 4.2 更新 `tests/acceptance/COVERAGE.md`：agent 对话簇补该用例的命中证据行
- [x] 4.3 在旧 `tests/testsuite-webui/tests/streaming.spec.ts` 的注释中说明其保留口径（多 chunk 合并为一气泡），并指向新 spec 承担实时性断言，避免两处口径重复或误导

## 5. 验证与收口

- [x] 5.1 `cargo build -p sebas -p sebas-acp --bin sebas --bin fake-claude --bin fake-acp-agent`
- [x] 5.2 `pnpm --dir tests/testsuite-webui exec playwright test conversation-streaming`：**首次尝试即通过**（retries 0）
- [x] 5.3 回归：`streaming` / `dialog` / `conversation` / `session-roundtrip` 四个 spec 全绿不回归
- [x] 5.4 `openspec validate add-conversation-streaming-journey` 通过
- [x] 5.5 （3c review 补强）进程级 e2e：`claude_delta_gap_spreads_ws_frame_arrivals_over_time` 与 `claude_delta_gap_spaces_the_default_scenario_deltas` 全绿

## 1. 诊断与基建

- [ ] 1.1 e2e 沙箱复现队列饿死：`tests/testsuite_e2e_test.rs` 新增用例——fake-claude 回合中途杀死子进程/掐断事件流，断言「提交入队后永不 drain」复现（修复前红）；同时验证泊车审批场景不计入停滞。命令：`cargo test --test testsuite_e2e_test -- --ignored`
- [ ] 1.2 `[dispatch] turn_stall_timeout` 配置键落地（默认 600，0 = 关闭），config 解析与校验单测通过

## 2. 引擎看门狗（core-session-channel delta）

- [ ] 2.1 引擎为 WORKING 会话记录 `last_event_unix`（事件热路径单点写入），泊车审批期间暂停计时；单测覆盖「泊车不计时、解除重计」
- [ ] 2.2 周期扫描停滞会话：超阈值 → 复用非终端 Error 收尾通道（SEED/WORKING → DONE + flush + drain）+ 发 warn 分级通知（点名会话与释放条目数）；单测断言收尾、drain、通知三件事，及 `timeout=0` 时扫描短路
- [ ] 2.3 快照加 `turn_engaged` 字段（WORKING ∨ 泊车 ∨ spawn 窗口，纯加法缺省兼容）；`session_payloads` 端点测试补字段断言

## 3. 前端判定与反馈（agent-workbench delta）

- [ ] 3.1 `client.ts` SessionDetail/SessionSummary 加 `turn_engaged?`；dashboard 优先消费它，缺省回退 `status_slug === 'working'`；`workbench-composer.test.ts` 补 waiting/starting 态的 queued/stop 形态断言
- [ ] 3.2 提交控件：泊车态下排队形态附「等待你的审批」指示、空输入呈停止方块；`pending-stack.test.ts` 补「原因 + 起等时刻」标注断言（栈条目按 `turn_engaged` 与泊车事实渲染等待原因）
- [ ] 3.3 pending 操作两级反馈：`pending-stack.ts` 按操作后服务端全量判「竞态竞输（静默）vs 确定性拒绝（notice-layer 低档点名条目与原因）」，网络失败归入后者；单测覆盖两条路径与 4xx 明细透出

## 4. 收尾验证

- [ ] 4.1 e2e 1.1 用例转绿（杀子进程 → 看门狗收尾 → 队列前进 + 通知可达），`invoke testsuite-e2e` 全绿
- [ ] 4.2 `invoke testsuite-acceptance` 全绿；`tests/acceptance/COVERAGE.md` 补 Pending liveness 行；浏览器套件 `pending-stack.spec.ts` 补拒绝反馈旅程
- [ ] 4.3 沙箱联调：`invoke testsuite-webui-sandbox` 起真实 UI，泊车/停滞两场景人工核验提交形态、停止可达、通知呈现

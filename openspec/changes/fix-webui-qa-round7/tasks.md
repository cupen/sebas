## 1. 转录长文本布局（P1，agent-workbench）

- [x] 1.1 浏览器旅程先红：`tests/testsuite-webui` 新用例——发送 2100+ 字符无空格消息，断言转录容器 scrollWidth 不超视口、用户气泡可见；主题切换后复断言（对应账本 DEF-01 证据 24-hscroll-left-edge.png）
  - 留待 3c/主 agent GUI：浏览器旅程用例未写未跑（2100+ 字符 scrollWidth ≤ 视口、主题切换重渲染复断言）；实现层修复与样式合同单测已完成（1.2）
- [x] 1.2 内容层约束补全：`.body` 升级 `overflow-wrap:anywhere`；markdown `table` 补块级横向滚动；`.meta .author` 补收缩守卫（`transcript-view.ts` + `components/markdown.ts` 配套样式）
- [x] 1.3 回归：既有转录/markdown 渲染测试全绿；`pre` 代码块滚动语义不回退；GFM 表格旅程断言横向滚动
  - 留待 3c/主 agent GUI：仅剩「GFM 表格旅程断言横向滚动」的浏览器旅程断言；单元半边已完成——vitest 33 文件 817 用例全绿，pre 滚动 / table 块级滚动 / author 守卫 / 时间戳内联样式合同已钉进 `transcript-view.test.ts`（layout contract describe）

## 2. ACP 模型拒绝终态化（P1，acp-model-selection）

- [x] 2.1 进程级复现 journey 先红：fakeacp 会话 ok-model 成功 → bad-model 拒绝，断言拒绝后回合在远短于 600s 内终态、无滞留 WORKING、后续回合无「操作者中断」条目（`tests/testsuite_e2e_test.rs`，对应账本 DEF-02 证据 21-stuck-running-check.png）
  - 留待 3c/主 agent GUI：进程级 journey 用例未写未跑（需沙箱起 fakeacp 会话）；实现与单元测试已完成（2.2）——pump 即时归类 + 引擎终态边 + 回合身份取消锚（`tests/pump_unit_test.rs`、`sebas-dispatch/tests/model_rejection_and_cancel_anchor_test.rs`）
- [x] 2.2 按 design D2 定位并修复：拒绝回执的终态语义（driver 补 terminal 回执或引擎 Error 终态边，二选一闭环）；`cancelled_turns` 改按回合身份关联消费
- [x] 2.3 回归：真实中断路径（审批等待中点停止）仍如实标注；claude 驱动 `set_model` 正常切换路径不回归；sebas-node `SetModel` ack 路径行为一致

## 3. 未认证 WS 重连闸（P3，webui-ws-rpc）

- [x] 3.1 `ws.ts` 升级失败（`everOpened=false` 的 close）进静默等待态，解除复用 `setAuthGated(false)` 路径；组件单测断言登录页不发周期性升级尝试
- [x] 3.2 浏览器旅程：auth 沙箱登录页 console 无重复 `/ws` 失败；登录后 WS 建立且实时事件可用
  - 留待 3c/主 agent GUI：auth 沙箱浏览器旅程未跑（登录页 console 静默断言、登录后实时事件链路断言）；实现与 ws/app-shell 单测已完成（3.1）

## 4. P3 打磨包

- [x] 4.1 `MODE_OPTIONS` 标签双语化（Ask·逐次询问 等），发送值与门控不变；快照/单测随文案更新
- [x] 4.2 浅色主题 composer 常态边框去错误语义观感（先核实实际生效变量再改）；明暗两态截图对照入账本
- [x] 4.3 转录时间戳位置两侧统一（实现期按视觉惯性二选一）；转录渲染测试随动

## 5. 收口

- [x] 5.1 `tests/acceptance/COVERAGE.md` 回填本轮缺陷→修复证据；按设计不改项（/goal 英文 tooltip=agent 内容、watchdog 时长）逐条记 cause
  - 留待 3c/主 agent GUI：账本回填需旅程/截图证据先行（DEF-01/DEF-02 修复证据、4.2 明暗两态截图对照）
- [ ] 5.2 全量：`invoke testsuite-e2e` + 相关浏览器旅程绿；`openspec validate fix-webui-qa-round7` 通过
  - 留待 3c/主 agent GUI：全量套件与 validate 由主 agent 收口（本轮门禁：cargo test 754 绿 / cargo build 成功 / vitest 817 绿）

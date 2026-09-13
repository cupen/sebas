# Tasks: workbench-agent-identity-and-process-folds

## 1. 后端：TurnEntry 结构化标题

- [x] 1.1 `sebas-dispatch` `TurnEntry` 增 `title: Option<String>`
  （serde 缺省 + skip_serializing_if），`ConversationEntryView`
  （sebas-webui `models.rs`/`api.rs`）同步透传；`cargo test -p
  sebas-dispatch -p sebas-webui` 既有用例全绿（验证旧 JSON 无 title
  反序列化为 None）
- [x] 1.2 ToolStart/ToolEnd 落盘处构造 title（`{tool} · {key_arg}` /
  `✓ {tool} · {key_arg}`，偏好键序提取 + 200 字符上限），单测覆盖：
  path 类命中、command/pattern 回退、无字符串参数只有工具名、旧格式
  兼容

## 2. 前端：过程大折叠与二级折叠

- [x] 2.1 `transcript-view.ts` 分块器改「单过程块」：回合内 thinking +
  tool 汇入一个块（首过程条目位置），文本段流序在外；`transcript-view
  .test.ts` 覆盖：混合回合一个 process 折叠、纯文本回合无折叠、
  多 run 过程并入一处
- [x] 2.2 过程块渲染：外层 `<details>` 默认收起（summary 概要
  `process · N`），内部逐条二级 `<details>` 默认收起、summary 显示
  title（回退通用标签），条目以 `position` 为键稳定身份；测试断言
  默认全收起与展开层级
- [x] 2.3 中间截断 helper（>64 字符 → 首 28 + … + 尾 28，`title`
  属性保全量），单测覆盖短串原样、长串截断、多字节字符不切半个

## 3. 前端：agent 身份与已收到角标

- [x] 3.1 `<sebas-transcript-view>` 增 `agentDisplay` 属性，assistant
  作者标签 display → slug → assistant 回退，头像维持文本形态（展示名
  首字母）；dashboard 按 `agent_kind` 匹配 `/api/agents` 目录传入；
  测试覆盖回退链
- [x] 3.2 已收到角标：最后一条 entry 为操作者 prompt 且会话 Working
  时该气泡显示「已收到」角标，agent entry 到达即消失（派生态）；
  `transcript-view.test.ts` 补两态断言
- [x] 3.3 `pnpm --dir sebas-webui/frontend test` 全绿、`cargo build`
  通过；`invoke testsuite-webui-sandbox` 手工冒烟：发消息见角标 →
  流式开始角标消失、过程折叠默认收起、二级折叠标题带工具名与路径
  - 收尾验证（2026-09-13）：单测 285/285 全绿（进程退出码噪声为 HEAD
    既有的 happy-dom 未处理错误，与本变更无关）；沙箱冒烟折叠链、二级
    标题、wire title、角标随 agent entry 消失全过。⚠ 角标的 live 出现
    窗口在现 wire 语义下不可达：`emit_turn_card` 在 DONE→WORKING 翻转
    后 drop+重播种回 SEED，首个流式事件才回 WORKING 且同时落第一条
    agent entry——「working + 末条为 prompt」不会出现；badge 渲染经
    detail 抓包注入该时刻、走真实 dashboard→组件渲染路径验证（上游
    需裁决：放宽前端 working 门，或调整 phase 语义）。

## 4. 收尾

- [x] 4.1 e2e/验收套件中涉对话渲染断言的用例适配新折叠结构；
  `invoke testsuite-e2e` 通过；`openspec validate
  workbench-agent-identity-and-process-folds --strict` 通过
  - 收尾验证（2026-09-13）：e2e/acceptance 为 HTTP 层、无 DOM 折叠断言，
    无需改动；浏览器套件适配两处旧断言（workbench.ts `toolGroup()` →
    `processFold()`/`processItems()`，conversation.spec「工具组展开」→
    「过程折叠」两级展开）。`invoke testsuite-webui` 58+3+4 全绿、
    `invoke testsuite-e2e` 19/19 通过、acceptance 涉对话 journey 抽样
    3/3（session_lifecycle / native_agent_turn_via_router /
    projects_session）、`--strict` 校验通过。

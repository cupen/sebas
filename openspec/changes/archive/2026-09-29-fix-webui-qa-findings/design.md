## Context

三轮 GUI 验收（主功能 42 点 / 认证 13 点 / 增量 9 点）的证据与复现步骤见
`C:/Users/cupen/AppData/Local/Temp/sebas-qa-0927/qa-report.md`、
`qa-auth-report.md`、`qa-delta-report.md`（截图同目录 screenshots/）。
被测形态：bare core `--webui`（auth=false 与 auth=true 各一）+ 独立
`--debug` router + fake-claude stub。

关键事实：

- 未读徽标的渲染代码在 `sebas-webui/frontend/src/views/project-rail.ts`
  完整存在（`rowUnread` / `unread-badge` / `session-unread` testid），
  但非聚焦到达场景运行时不出徽标（QA 两轮证据一致）——回归，非设计移除。
- `permission_mode_result` 条目类型前端可渲染（transcript-view），但后端
  从不写入（两轮 API entries 均无该类型）——需求缺口在后端落盘点。
- 崩溃僵尸（D5）：fake-claude `crash` 后回合快速失败正常，但**下一条**
  消息把会话置 `Queued`，唯一出口是 ~600s 的 stall watchdog 强制收尾。
- D1（排队回复重复）仅在旧前端构建上复现且有 API 层 delta 翻倍证据；
  新构建未复现——间歇性竞态，先调查后修。
- M7 是 DD2 的显形原因链：config.toml 的 `[provider.*]` 不进 provider
  store → `default_selection` 为空 → 自动标题静默回退「预览」，而预览
  实现跟随**最新**消息，违反命名链的「首条」口径。
- 依赖注意：`add-agent-settings-and-session-titles`（complete，未归档）
  持有 `project-session-actions` 的命名链需求原文；本 change 的
  project-session-actions delta 以其全文为基底。

## Goals / Non-Goals

**Goals:**

- 每个 QA 缺陷/缺失都有行为级修复 + 至少一条自动化回归（单测或 e2e）。
- 回归项（DD3）补浏览器级 e2e，把「徽标在非聚焦到达」钉进套件。
- 不扩大既有能力边界：全部是对现行 spec 意图的兑现或细则补强。

**Non-Goals:**

- 不重做通知层（分级通知层 spec 不变，只接成功率回执）。
- 不改 driver 的探针超时语义（D3 只加即时确认呈现与终态路径）。
- 不动 stall watchdog 的存在性（D5 后它降级为兜底，不删除）。
- MA2 / M4 / M5 / OB1（见 proposal Non-goals）。

## Decisions

1. **DD3 修复路径 = 先 e2e 锁行为再修实现**。用
   `tests/testsuite_webui_browser`（native Playwright 装配）新增
   「非聚焦会话到达 → 徽标出现」用例复现；随后按证据修（首选嫌疑：
   `session.updated` 投影未携带推进 `msg_count` 的字段或 rail 行渲染
   依赖缺失，次选 `rowUnread` 的 gate 条件）。备选（直接改代码凭自觉）
   否决——回归项必须先有红灯测试。
2. **D2 契约条目在后端应用点落盘**：mode switch 被 core/dispatch 应用
   成功时写入持久化 `permission_mode_result` 条目（复用既有条目存储与
   事件广播；前端零改动或仅微调）。否决「前端本地合成条目」——跨客户端
   与刷新不一致，且 reload 后消失。
3. **D5 提前终态**：driver 已能观测子进程死亡（fast-fail 路径存在），
   把「死亡已确认」的会话在映射层直接终态化；后续消息走正常 lazy-spawn
   或类型化拒绝。否决「缩短 watchdog 超时」——误伤真慢后端（slow 场景
   合法长静默）。
4. **D3 取消即时确认**：composer 侧在 cancel 请求被接受即进入
   pending-cancel 呈现；后端在子进程变得可中断/退出时落「已停止」终态。
   不承诺静默期内强杀——驱动探针协议决定强杀不可靠。
5. **D4 焦点调和在前端路由层**：displayed project 变化/项目移除事件到
   达时，若 focused session 不属于新 displayed project 的可见集合，
   清焦点到空态。否决「后端在移除时清焦点指针」——焦点是每浏览器
   本地状态。
6. **M7 config-seeded provider 只读可见**：Models 页列出 store 行 +
   config 种子行并标注来源；config 行本页只读（编辑引导走既有路径）。
   否决「把 config 行导入 store」——破坏「一个文件一个写入者」边界
   （AGENTS.md），且静默导入会造成双源漂移。
7. **M6 成功回执走既有 notify 层**：归档/恢复/注册/移除/重命名/建会话
   成功补 info toast；失败仍走各自既有内联/类型化呈现，不双弹。
8. **OB2 双闸**：服务端对 root 自指（self username）的降权/禁用/删除
   返回 40x；前端对自己行禁用对应控件。保留自助改密。
9. **M9 toolchain 探测修复**：`/api/about` 的探测实现修为可区分
   「未安装 / 探测失败 / 版本 X」三态；不扩 Build 段（归
   `add-about-build-info`）。
10. **D1 调查优先**：先以「perm 挂起 + 排队消息 + 放行」路径写进程级
    e2e 压测循环；命中则修 dispatch 的 queued-submission → turn 输出
    接线（嫌疑：放行唤醒时 delta 被双路 append），未命中则做竞态审查
    + 保留压力用例，并在任务里如实记录结论。
11. **归档顺序**：`add-agent-settings-and-session-titles` 先归档（specs
    落主 spec），本 change 的 project-session-actions delta 随后合并；
    若顺序反转，以本 delta 全文为准人工合并。

## Risks / Trade-offs

- [D1 间歇性可能无法在 CI 复现] → 保留压力用例 + 竞态审查结论落 tasks；
  不假装修复。
- [DD3 根因跨 backend/frontend 边界] → e2e 红灯先行，修复点由证据定。
- [D5 提前终态可能误判「慢启动」为死亡] → 仅在驱动确认进程退出
  （exit 事件/管道关闭）时终态化，不用超时推断。
- [M7 只读 config 行让操作员困惑「为什么不能编辑」] → 行内明示来源与
  编辑引导。
- [project-session-actions delta 与未归档 change 的合并冲突] → design
  决策 11 的顺序约束 + 全文基底降低冲突面。
- [通知回执（M6）过量弹 toast 打扰] → 仅限清单动作；既有去重/栈上限
  兜底。

## Migration Plan

纯行为修复，无数据迁移。回归套件（进程级 e2e + webui browser e2e）随
binary 发布；发布顺序不敏感，可整 change 一次落地。

## Open Questions

- D1 的确切根因（待复现）；若证实为旧构建独有且新代码已消除，任务收口
  为「回归用例 + 结论记录」而非代码改动。

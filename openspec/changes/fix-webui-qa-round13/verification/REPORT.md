# fix-webui-qa-round13 验证报告

- 日期：2026-10-04
- 验证人：主 agent（编排）+ QA-B / QA-A 两个 subagent（Playwright 真实浏览器黑盒，截图证据在 `shots/`）
- 被测：`http://127.0.0.1:9877/`（沙箱 `target/qa-r13-sandbox/`，auth 开，admin/member/viewer 三角色，
  fake-claude 桩 + debug router `test/*` 场景模型）
- 被测构建：`feat/webui-qa-round11` 工作树（含未提交的 fix-webui-qa-round12 tasks 1–3 实现：
  回合通知接线核验、usage 幸存者表、技能 sync 门禁）
- findings：`findings-b.md`（QA-B 会话核心链路簇）、`findings-a.md`（QA-A 设置簇 + 补测）

## ⚠️ 环境事故与操作员 home 影响清单（操作员需读）

验证期间发生两起环境事故，第二起波及了操作员真实 home。**按红线本流程未对 `~/.sebas`
做任何清理写操作**，以下为精确清单，请操作员自行决定是否清理：

1. **事故二（本流程责任）**：22:34（本地 06:34）的沙箱恢复重启因主 agent 的 shell 引号错误
   （bash 双引号展开吞掉 PowerShell `$env:` 赋值），core 在 `SEBAS_HOME` 未设下启动，落到
   `C:\Users\cupen\.sebas`（证据：`target/qa-r13-sandbox/core2.log` 首屏 path 行）。窗口约
   15 分钟（06:34–06:49 本地）。期间 QA-B 经 GUI 写入真实 home 的内容：
   - `settings.db` agents 表：新增 agent 行 `empty` / `error` / `slash` / `slow` / `thinking`
     （fake-claude 场景桩，path 指 `target/debug/fake-claude*.exe`，args 各场景）
   - `settings.db` providers：新增 provider `fake`（base_url 指向 127.0.0.1:8791，哑 key），
     并可能改动「新建会话默认模型」设置
   - `projects.db`：`work` 项目下的数条会话行 + session_map 行（GUI 直建，本不持久化，
     但该实例重启前写入）；对应会话转录落在真实 home 的 acp sessions 目录
   - `auth.db`：仅 admin 登录的审计痕迹，无新建用户
   - config 钉死路径（channel socket、skills 仓、media、workspace）未受影响；config 种子
     agent 因「store 同 id 行优先」被忽略，未写入
   - 清理建议：删除上述 agent 行与 provider 行、还原默认模型设置、删除窗口期会话与转录；
     或整体接受（均为哑数据，不影响真实凭据）
2. **事故一（外部因素）**：06:30:57 与 06:49（本地）两次出现非本流程启动的
   `invoke testsuite-webui-sandbox` 装配（`Temp/sbtestsuite.*`），其清场按 `sebas.exe` 映像名
   杀掉了本沙箱 core/router（QA-B 有 Bash 台账自证未运行 invoke）。**本项目存在三个每日
   QA 自动化（00:10 / 03:04 / 06:00，`CronList` 可见，runCount 9–12），彼此窗口重叠，
   是这类冲突的结构性来源**——建议操作员合并为每日一次。本流程已将沙箱二进制改名
   `sebas-qa13.exe`/`fake-claude-qa13.exe` 规避按映像名清场。

## QA-B 结果（会话核心链路簇）

- **PASS 20 项**（登录、会话创建/自动命名、项目注册、正文回合、thinking 呈现、并行审批卡
  出卡、多行输入、ask 批准/拒绝、edit 档、切换回执、流式停止、空回合、错误回合后可用、
  历史页、token 单调累计、console 横切等，逐项证据见 findings-b）
- **缺陷 2**：B-1（P3）Allow 档 `perm` 无审批卡直接执行（待复核定性）；B-2（P3）thinking
  展开面板留白偏大
- **console**：pageerror=0、console.error=0、HTTP≥400=0
- **在册项**：N-1 形态实录相符（GUI 会话重启蒸发 vs API 会话存活）；R12-B-1/B-2、O-2/O-5
  因服务中断未验证（转 QA-A 补测）
- 详情：`findings-b.md`

## QA-A 结果（设置簇 + 补测）

- **PASS 34 项、任务 20/20 覆盖、无阻断**（三角色 RBAC 横切、用户管理闭环、provider/别名、
  Esc 关弹窗、agents 目录增删改、技能删除门禁、usage 双粒度、主题持久化、SPA 导航、
  项目深入、auto 档、crash 恢复、F5 保持、R12-B-2 通知、深链、native 三连等，逐项证据见 findings-a）
- **无新增 A-## 缺陷**；QA-B 移交复核均维持原判：B-1（2/2 采样 100% 复现）、B-2（第 2 例确认）
- **口径观察 2 条**：观察-1 usage 页只见 router 流量（ACP 回合 usage 不上页，架构口径待确认）；
  观察-2 usage 折线图 y 轴刻度重复（1/1/1/0/0）
- **round12 在册项验证全绿**：
  - R12-A-1 技能 sync 门禁：member/viewer 无「同步」有「刷新」✓ 不回归
  - R12-B-1 crash 后 token：1500/150 → 1600/160 单调续增 ✓ 已修
  - R12-B-2 回合终点通知：历史页驻留 + API 会话完成 → info 瞬时条（a31）✓ 已修
  - O-2 crash 恢复假回执：本轮未复现（round12 task 4 未实现却未复现——形态待 round12 实现时复核）
  - O-5 死链深链 404：首载 1 次、静置+交互零增量，有界 ✓（round12 task 5.3 未实现却已达界，与
    round11 观察差异或源于环境，round12 实现时按原样收口即可）
- **console**：pageerror=0；console.error 14 条均为测试故意触发的预期 4xx（401/403/404 资源镜像）
- 详情：`findings-a.md`

## round13 收口执行（spec-go）

- **B-1 改判**：非缺陷。证据 `mode-vocabulary.ts`（round2 M-C6：allow 与 auto 同映射
  bypass tier；round7 描述互相点名等价）。收口 = AGENTS.md 过时句修正（task 5.1）+
  QA-BRIEF/REPORT 标注更正。
- **3a 执行**（subagent 包干 13 task）：全部完成。关键发现——B-2 根因不是
  min-height/gap 而是展开面板继承了 `.body` 的 `white-space: pre-wrap`（round11 4.1
  为多行提交所设），修法收敛在展开态 `.fold-body` 恢复 `normal`，pre 代码块/折叠态/
  横滚不回退；门禁 pnpm 943 全绿（基线 936 + 新增 7）。
- **3b 验收**（主 agent）：tasks 全勾核对、门禁重跑、spec 三条抽查对照代码全过。
- **6.1 GUI 抽查**（主 agent 真浏览器）：`r13_thinking_expanded.png`（空带绝迹）、
  `r13_usage_source_note.png`（说明行 + 刻度 0/62/123/185/246 互异）、
  `r13_native_hint.png`（无 env 一次性实例：禁用提示点名 env、无设置页误导）。
- **3c review**（subagent）：spec 逐条覆盖核对通过；补 1 个集成单测
  （`agent_kinds_test.rs` provider 配置不翻转 native 可用性）；修 2 个过时测试断言
  （first-paint 旧 native 文案、usage US2 locator）；**发现并收口主 spec 矛盾**——
  change 内补 `specs/agent-workbench` delta（native cause 口径反转），避免归档后
  语料冲突；6.2 定向套件 13/13 + 16/16 绿。
- **过渡提交**：94cfe93（round11 文档）→ 67ce88d（round12 实现）→ 186fe47（round13）。

## 整体验收

- `invoke testsuite-e2e`：87 例中 86 绿 + 1 红（`session_lifecycle.session_round_trip_via_webui_http`，
  症状「spawned core reachability 超时」，core.log 显示启动正常、webui.log 空——负载下
  的环境性抖动；**定向复跑 1.09s 通过**，且 round13 零 Rust 改动、其余 86 例全绿，
  不归属任何本轮 change）
- `invoke testsuite-acceptance`：全绿（含 projects_session / session_and_turn /
  remote_node 各旅程）
- 前端单测 943 全绿、`agent_kinds_test` 7/7、`openspec validate --strict` valid

## 缺陷总账（round13 新增，全部来自本轮 GUI 验收）

| ID | 级别 | 一句话 | 处置建议 |
|---|---|---|---|
| ~~B-1~~ | 改判非缺陷 | Allow·放行 档 `perm` 无审批卡直接执行且无环后正文，形态与 Auto 档完全一致（QA-B 提出，QA-A 2/2 复现） | **改判**：allow 与 auto 同为放行档系 round2 M-C6 既有裁决（`mode-vocabulary.ts` 明文「同映射 bypass tier」），行为符合设计；收口 = AGENTS.md 过时句修正（round13 task 5.1）+ 本标注，无代码变更 |
| B-2 | P3 | thinking 过程展开面板「💭 thinking」标签与内容间留白偏大（2 例确认） | 前端样式打磨 |
| 观察-2 | P3 | usage 折线图 y 轴刻度重复（1/1/1/0/0） | 刻度去重 |
| 环境-1 遗留 | P3 | native disabled 提示「到「设置 → 模型」配置」与真实启用条件（core env：SEBAS_AGENT_PROVIDER_API_KEY / SEBAS_AGENT_ROUTER_URL）不符，引导落空 | 文案与启用条件收敛（文案改准或设置页可达地启用） |
| 观察-1 | 口径 | usage 页只呈现 router 流量，ACP 回合 usage 不上页（会话头部计数不受影响） | 架构口径确认：文档化「usage 页=router 用量」或立项纳入 ACP 用量 |

- **在册滚动**：N-1（workbench 直建会话不进 session_map、core 重启蒸发）三轮未修，属功能级
  工程（native 后端接入映射持久化 + dormant 恢复 + 检查点转录），建议独立立项，不再随 QA 轮滚动。
- **round12 尾巴**：tasks 4（模型切换回执收口）/5（e2e 7 红回归 + 深链去重）/6（收口验证）未做，
  随 /spec-go 与本轮一并编排。

## 验证统计

- 覆盖：QA-B 20 PASS + QA-A 34 PASS = **54 项 PASS**；新缺陷 **2+2 观察**；无 P1/P2 新缺陷
- console：两簇合计 pageerror=0；4xx/5xx 仅测试故意触发与 2 次外部清场断连
- round12 已修 5 项（1.x/2.x/3.x）全部 GUI 复核不回归

# Design: polish-workbench-walkthrough-ux

## Context

走查证据（沙箱 `/tmp/sebas-guitest`，core 9875 + router --debug + fake-claude，全程截图与 ARIA 快照留档）：

- 点击 History 条目触发 `POST /api/sessions/{key}/restore`，条目即刻消失，无确认、无 toast、无导航；被恢复的无项目会话在 rail 不可见（`project-session-actions` 现行 spec 明文规定该行为，故为需求修订而非缺陷修复）。
- `sebas-webui/src/archive.rs` 的 `default_path()` = `$SEBAS_HOME` 或 `$HOME/.sebas/archive.json`，不在沙箱必钉 env 清单内；走查中沙箱 webui 读到真实归档并因上述点击语义把真实条目 `web\0web-1789633159295216842-0` 从 `~/.sebas/archive.json` 移除（mtime 实锤；目录中尚存 `archive.json.polluted-backup-20260912`，属二次事故）。
- 聚焦会话内自发自收的回合仍累计「~2 new since you last viewed」，需手动 mark all seen。
- 创建会话弹窗报「尚未配置任何模型」（来源 providers overlay），而会话面板模型下拉同时可用 agent 内置目录（default/opus/sonnet/haiku）并切换生效（`current_model=sonnet` 落库）——两处口径矛盾。
- composer 权限模式下拉在默认会话上显示空白、无 label；选项为裸词，创建弹窗中同词汇却带中文解释；`UNGATED` 红章与 `mode auto` 章并存，模式切换过渡态曾出现红色 `UNKNOWN`。
- Agent 下拉把 `Native Kernel (unavailable: native backend needs SEBAS_AGENT_PROVIDER_API_KEY (or SEBAS_AGENT_ROUTER_URL))` 整句外露。
- 关闭的弹窗残留在 ARIA 树（归档确认弹窗视觉关闭后仍被快照捕获）；Settings 弹窗在较长文案分区出现横向滚动条，关闭按钮被裁切；Playwright 语义点击全量超时（疑与残留弹窗或持续重渲染有关，根因未定论）。
- rail 占位会话行截图/快照均未见 spec 要求的 short-id 回退文案（`project-session-actions`「Session rows are named by the first prompt」），列复测项而非定论。

## Goals / Non-Goals

**Goals**

- 归档「查看」与「恢复」两个意图分离，恢复可发现、可确认、有反馈。
- 归档文件默认路径收敛进沙箱可钉定的数据目录族，迁移零手工。
- 未读边界与「正在看」的直觉一致。
- 同一词汇在创建弹窗与会话面板口径唯一；内部 env 名不出现在用户可见文案。
- 关闭的弹窗离开可访问性树；Settings 无横向溢出。

**Non-Goals**

- 不引入 i18n 框架（文案统一在现有 zh-CN 基调内逐条改写）。
- 不新增命令面板/全局快捷键（走查中 Ctrl+K 无响应与代码一致：前端无该 handler，非回归）。
- 不动 wire 协议、HTTP 路由面、restore/archive API 的请求响应形状。
- 不处理 process run 折叠、通知四级、WS 断线重连可视化（沙箱不可验证项，另行立项）。

## Decisions

1. **History 点击 = 只读查看；恢复 = 显式按钮 + 确认弹窗 + toast。**
   备选「保留点击即恢复、仅加确认弹窗」被否：点击的首要意图是回看内容，恢复只是次要动作，任何把恢复挂在点击热区上的方案都保留误触面；「hover 显式按钮」被否：触屏不可用且热区过小。归档视图复用会话视图只读态（消息门已拒绝归档会话的发送，后端无需改动），顶部操作区放单个「恢复到原项目」wa-button。
2. **archive.json 默认路径跟随 state DB 目录；`SEBAS_ARCHIVE_PATH` > `<state-db 目录>/archive.json` > 旧 `$HOME/.sebas/archive.json`（只读迁移源）。**
   备选「仅改文档提醒钉 env」被否：默认值落真实 `~/.sebas` 与沙箱零触碰原则结构性冲突，二次事故证明文档防线不够；「跟随 config 目录」被否：config 是 `-c` 显式传参，数据同目录语义弱于 state DB。迁移在 webui 启动时执行：新路径不存在且旧路径存在 → rename 到新路径，失败则 warn 并继续用旧路径（只读降级，不写旧路径之外的语义不变）。
3. **聚焦 + `document.visibilityState === 'visible'` 时到达的回合直接推进 per-browser seen boundary。**
   复用 `session-unread-badge` 既有 cursor 机制，不加第二套状态；「仅不计自己发的消息」被否：agent 回复同样会在眼前挂未读，半吊子；IntersectionObserver 视口级判定记为后续（tab 可见性粒度已消除主要误报）。
4. **模式词汇单一出处**：把「ask（逐次询问）/edit（自动接受编辑）/allow（放行并留审计）/auto（不门控，留审计）」的带解释形式提为共享常量，创建弹窗与 composer 下拉同源渲染；composer 下拉补 aria-label「权限模式」，默认会话显示「默认（ask）」而非空白。
5. **状态徽章合一**：`mode auto` 与 `UNGATED` 合并为一枚章（auto → 「自动执行」琥珀章，allow → 「放行」章…），desired/effective 过渡态显示中性灰章「模式切换中…」，删除红色 `UNKNOWN` 文案；权限语义色只用于真正需要警惕的 ungated 态。
6. **执行体不可用措辞**：下拉项显示「原生内核（未配置模型凭据）」，env 名与补救入口（Settings → Models）移入次行说明与 tooltip；`SEBAS_AGENT_PROVIDER_API_KEY` 不再出现在默认可见文案。
7. **模型警告对齐真实可用性**：创建弹窗仅在「目标 agent 的 resolved models 为空」时警告；provider 目录与 agent 内置目录并在两处各自注明来源（「可选项来自 agent 内置目录」/「provider 在 Settings → Models 维护」）。
8. **弹窗卫生**：关闭即从模板条件中移除节点（Lit `html` 分支已按条件渲染，排查残留分支补 `inert`/`aria-hidden` 兜底）；History 组头 `div[role=button]` 换原生 `<button>`；Settings 弹窗内容区 `min-width: 0` + 长词断行，消除横向溢出。
9. **e2e 可测性顺手项**：给恢复按钮、模式/模型下拉、未读边界条补 `data-testid`；tasks 内含「Playwright 语义点击超时根因排查」一条（残留弹窗假设优先验证），不阻塞其余任务。

## Risks / Trade-offs

- [老实例升级后归档文件搬家，用户找不到旧归档] → 启动时自动迁移 + 迁移 toast（webui 首屏 notify 既有通道）+ 迁移失败保留旧路径只读可用。
- [seen boundary 自动推进在「聚焦但切去别的窗口」场景可能过早标记已读] → 以 `visibilityState` 门控，后台 tab 不推进；残余误差可接受，记为已知限制。
- [归档视图复用会话视图可能误开发送门] → 归档态的 400 消息门既有测试保留，前端只读态由 archive 标志驱动而非路由猜测。
- [Playwright 超时根因未定论] → tasks 单列排查项，修复不依赖其结论；test-id 路径先行落地。

## Migration Plan

1. 后端先落 archive 路径迁移（含测试），前端再落 History 语义与 UI 一致性（无协议依赖，顺序可并行，合并顺序后端先行）。
2. AGENTS.md env 清单同 PR 更新，沙箱配方与 e2e（testsuite-webui-browser）同步补 `SEBAS_ARCHIVE_PATH`。
3. 回滚：前端 revert 即回点击即恢复语义；后端 revert 前需确认无依赖新路径的新写入（迁移是 rename，回滚后旧路径缺文件 → 保留的迁移副本逻辑不回滚也可读，风险低）。

## Open Questions

无——留作实现期自由度的点（迁移 toast 的具体措辞、data-testid 命名细则）不改变规格与任务拆分。

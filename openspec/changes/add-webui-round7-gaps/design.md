## Context

第七轮 GUI 验收的三项缺失（GAP-01/02/03），代码事实均已核实：

- **GAP-01 用量不可见是「半已采集」**：claude 驱动会话的回合 usage 引擎已累计（`sebas-acp/src/claude/driver.rs:1150-1163`、`:1250,:1268` → `sebas-dispatch/src/engine/mod.rs:1431-1441` CardUsage，随 SessionInfo 快照输出 :625 附近），但 webui 类型/视图从未消费（`api/client.ts:108-148` SessionInfo、`:150-` Summary 均无 usage 字段）；通用 ACP 驱动在 codec 层显式丢弃 ACP usage（`sebas-acp/src/acp_driver/codec.rs:62-65`，注释 design R1：ACP UsageUpdate 报 context window/cost，非 token 计数）。usage.db 唯一写入者是 router（`sebas-router/src/usage.rs`、`sebas-domain/src/state_paths.rs:476`）。
- **GAP-02 指示缺位但数据通道齐全**：dashboard 即主工作台且已挂载（`app-shell.ts:58-64`、`:826-856`）；`core.reachability` 有 WS 推送订阅（`app-shell.ts:605`）与主动查询（`:642`）；呈现面只有断线横幅（`notify.ts:143-147`）与 composer 门禁，无常驻徽标。
- **GAP-03 browse 链路纯只读**：`GET /api/fs/browse-dirs`（`api.rs:1736-1752`）与边界校验单点 `safe_path`（`fs.rs:114-165`，`within_workspace_root` fail-closed）齐备；前后端均无任何 mkdir 能力（grep 零命中）；folder-picker（`components/folder-picker.ts`）被 project-rail 与 new-session-dialog 复用。

## Goals / Non-Goals

Goals：三项缺失各成需求并带浏览器级断言；全部复用既有数据通道与校验单点，不新增存储、不动 wire。
Non-Goals：见 proposal（router 口径、新库表、cost→token 换算、dashboard 改版、通用 ACP 采集改造）。

## 关键决策

- **D1 用量展示走「已采集未消费」半边**：webui `client.ts` 的 SessionInfo/SessionSummary 补 usage 字段（对齐引擎快照），会话呈现面展示累计 input/output——呈现位置（转录头部/会话行芯片）实现期按视觉惯性定，两处同源。通用 ACP 无 token 上报 → 显式「未上报 token」文案，不冒充 0。备选「core 写 usage.db 汇入用量页」被否：usage.db 写入者是 router，第二写入者违反一个文件一个写入者红线（persist-router-usage 归属）；备选「codec 解析 ACP UsageUpdate」被否：其语义是 context window/cost 而非 token，换算即发明数据。`/usage` 页不动。
- **D2 徽标复用 reachability 通道**：app-shell header 常驻圆点/徽标，状态机三态（ok 低调 / down 醒目 / 恢复自动翻回），kind/cause 进悬停 tooltip；与既有 `setWsDown` 横幅联动（同源不同强度）。不新增 API、不新增轮询（订阅 + 既有主动查询足够）。
- **D3 mkdir 单层 + 单点校验**：`POST /api/fs/mkdir {path}` 与 browse-dirs 同层（webui-local，无 core 参与），复用 `safe_path` 边界校验不得旁路；语义为单层创建（父必须已存在，拒绝 mkdir -p 式批量创建，防误操作面扩大）；名称段校验拒绝空/`.`/`..`/路径分隔符。folder-picker 加「新建文件夹」按钮 + 内联命名输入，成功后局部刷新当前节点。webui 由此新增第一个写文件系统的 API 面，fs.rs 既有越界测试模式（`:381`/`:486` 风格）同步扩展为机械断言。

## 实施修订（review 轮，2026-10-01）

- **D1 修订（引擎侧两处小改）**：review 旅程实证引擎快照对任何有卡态的会话投影 `Some(st.usage)`——从未上报的会话（通用 ACP）被投成 `{0,0}`，spec「SHALL NOT 以 0 冒充」不可达，纯前端消费不够。修订：CardState 增 `usage_reported` 事实（真带 token 计数的 UsageUpdate 帧才置位）与 `usage_total` 会话累计量（不随 Finished 清零，`usage` 回合缓冲与 feishu footer 语义不动）；快照投影按 reported 门控输出累计量。这也顺带修正了「芯片实为本回合终值」的观察——真实 claude 连续回合的「增长」由累计量保证。proposal Impact 中「sebas-dispatch 不改代码」的假设就此修正（最小面：card_state.rs + engine/mod.rs 投影与落账）。
- **D2 修订（fresh load 竞态）**：review 实证 WS 打开几乎总先于 checkAuth 完成，onWsState 的 connected 分支在就绪前被吞、初始 get 永不发出（fresh load 徽标停 unknown）。修订：`markAuthReady()` 就绪即无条件补一次 `refreshCoreReachability()`（get 幂等、失败不推翻既有状态）。
- **徽标与横幅一致性补强**：WS 断连 = 承载推送的通道消失，此刻对核心状态的真实认知是「未知」——徽标回 unknown 中性态，不再沿述最后已知的「已连接」与断线横幅矛盾；重连后 get 自动收敛。
- **装配修正**：core-link-badge 旅程只在 detached 双进程拓扑成立（tasks 2.2 原文的 dead-core 装配是单进程形态，SIGKILL 的就是 webui 本身，翻转推送无从存在）；已把该 spec 归入 detached config（主 config testIgnore 排除 + tasks.py case 路由）。

## 风险

- 用量呈现位置若放在会话行会引入 rail 密度回归；实现期选点后须跑行布局快照类测试。
- mkdir 端点是新的写面：越界/非法名的负路径必须全部有测试（fail-closed 不能只靠 review）。
- 徽标与断线横幅的状态源若不一致会出现「点绿横幅红」的自相矛盾——两者必须订阅同一事件流。

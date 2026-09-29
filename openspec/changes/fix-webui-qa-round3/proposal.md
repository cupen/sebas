# Proposal — fix-webui-qa-round3

## Why

第三轮全功能 GUI QA（subagent 浏览器黑盒，证据 `C:/Users/cupen/AppData/Local/Temp/sebas-qa/qa-evidence/{w1,w3}/`）在核心链路全通的前提下发现一批 P2/P3 呈现与交互缺陷：thinking 过程折叠把正文误标成 thinking（违反 `agent-workbench` 既有 process-fold 语义）、权限模式下拉鼠标二次展开失效、`/sessions` 栅格与 usage 刷新按钮在 1280 宽视口溢出、provider 探测错误文本被裁切、重命名后详情头部不同步、History 显示归档前旧标题、usage 统计卡不随所选窗口过滤（违反 `usage-statistics` 既有「当前窗口汇总」）、登录前 `/ws` 401 重试噪音；另有两处功能补全：项目重排序有 API 无 UI、会话选择不写 URL（深链不可达自持）。

## What Changes

- thinking/工具过程折叠成员修正：正文段不挂过程芯片；thinking 段折叠缺省形态可与正文区分（P2）。
- 权限模式下拉指针交互修正：选择一次后可再次用鼠标展开（P2）。
- 1280 宽视口布局收纳：`/sessions` 卡片栅格不裁剪（可换行/滚动）、usage 刷新按钮不溢出、provider 探测错误文本完整可读（P2/P3）。
- 会话标题一致性：重命名后详情头部即时同步；归档 History 显示归档时刻的现用标签（P3）。
- usage 统计卡口径：汇总数字随所选时间窗/粒度联动（P3）。
- 功能补全：项目栏重排序入口（上移/下移，走既有 `/api/projects/reorder`）；会话选择写 URL（`/sessions/{key}` pushState，刷新自持焦点，rail 标记语义不变）（P3）。
- 登录前 `/ws` 重试静默/退避，消除每页 3–6 条 401 console 噪音（P3）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`：过程折叠成员与可区分性场景；History 条目标签取归档时刻现用标签；新增项目栏重排序入口要求。
- `webui`：mode 下拉可重复指针展开；会话栅格窄视口收纳与探测错误可读；会话选择写 URL 与刷新自持。
- `usage-statistics`：usage 视图汇总数字与所选窗口一致、控件不溢出。
- `webui-ws-rpc`：未认证客户端连接姿态（静默/退避，不再重试风暴）。

## Impact

- 前端 `sebas-webui/frontend/src/views/`（transcript-view / dashboard / project-rail / usage / settings-modal / mode 下拉组件 / router）为主；usage 统计卡为纯前端口径修正。
- 后端无改动（重排序走既有端点；URL 为前端 router 行为）；`/ws` 重试姿态为客户端行为。
- 无 wire 变更、无破坏性变更。验证：前端单测 + 既有 Playwright 套件 + GUI 手测逐项。

## Non-goals

- 不改 i18n 文案体系（英文混排清扫由 `webui-i18n-sweep` 单独立项）。
- 不改审批路由与 RBAC（各自由 `fix-parallel-approval-routing`、`gate-agent-directory-writes` 承接）。
- 不改后端聚合端点形状（usage 口径修正是前端取数联动）。
- 不新增工作台头部回合状态文字徽章（现行停止按钮 + rail 圆点语义足够，维持设计现状）。

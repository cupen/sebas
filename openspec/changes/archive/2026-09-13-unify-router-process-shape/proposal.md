## Why

router 目前有两种进程形态：裸 core 的内嵌形态（`core --router`，跑在 core
进程内）与 watchdog 受管子进程。内嵌形态无法被服务页动态启停；且
provider 数据已归 core（make-core-own-provider-data）后，router 进程的
唯一价值是给外部 CLI 当网关和承载 Router 档 agent 会话，内嵌形态失去
存在理由。统一为「router 永远是独立进程、默认关闭、服务页动态启停」。

## What Changes

- **删除内嵌 router（BREAKING）**：`run`/`core` 的 `--router` 与
  `--debug` 旗标移除（传入即 unknown-argument 报错），内嵌启动代码删除。
- **形态唯一化**：router 只以独立进程存在——watchdog 受管子进程与手工
  `sebas router [--debug]` 两种启动方式，进程本体同一入口。
- **停止保护**：`ServiceSet` 增 `force`（默认 false）。停止 router 时若
  存在活跃 routed 会话（provider mode = Router 且状态非终态），服务端
  拒绝并携带会话计数；`force: true` 放行；core 不可达时放行（无 core
  即无活跃流）。
- **服务页**：停 router 被拒时弹确认对话框显示活跃会话计数，提供
  「强制停止」出口（force 重发）。
- **默认关闭已是现状**（`[watchdog.router] enabled` 默认 false、Docker
  CMD 无 router），本轮不改，仅由形态统一固化；设置页 provider/models
  编辑走 core、不依赖 router 进程，无需降级工作。
- **测试设施**：浏览器套件与 e2e 的装配从 `core --router --debug` 单进程
  改为 core + `sebas router --debug` 两进程；AGENTS.md 菜谱同步。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities
- `cli-service`: 新增「router 仅独立进程运行」需求（删内嵌形态与
  `run`/`core` 的 `--router`/`--debug` 旗标）。
- `watchdog`: 「Service lifecycle」增 `ServiceSet.force` 与 router
  停止保护（活跃 routed 会话拒绝 + force 放行 + core 不可达放行）。
- `webui`: 「Services 分区与 router 状态归属」增 router 停止被拒的
  强制出口交互。
- `testsuite-webui-browser`: 「沙箱装配与清理边界」装配改两进程形态。

## Impact

- `src/run.rs`（删内嵌 router 启动与端口日志）、`src/cli.rs`（删旗标）。
- `src/watchdog/executor.rs` + `control_rpc.rs`（force 透传与停止保护）、
  core 侧活跃 routed 会话查询（经 session channel snapshot）。
- `sebas-webui` 前端 services 分区（settings-modal）确认弹窗。
- `tasks.py`（浏览器/e2e 沙箱装配）、`tests/testsuite_e2e_test.rs`、
  AGENTS.md 沙箱菜谱。

## Non-goals

- 不给裸 core/Docker 形态加动态服务控制面（要动态管理用 watchdog 模式；
  容器要 router 就 compose 加 `sebas router` 侧车）。
- 外部 CLI 直连 router 的消费者不计入停止保护（无从统计），文档如实说明。
- 不改 provider 三档（Off/Direct/Router）语义与设置页数据流（已走 core）。
- 不动 router 自身的下游鉴权（`[router] auth_token` 默认关闭，独立演进）。

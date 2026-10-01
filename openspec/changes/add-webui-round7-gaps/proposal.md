## Why

第七轮 GUI 全量验收（证据目录与 fix-webui-qa-round7 同源：`C:/Users/cupen/AppData/Local/Temp/sebas-qa-shots/`）发现三项能力缺失：ACP 回合的 token 用量对用户完全不可见——调度引擎早已累计 claude 会话 usage 并随 SessionInfo 快照输出，但 webui 类型与视图从未消费，通用 ACP 驱动的 codec 更是显式丢弃 usage（跑 10+ 回合后用量页仍只有 router 探活那 1 条）；主界面没有 core 连接状态常驻指示（只有断线后才出现的横幅与 composer 门禁，reachability 数据本有推送与查询通道）；添加项目的目录选择器无新建子目录入口（browse 链路纯只读，操作者只能去文件系统手工建目录再回来刷新）。

## What Changes

- 会话级 token 用量可见：webui 消费 SessionInfo 已有的 usage 快照，在会话呈现面展示累计 input/output token；未上报 token 的 agent（通用 ACP）如实标注，不冒充实数（usage-statistics）
- core 连接状态常驻指示：app-shell 常驻徽标，数据源用既有 `core.reachability` 订阅与查询，断线时与既有横幅联动、恢复自动翻转（agent-workbench）
- 目录选择器新建子目录：新增 `POST /api/fs/mkdir`（workspace root 边界内、父目录须存在、单层创建），folder-picker 加「新建文件夹」入口，成功后树内即时可见（webui/projects）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `usage-statistics`: 新增「会话级 token 用量可见」需求（router timeseries 口径不动）
- `agent-workbench`: 新增「core 连接状态常驻指示」需求
- `webui/projects`: 新增「目录选择器可新建子目录」需求

## Impact

- `sebas-webui`（client.ts SessionInfo/Summary 类型、会话用量呈现、app-shell 徽标、folder-picker、`fs.rs` mkdir 端点）
- `sebas-acp` / `sebas-dispatch` 不改代码（usage 采集引擎侧已存在；通用 ACP 无 token 上报是协议事实，如实呈现）
- 无新增库表；`usage.db` 归属与口径不动；无 wire 形状变更
- 验收账本 `tests/acceptance/COVERAGE.md` 回填 GAP-01/02/03 证据

## Non-goals

- router 用量口径与 `/usage` timeseries 保持现状（本 change 不把 ACP usage 混入该页）
- 不新增库表、不让 core 成为 `usage.db` 第二写入者（一个文件一个写入者红线）
- ACP context/cost → token 换算（语义不同，换算属发明数据）
- Dashboard 大改版 / 独立概览页（本轮只补连接状态指示）
- 通用 ACP 驱动的 usage 采集改造（等协议侧有 token 计数口径再立项）

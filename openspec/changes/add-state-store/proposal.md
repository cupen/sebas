# add-state-store

## Why

`~/.sebas/` 下的 JSON 文件群已经暴露出结构性问题:`state.json` 只在 core 优雅退出时写一次,WebUI 因此展示陈旧会话;`providers.json` 被 router 与 gateway admin API 双写,靠墓碑 + 未知段透传维持一致性(`state_store.rs` 文件头即一部失败史);`add-project-workbench` 又计划新增 `projects.json`(webui 独占),绕开而非解决多写者冲突。SQLite 单写者 + 已规划的 `core.sock` 通道,能一次收编这些问题。

## What Changes

- **core 内新增 SQLite 状态库**(单写者 task,actor 式 mailbox):`sebas.db`(WAL 模式)收编领域状态——sessions、projects、providers(含 model_aliases,墓碑改软删)、settings
- **`core.sock` 通道扩展状态方法**:快照读取、变更订阅、CRUD;webui/gateway 成为状态客户端,in-process 的 router/feishu 直接库调用
- **自动迁移框架**:`PRAGMA user_version` + 顺序迁移链,DDL 事务化执行,启动时自动前滚
- **放弃旧 JSON 数据**:开发阶段不做数据迁移,新库从零建表;旧 state.json / providers.json / settings.json 停用后留在磁盘、不再读取
- **替换文件的语义**:state.json 快照陈旧问题消失(库常新);providers.json 双写者问题消失(core 唯一写者,gateway admin API 代理到通道);`projects.json` 不再被创造(workbench 将改用状态方法)
- 依赖与时序:建立在 `add-core-session-channel` 的通道与鉴权之上;归档顺序 console → channel → **state-store** → workbench

## Capabilities

### New Capabilities

- `state-store`: core 的 SQLite 状态权威——表结构与 schema 版本契约、自动迁移语义(启动前滚、只加不改纪律、破坏性变更双阶段)、单写者所有权、通道状态方法集、损坏自愈、core 不可达时的诚实降级

### Modified Capabilities

- `session-persistence`: 持久化载体从 state.json 文件改为 SQLite;原子写/损坏容忍/版本迁移等文件级需求由迁移框架与 DB 事务接管
- `provider-management`: "Broken overlay self-heal" 从文件备份改为 DB 自愈;卡片 CRUD 的数据源改经状态库,UI 行为不变
- `gateway-model-aliases`: 别名存储从 providers.json 的 `model_aliases` 段迁至状态库,读写经通道
- `gateway-admin-api`: provider/别名的增删改查落库经 core(通道),admin API 语义保持、后端更换

## Non-goals

- 不提供 legacy JSON 数据迁移(开发阶段数据直接放弃,见 What Changes)
- 不动 `services.json` 与 watchdog 控制面(webui 靠它启用 core,归 core 会自举循环)
- 不收编追加流与夹具:usage jsonl、record/replay journals、内存 admin session 维持现状
- 不做多用户、非 loopback 访问、跨机同步
- 不改飞书侧路由与卡片交互行为
- 不实现 workbench 的项目 UI(归属 `add-project-workbench`,本 change 只提供存储与通道方法)

## Impact

- 新增 `sebas-state/` 模块(core 进程内):连接管理、迁移链、表定义
- `src/core_channel/`(channel change 引入):扩展状态方法与变更广播
- `sebas-gateway`: providers/别名热重载数据源改为通道推送,admin API 后端代理
- `sebas-webui`: settings/provider 页面改读状态方法
- `sebas-router`: state_store.rs 退役,读写改进程内状态库句柄
- 新增依赖:`rusqlite`(bundled SQLite)

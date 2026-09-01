# design — add-state-store

## Context

三个痛点汇聚成本 change(动机详见 proposal.md):state.json 优雅退出才写导致的 WebUI 陈旧快照;providers.json 双写者靠墓碑 + 未知段透传维稳(`sebas-router/src/state_store.rs` 文件头记录了整部演进史);workbench change 即将新增第三个独占 JSON 文件。`add-core-session-channel` 已规划 `core.sock`(UDS + 行式 JSON + 密钥/peer-uid 鉴权),本 change 的通道方法建立在其上,不重新发明传输。

关键结构事实:router、飞书 WS 循环与 core 同进程(`src/run.rs`/`ws_loop.rs`)。**in-process 消费者不需要走通道**,只有 webui/gateway 需要远程方法——"复杂消息路由"的实际面积比直觉小。

约束性决策(用户确认):**项目处于开发阶段,不做旧 JSON 数据迁移**——新库从零开始,旧文件停用即弃。

## Goals / Non-Goals

**Goals:**
- SQLite 单写者状态库,取代 state.json / providers.json / settings.json 三文件的领域状态职责
- 零操作自动迁移:启动即前滚,回滚路径明确
- 通道状态方法 + 变更订阅,消灭多写者协调机制(墓碑/透传/原子重写)

**Non-Goals:**
- 不做 legacy JSON 数据导入与迁移工具(开发阶段数据直接放弃)
- 不动 watchdog 控制面(services.json)与 usage jsonl、record/replay journals
- 不引入多进程直连数据库(见 D1)
- 不引入 ORM / 异步 SQLite 驱动栈(见 D4)

## Decisions

### D1 单写者归 core,而非多进程 WAL 共享

多进程直连同一 WAL 库技术上可行,但被否:schema 迁移权散到各进程,二进制升级顺序变成正确性问题;router 的 SessionMap 仍是内存态,会出现双真源;每个 crate 各带一份 rusqlite 查询层,crate 边界更糊。单写者让迁移、一致性、订阅广播各只有一处实现。替代方案记录在案:若未来拒绝通道依赖,多进程 WAL 是逃生舱,不是正解。

### D2 自动迁移:做,且用 user_version + 代码内顺序迁移链

**这是对"是否 auto migrate"的直接回答:做。** 理由与形态:

```
打开 DB ──► 读 PRAGMA user_version = N
              │
              ├─ N == 当前版  ──► 正常服务
              ├─ N <  当前版  ──► VACUUM INTO 备份 → 逐个跑迁移 vN→vN+1→…
              │                  每个迁移 = 一个事务(含版本号提升,见下)
              └─ N >  当前版  ──► 拒绝打开,报错指明版本(绝不静默读写)
```

- **版本戳用 `PRAGMA user_version`**,不建 migrations 记账表:单整数、引擎级原子、零样板。备选的 migrations 表(如 flyway 式)在单写者 + 单整数版本需求下是纯开销。
- **迁移链定义在代码里**(`const MIGRATIONS: &[fn(&Transaction) -> Result]>`),不用 sqlx 的迁移文件栈:单写者库不需要异步驱动和目录扫描,rusqlite 同步事务足够。
- **为什么敢自动跑:SQLite 的 DDL 是事务性的**(与 Postgres/MySQL 不同,`CREATE TABLE`/`ALTER TABLE` 可回滚)。"迁移跑一半库就废了"这个 PG/MySQL 时代的恐惧在这里不成立——失败即整体回滚,库留在原版本。这是 spec "Failed migration rolls back" 场景的技术底气。
- **不提供手动 `sebas migrate` 命令**:单用户本地应用,没有 DBA,独立迁移步骤只会被忘记。启动时自动前滚 + 失败即拒绝启动,是最诚实的失败模式。
- **迁移 1 = 纯建表**:没有历史数据要迁,首个迁移就是最终 schema 的第一版;后续版本只做增量。

### D3 回滚安全:三件套 + 一条纪律

1. **迁移前快照**:用 `VACUUM INTO` 生成单文件备份(`sebas.db.backup-<from>-<to>`),保留最近一份。不用裸拷贝——WAL 模式下直接 cp `db`+`wal` 有撕裂风险,VACUUM INTO 是引擎背书的干净快照。
2. **高版本拒绝打开**:rollback 到旧二进制撞上新库时,明确报错而非误读。用户路径:停服 → 恢复备份 → 起旧二进制;开发阶段亦可直接弃库重建。这个不对称性(升级自动、降级人工)写进用户文档,是换取"绝不静默丢数据"的代价。
3. **备份保留策略**:只留最近一份,避免 `~/.sebas` 膨胀。
4. **只加不改纪律**(工程约束,写入模块文档):常规迁移只允许加表、加可空/带默认值列、加索引;改名/删列/收紧约束等破坏性操作必须走双阶段(先加新列双写 → 下个兼容窗口再删)。SQL 里 INSERT/UPDATE 一律显式列名,禁止 `INSERT INTO t VALUES(...)` 位置插入——这是列序变化时旧迁移重放不炸的前提。

### D4 rusqlite(bundled)+ 专职写者 task,与项目 actor 风格同构

- `rusqlite` + `bundled` feature:自带 SQLite 源码编译,延续单二进制哲学,不赌系统 libsqlite3 版本。
- 连接是同步的:放**专职线程**,外面套一个 mailbox task——mpsc 收命令、oneshot 回结果。这正是项目里 supervisor/SessionManager 的既有模式(见前一轮 actor 讨论),**状态库成为又一个手写 actor**,不需要任何框架。
- in-process 消费者(router/feishu)拿到的是同一个 async 句柄;webui/gateway 经通道方法,最终也汇到同一个 mailbox。全系统一条写路径。

### D5 无数据迁移(开发阶段决策)

旧 JSON 文件(state.json / providers.json / settings.json)**不导入、不改名、不再读取**——新库从零建表,provider/settings 由用户经卡片或 admin API 重新配置一次。理由:数据尚无生产用户,导入器是一次性纯成本且自带正确性风险;旧 spec 的"损坏自愈/调和"语义随文件一起退役,库的损坏策略改为显式失败、绝不静默重置(见 spec)。升级前如需保留配置,手工抄录即可。

### D6 通道方法与变更事件

方法按域组织:`state.providers.*`、`state.aliases.*`、`state.settings.*`、`state.projects.*`、`state.sessions.snapshot`。变更通知是单一事件流,带 scope 标签(providers/aliases/settings/projects/sessions),提交后投递,允许合并(一串提交一个通知)。webui 订阅所有域;gateway 只关心 providers/aliases。

### D7 gateway:文件热重载 → 订阅投影

`hot_reload.rs` 的文件监听退役,gateway 内存投影改由订阅维护;通道断连时保持最后有效配置并在 admin surface 暴露"数据源不可用"(对应 spec "channel failure keeps serving" 场景)。admin API 语义不变,后端从"写 overlay 文件"改为"经通道写库"。

### D8 settings 一并迁入(记录的假设)

settings.json(CardConfig)进入 settings 表(旧文件不迁移,设置从默认值开始、用户重配一次),router 进程内读写。依据:它与 provider 数据同源同生命周期,留在文件里就得给 webui 留第二条文件写路径,违背本 change 初衷。

## Risks / Trade-offs

- [rusqlite bundled 增加编译时间] → 接受;一次性成本,换单二进制确定性
- [WAL 的 `db-wal`/`db-shm` 伴生文件] → 备份/恢复一律走 VACUUM INTO 快照,不裸拷贝
- [core 故障面扩大:库不可用 = 全部领域功能不可用] → 诚实降级契约(状态页明示成因)换掉静默陈旧;这是 channel change 已确立哲学的延伸
- [早期用户的 provider/settings 配置升级后丢失] → 开发阶段明确接受(用户确认);重配一次即可,成本低于维护导入器
- [跨版本回滚需人工介入] → 高版本拒绝 + 备份恢复路径;开发阶段亦可弃库重建。拒绝静默降级是有意为之
- [迁移链随版本膨胀] → 只加不改纪律 + 兼容窗口双阶段;迁移函数不可变,进了链就不许改

## Migration Plan

1. 依赖先行:`add-core-session-channel` 落地(core.sock、鉴权、SessionBackend)
2. 本 change 按任务序实施:骨架 → 迁移框架 → 表/仓储 → 通道方法 → 消费端切换 → 清理
3. 上线行为:启动自动前滚建表,无操作员步骤;旧 JSON 文件停用但不删除(留在磁盘不再读取)
4. 回滚:停服换旧二进制;旧二进制拒开新库 → 恢复 `sebas.db.backup-*`,或直接弃库重建(开发阶段数据可弃)
5. 归档时同步改写 `session-persistence` spec 的 Purpose(去 state.json 化)——delta 流程不携带 Purpose 变更,需归档时手工改主 spec

## Open Questions

- `add-project-workbench` proposal 里 `projects.json`(webui 独占)一段需在本 change 归档前改写为"项目注册落库"——改 proposal 归属 workbench 作者/下次评审,本 change 不代改
- usage jsonl 是否迁库(查询便利 vs 追加流语义)留待独立 change,本期不决

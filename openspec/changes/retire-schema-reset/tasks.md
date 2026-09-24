## 1. derive 层：rename_from 标注

- [ ] 1.1 `sebas-schema-derive` 解析 `#[column(rename_from = "...")]` 并编译期校验（≠自身列名、不与同 struct 其它字段重复），`sebas-db` 的 `SchemaColumn` 增加 `rename_from: Option<&'static str>` 字段（`schema.rs` 侧类型同步）；单测覆盖解析、重复拒绝、自身冲突拒绝，`cargo test -p sebas-schema-derive` 通过
- [ ] 1.2 补文档注释（derive crate 顶部类型映射表加 rename_from 行），`cargo doc` 无警告即可

## 2. 注册清单结构化（design D3 前置）

- [ ] 2.1 `sebas-db` 的 `TableSchema` 把 `create_ddl` 拆为 `create_table_ddl` + `index_ddls: &[&'static str]`，各注册清单（根 `repo.rs` 与 `sebas-models` 的 KV/provider 表）与首建/重建路径（`rebuild_schema`）改为两段组装，`schema.rs` 现有全部测试不改断言跑通
- [ ] 2.2 校验首建布局不变：新建库的 `sqlite_master` SQL 与拆分前逐字一致（临时测试或断言），防注册重构悄悄变形

## 3. 计划构建（design D1a）

- [ ] 3.1 `sebas-db` 的 schema 模块新增只读计划构建：逐表 diff 出动作清单（Add / Rename{old,new} / TypeChange / Drop / CreateTable），rename 对解析（`rename_from` 命中 live 旧列）与受限预判（`pragma index_list`/`index_info`/PK 成员）在此完成；单测：种子旧结构库断言产出的动作序列正确
- [ ] 3.2 rename 未命中（旧列不存在）退化为 Add + WARN；「一缺一多」无标注不判改名；单测各一条

## 4. 执行器与安全网（design D1b/c、D4、D5）

- [ ] 4.1 备份函数：计划含破坏性动作时 `VACUUM INTO '<db>.pre-sync'`（已存在先删后建），备份失败拒启动且不动库；单测覆盖成功、失败两路
- [ ] 4.2 事务执行器：Add / Rename / Drop 快路径照拼、TypeChange 与受限 Drop 走 12 步重建（FK OFF→事务→ON），全程单事务，失败回滚 + 拒启动诊断点名步骤；单测：类型变更保数据、受限删列走重建、注入失败步骤后库字节不变（对比 mtime+内容哈希）
- [ ] 4.3 删除重置路径：`SyncFail::Incompatible`、`reset_and_rebuild`（现居 `sebas-db/src/schema.rs`）、重置分支全部移除，未知/缺失 `version_format` 改 WARN + 照常 reconcile，`SyncOutcome` 换结构化动作计数（design D6/D7）；`rg -n "reset_and_rebuild\|Incompatible" src/ sebas-db/ sebas-models/` 无残留，旧重置类测试改写为迁移断言

## 5. 全量测试与验收

- [ ] 5.1 spec 场景逐条落测试：fresh/加列/声明改名保数据/未声明不猜/类型重建保行/删列弃数据/未知版本 reconcile/版本值不触发/备份先行/备份失败阻断/失败回滚拒启——对应 `specs/state-store/spec.md` 全部场景，`cargo test -p sebas -p sebas-db -p sebas-models` 全绿
- [ ] 5.2 旧迁移链库入库路径：user_version=2 无 schema_meta 的种子库首开被 reconcile 吸纳且数据保留（原 `legacy_user_version_db_without_meta_resets_on_first_open` 反转为保数据断言）
- [ ] 5.3 沙箱联调：按 AGENTS.md 食谱（`SEBAS_STATE_DIR` 一次性目录）起 bare core，手工构造改名+类型变更+删列三个场景各跑一轮启动，核对日志点名与 `.pre-sync` 备份存在；跑 `invoke testsuite-e2e` 确认无回归
- [ ] 5.4 `SCHEMA_VERSION` bump 至落地日期常量；`corrupt_db_refuses_to_open_and_file_is_untouched` 原样保留通过（损坏边界未被波及）；两个分层库（`settings.db` / `projects.db`）各跑一轮「改名 + 类型变更 + 删列」验证各自独立适用、互不触碰

## 1. 映射表与解析

- [ ] 1.1 落地「逻辑名 → 所属库 → 文件名 → 覆盖变量」映射表与统一解析函数（复用 `sebas-domain` 已收敛的 tilde 展开）；验证：单测覆盖全部逻辑名的默认值、所属库与覆盖变量名
- [ ] 1.2 优先级实现：逐文件覆盖 > 状态目录 > 默认；验证：三种组合各一条单测（仅目录、目录+单文件、都不设）
- [ ] 1.3 状态目录变量是纯环境变量，不依赖配置文件；验证：单测断言在配置文件缺失时仍能解析状态目录

## 2. 拆前核对：跨域原子性枚举（**必须先于 3.x**）

- [ ] 2.1 枚举全部**写事务**的域边界，逐个判定落在 `settings.db` 还是 `projects.db`；验证：产出一张表（事务 → 涉及表 → 所属库），且**没有任何一个写事务横跨两库**
- [ ] 2.2 核对 channel 的 state 快照读是否要求跨域自洽；验证：结论写进 `design.md`（若要求，说明它今天已是多次查询、拆库不使其更差；若发现写事务横跨 → 回到 2.1 调整边界，**不引入跨库两阶段提交**）
- [ ] 2.3 复核 `save_persisted_state`（providers + model_aliases + runtime_state）与 `import_defaults_once` 都在 `settings.db` 内完成；验证：单测断言该事务只触达 settings 库的连接

## 3. core 的两个库

- [ ] 3.1 用 `sebas-db` 各开一次 `settings.db`（`providers` / `model_aliases` / `settings`）与 `projects.db`（`projects` / `session_map`）；验证：`cargo test -p sebas` 全绿，且两库文件按映射表落在状态目录内
- [ ] 3.2 `providers` 表扁平化为类型化列（name / preset / 三个 base_url / api_key 等，列名与既有 JSON 键对齐），`sebas-models` 的 `ProviderRow` 同步重塑；验证：channel 上的 provider JSON 形状经 serde 断言与改造前一致，且 `Item = Map<String,Value>` 不再出现在存储路径
- [ ] 3.3 两库各自的 WAL / `busy_timeout` / schema 注册表与版本戳独立；验证：单测断言各自 open 后 pragma 生效，且重置一个库不影响另一个（改造 `projects.db` 的 schema 后 `settings.db` 的值仍在）
- [ ] 3.4 单写者不变：仅 core 打开这两个库；验证：`grep -rn "settings.db\|projects.db" --include=*.rs sebas-webui/src sebas-router/src sebas-im/src` 无打开点（读取一律经 state 方法）
- [ ] 3.5 一个库不可用时如实降级、不假装成功；验证：单测构造「`projects.db` 打不开」场景，断言项目面呈现 unavailable 且设置面仍可用
- [ ] 3.6 各表 CRUD 经 `#[derive(ActiveRecord)]` 生成，注册表只留 DDL 与约束；验证：`grep -rn "fn save_projects\|fn add_project" src/` 无自由函数形态，标准 CRUD 无手写 SQL

## 4. 各落点改走映射表

- [ ] 4.1 `archive.json` 与 `projects.json` 改走映射表（默认落点不变，仍在 `~/.sebas`）；验证：不设任何变量时两路径与改造前逐字相等（断言测试）
- [ ] 4.2 `nodes.json` 改为可派生 + 显式覆盖（配置键优先级不变），默认位置由配置目录移到状态目录；验证：单测断言「配置键 > 目录派生 > 默认」且默认落在状态目录
- [ ] 4.3 默认收敛断言：不设任何变量时，每个逻辑名的落点都在默认状态目录下；`nodes.json` 是唯一发生迁移的落点，其余与改造前逐路径相等；验证：新增测试通过（期望清单来自改造前实测 + `nodes.json` 新位置）

## 5. 修掉 services.json 越界点

- [ ] 5.1 `services.json` 改为从状态目录派生并支持显式覆盖（仍为文件）；验证：单测断言钉住目录后文件落在目录内，且读取/写入行为不变
- [ ] 5.2 **越界回归**：钉住状态目录并完整跑一次 watchdog 生命周期（启动 / 覆盖层写入 / 停止）；验证：断言操作员的真实配置目录**未被创建或修改**（mtime + 存在性比对）

## 6. 退休 SEBAS_STATE_DB 与机械断言

- [ ] 6.1 退休 `SEBAS_STATE_DB`，改为状态目录 + 逐库覆盖变量；验证：导出该变量后行为与不导出时一致（单测），且启动日志对退休变量给出明确提示
- [ ] 6.2 派生覆盖断言：枚举全部逻辑名，断言每个派生路径都在钉住的目录内；验证：新增测试通过，且临时把某个逻辑名改回硬编码 `~/.sebas` 时该测试失败（附一次失败演示）
- [ ] 6.3 复核没有新的硬编码状态路径；验证：`grep -rn "\.sebas/" src/ sebas-*/src/` 的命中都在映射表的默认值定义处

## 7. 菜谱与文档

- [ ] 7.1 `tasks.py:_sandbox_env` 改为钉 1 个状态目录变量（逐文件变量降级为可选覆盖），保留 `HOME`；验证：沙箱起得来，且既有 journey 全绿
- [ ] 7.2 `AGENTS.md` 的沙箱菜谱更新：必钉集合由「五个 env」改为「一个目录 + HOME」，说明分层库的位置与逐文件覆盖的用法；验证：文档与 `tasks.py` 实际行为一致
- [ ] 7.3 记录 `services.json` 越界点已修、以及「它为何仍是文件」的例外说明；验证：文档含该条目与验证方式

## 8. 全量回归与收口

- [ ] 8.1 跑 `invoke testsuite-e2e`（含 webui 沙箱任务）；验证：全绿
- [ ] 8.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
- [ ] 8.3 沙箱越界总复核：钉住目录跑一遍完整旅程后，比对真实 `~/.sebas`、`~/.config/sebas`、`~/.local/share/sebas` 的清单与 mtime；验证：三者逐项未变，结论附 PR 描述

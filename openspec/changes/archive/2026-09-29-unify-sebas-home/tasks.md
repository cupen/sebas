## 1. 映射表扩族与正名（sebas-domain）

- [x] 1.1 `StatePath` 枚举扩族收编新落点：`ConfigDefault`（config.toml 缺省）、`CoreSecretDefault`、`ChannelSocket`、`ControlSocket`、`MediaDownloads`（`cache/downloads/`）、`NodeStateDir`（`node/`）、`UpgradeDataDir`（`upgrade/`）；`SEBAS_CORE_SOCKET`/`SEBAS_CONTROL_SOCKET`/`SEBAS_NODE_DIR` 收进 override 表（消费点改读 `StatePath`，行为等价）；config/secret 的「显式 `-c` 锚定」保持为消费点特例。verify：`cargo test -p sebas-domain` 新增枚举项默认派生单测全绿
  - 备注：`file_name()` 更名 `rel_path()`（新族含子目录段，名字不再名实相符）；`state_dir()` 更名 `sebas_home()`。router 订阅侧（`sebas-router/src/core_channel.rs::socket_path`）改读 `StatePath::ChannelSocket`——env 缺席时从「返回 None 不订阅」改为「落 `<SEBAS_HOME>/run/core.sock` 派生」，watchdog 托管路径（env 注入）行为不变。
- [x] 1.2 正名反转：`SEBAS_HOME` 升首选读，`SEBAS_STATE_DIR` 降为仍生效的别名并进 warn 检出（同设冲突时 warn 说明 home 赢）；更新退休变量检出清单与 `src/run.rs` 启动 warn 接线。verify：state_paths 单测覆盖「仅别名生效且等价」「同设 home 赢」「warn 触发」三例
- [x] 1.3 三个既有机械断言按新族扩列（`defaults_converge…`、`derivation_covers_every_logical_name…`、`dir_variable_alone_relocates_every_logical_name`）。verify：`rtk cargo test -p sebas-domain` 全绿
  - 备注：第三个断言改名为 `home_variable_alone_relocates_every_logical_name`（钉正名）；别名单独生效另有等价断言。

## 2. config 缺省与 secret 锚定

- [x] 2.1 去掉 `src/cli.rs` 各子命令 `-c` 的静态 `default_value`，收敛到 `default_config_path()` 解析器（`-c` 缺席 → `<SEBAS_HOME>/config.toml`）；router reload 回落（`sebas-router/src/config.rs` 的 `~/.sebas/config.toml`）改走同一解析器。verify：单测「`-c` 缺席解析进 home」「显式 `-c` 赢」「`SEBAS_ROUTER_CONFIG` 仍最优先」
  - 备注（实现走形记录）：`default_value_t` 方案被否——clap_derive 4.5.55 把 `default_value_t` 表达式包进进程级 `OnceLock`，首次 parse 即冻结，env 动态缺省必然踩坑（已实测复现）。按 design D3 原文落地：字段改 `Option<String>`，`src/cli.rs::resolve_config` 在消费点解析（main.rs 各 dispatch 位与 From 实现）。
- [x] 2.2 核对 core.secret 路径：显式 `-c` 时与 config 同目录的规则不变，缺省时随 config 落 home 根。verify：`secret_file_path_defaults_to_config_dir` 单测扩展出 home 缺省形态

## 3. socket 收编

- [x] 3.1 channel socket 缺省改 `<SEBAS_HOME>/run/core.sock`（`src/core_channel/server.rs` 解析改读 `StatePath`，`run/` 随启动创建 0700；`SEBAS_CORE_SOCKET` 与 `[service.core] channel_path` 覆盖不动）；核对 socket 路径过长时 bind 报错可读。verify：单测覆盖 home 派生与两级覆盖；既有 stale socket 收复测试仍绿
  - 备注：0700 只对**本次新建**的父目录收紧（显式指进既有共享目录绝不改别人权限）；路径过长在 bind 前 pre-check（≥104 字节给点名路径的可读报错）。优先级：config 键 > env > 派生。
- [x] 3.2 control socket 同款改造（`src/watchdog/control_rpc.rs`，`SEBAS_CONTROL_SOCKET`/`--socket` 覆盖不动）。verify：watchdog 既有 pinned env 测试扩展断言新缺省
  - 备注：main.rs `run_control` 的显式 env or_else 分支删除（env 已在映射表内）；新增 `control_socket_default_follows_sebas_home_and_env_override` 单测。

## 4. media / node / upgrade 收编

- [x] 4.1 media 下载缺省改 `<SEBAS_HOME>/cache/downloads`（`src/config.rs` 默认值与可写性校验）。verify：`skills_dir_default_and_override` 同款单测
- [x] 4.2 sebas-node 状态缺省改 `<SEBAS_HOME>/node/`（`sebas-node/src/config.rs`，`--state-dir` > `SEBAS_NODE_DIR` > `[node] state_dir` 链序不变）。verify：sebas-node config 单测四层优先级
  - 备注：sebas-node 已有 sebas-domain 依赖（无需加边）；`dirs` 依赖随缺省锚移除而下线。
- [x] 4.3 watchdog 升级数据缺省改 `<SEBAS_HOME>/upgrade/`（`src/upgrade.rs` 的 `data_dir` 链，`[watchdog.storage] data_dir` 优先不变）；`src/watchdog/services.rs` 的 `pinned_lifecycle_never_touches_operator_home` 扩断言 `run/`/`cache/`/`node/`/`upgrade/` 亦不落 fake home。verify：该 pinned 测试与新单测全绿
  - 备注：installer 侧 `data_dir_for_user` 缺省锚同步改 `<user home>/.sebas/upgrade`；SEBAS_HOME/SEBAS_STATE_DIR 的 env 测试锁并入 crate 级 `home_env_test_lock`（并行用例互相踩 env 的竞态修复）。

## 5. 测试基建与进程级 e2e 扩面

- [x] 5.1 `tests/support/mod.rs` 的 `Sandbox::envs` 改钉 `SEBAS_HOME`（config 移入沙箱 home 根），`tasks.py` 两套装配同步。verify：`cargo test --test testsuite_e2e_test -- --ignored` 抽一例跑通
  - 备注：钉法改动完成（support/mod.rs、tasks.py `_sandbox_env` 与 smoke 装配、real_agent_e2e、testsuite-webui 的 detached.ts 均改钉正名/去别名钉）；新钉法下 e2e 首跑随 7.2 全量绿。
- [x] 5.2 `single_state_dir_journey_pins_every_state_location` 扩成 home journey：只钉 `SEBAS_HOME`，断言全族落点（四库、三 json、config 缺省解析、socket、`run/`、`cache/`、`node/`、`upgrade/`）都在 home 内、fake home 无痕、退休名无处出现。verify：`invoke testsuite-e2e --case <journey>`
  - 状态备注：已扩为 `sebas_home_journey_pins_every_file_location`（Sandbox 新增 `new_derived_home` 变体：config 不带 channel_path / `[media] download_dir` / `usage_db` 三键，派生形态即断言面）。四进程旅程：core 免 `-c`（auto-arm 写 `<home>/core.secret`）+ webui 免 `-c`（secret 文件发现）+ router 免密（`SEBAS_ROUTER_CONFIG` 锚定发现，watchdog RouterSpawner 同款注入）+ 真节点免 `--state-dir` 配对。断言：四库实物 + 域分离、archive/nodes json 实物、`run/core.sock` 在场且 SIGTERM 后移除、`cache/downloads` 由 config 可写性校验建出、`node/` 派生实物、core.secret 落 home 根、映射表全 15 行派生 containment（含无写入者的 services.json / upgrade/——实物分别由 watchdog 覆盖层旅程与真实升级负责）、fake home 无 `.sebas`、退休名（sebas.db / projects.json）无处出现、正名钉法启动日志无别名/退休告警、router 订阅成功日志。auth.db 经 `sebas auth add` 供应（`auth = false` 沙箱不开用户库）。verify 实测绿。
- [x] 5.3 新增别名兼容用例：仅钉 `SEBAS_STATE_DIR` 跑冒烟旅程并断言启动 warn。verify：e2e 用例绿
  - 状态备注：新增 `legacy_alias_state_dir_still_works_and_warns`（Sandbox 新增 `pin_home_via_legacy_alias` 钉法切换）。冒烟（注册 + fake-claude 回合 Done）+ 两库落点实物等价 + core 启动日志点名「旧名 / SEBAS_HOME」且无「同时设置」冲突告警。verify 实测绿。
- [x] 5.4 跨进程可达性在新 socket 缺省下复核（webui↔core reachability 翻转、watchdog control RPC、router 订阅）。verify：`invoke testsuite-acceptance` 相关旅程全绿
  - 状态备注：三子项均实证。①webui↔core 翻转：acceptance 全旅程 `wait_reachable` + e2e `watchdog_supervised_core_recovery`（SIGKILL core → unreachable cause → 监督重启 → reachable）全绿；②watchdog control RPC：e2e `watchdog_service_override_lives_in_state_dir_and_survives_restart`（webui admin adapter ↔ 派生 `run/control.sock`，adapter_ok + ServiceSet 落盘 + 重启读回）全绿；手工沙箱补证 CLI 客户端——watchdog 拓扑下 `sebas status` / `sebas services` 无 `--socket` 直达派生 `<home>/run/control.sock`（exit 0、四服务列表、`/api/admin/services` adapter_ok=true）；③router 订阅：5.2 home journey 内实证（config 无 channel_path 时双端同源派生 `run/core.sock`，「core channel subscribed」日志 + secret 文件发现）。

## 6. 文档与示例同步

- [x] 6.1 AGENTS.md 沙箱菜谱改写：`SEBAS_STATE_DIR` → `SEBAS_HOME`、config 进 home、`-c` 可省的说明、socket/`run/` 新位、smoke-real 拓扑段核对。verify：按新菜谱手工起沙箱完成一轮会话往返
  - 备注：文档改写完成；「手工起沙箱往返」由 3c 的 5.4 手工沙箱补证与 5.3 别名冒烟旅程（注册 + fake-claude 回合 Done）覆盖。
- [x] 6.2 `config/config.toml.example` 缺省注释更新（media、usage_db、channel_path 等新缺省）；`openspec/glossary.md` 收「sebas home」词条。verify：文档 diff 评审通过
  - 状态备注：config.toml.example 已更新；glossary「sebas home」词条已补（`openspec/glossary.md` 领域概念首条）。
- [x] 6.3 运维说明落地：BREAKING 两项（config/socket 缺省）、NFS 部署 socket 指引、手动迁移三步（停机 → `mv` → 起机，node 身份整目录搬）。verify：README/docs 评审通过
  - 备注：README 新增「sebas home（数据与落点）」一节；`docs/remote-execution-node.md`、`docs/architecture/process-ipc-subcommands.md`、`.ansible` 角色注释同步。
- [x] 6.4 `sebas service` unit 渲染与 watchdog spawn 参数核对：确认不依赖 cwd config 缺省，一律显式 `-c`。verify：代码审查 + service 安装冒烟（`systemd` 环境不可得时以渲染输出断言）
  - 备注：`validate_config` 本就拒绝相对 `-c`（exit 5）、ExecStart 烤显式绝对路径（既有渲染断言）；watchdog 子进程全部显式 `--config`。新增 `validate_rejects_relative_config_path` 单测。

## 7. 全量回归

- [x] 7.1 `rtk cargo test` 全绿 + `rtk cargo clippy` 无新告警。verify：两条命令通过
  - 备注：753 passed / 93 ignored；clippy 与 HEAD 逐条 diff 仅既有告警行号平移（0 新增）。
- [x] 7.2 `invoke testsuite-e2e` 与 `invoke testsuite-acceptance` 全量通过。verify：套件报告
  - 备注：e2e 80 passed / 0 failed；acceptance 10 passed / 0 failed（报告 `.artifacts/verify/report-e2e.html`、`report-acceptance.html`）；`rtk cargo build` 0 错误、`rtk cargo test` 754 passed / 94 ignored。

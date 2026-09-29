## Context

拷问说明：立项拷问的两轮提问（统一范围、变量正名）未获回答，本文按各项的推荐答案落档；每条决策都给了被否备选，评审时可翻案再改工件。

现状（见 proposal Why）：db 面已由 single-state-dir 统一，规则表唯一住在 `sebas-domain/src/state_paths.rs`（`StatePath` 枚举 + `SEBAS_STATE_DIR` > `SEBAS_HOME` legacy 别名 > `~/.sebas`）；config 缺省是 cwd 相对的 `./config.toml`（clap `default_value`，`src/cli.rs` 12 处）；core.secret 缺省与 config 文件同目录（`src/config.rs` `core_secret_file_path`）；channel/control socket 缺省走 `XDG_RUNTIME_DIR`（`src/core_channel/server.rs` / `src/watchdog/control_rpc.rs`）；media 缺省 `~/.cache/sebas/downloads`；sebas-node 状态缺省 `<dirs::data_dir()>/sebas-node`；watchdog 升级数据缺省 `<dirs::data_dir()>/sebas`。`expand_tilde` 刻意不吃 `HOME` env（Known Folder）；skills sync 侧另有 env-first 的 `resolve_home`——两套语义并存是有意的，本 change 不动它们。

## Goals / Non-Goals

**Goals:**

- 一个 `SEBAS_HOME` 钉住全部 sebas 自有落点；名册集中在 `state_paths` 映射表并由机械断言看守。
- `SEBAS_STATE_DIR` 平滑降级为别名：行为等价 + 启动 warn，现有脚本与部署不断。
- 沙箱菜谱简化为「钉一个变量 + config 放沙箱内」。

**Non-Goals:**

- 不收跨工具共享面（skills 仓、sync 落点、workspace、ACP work_dir/sessions_dir）——见 proposal Non-goals。
- 不做自动数据迁移、不引入新 env 变量、不做 XDG 重排。

## Decisions

**D1 正名 = `SEBAS_HOME`，别名反转优先级。** `SEBAS_HOME` 成为唯一正名；`SEBAS_STATE_DIR` 仍被完整采纳但启动 warn（沿用退休变量检出的先例，`retired_env_vars_present` 同款），二者同设时 `SEBAS_HOME` 赢——正名优先于别名，语义自洽。备选：双正名等价（否：「统一」语义打折，两个名字长期并存）；维持 `SEBAS_STATE_DIR` 正名（否：scope 已超出 state，名不副实，且提案主题就是 sebas home）。`state_paths` 内实现为一次互换：`SEBAS_HOME` 提为首选读、`SEBAS_STATE_DIR` 挪进 warn 检出名单的「仍生效」档。

**D2 布局 = 根平铺名册不动 + 四个新子目录。** 既有逻辑名（四库、archive/services/nodes json、legacy defaults.json）仍在 home 根，文件名不变——现有安装零迁移、零行为差。新收编落点进子目录：`run/`（两个 socket，易逝语义与持久名册分开）、`cache/downloads/`（media）、`node/`（sebas-node 状态，整目录语义）、`upgrade/`（watchdog 升级数据，内部布局 upgrade.lock/versions/current/rollback/bin/downloads 原样）。备选：全平铺进根（否：socket 与持久状态混放，`run/` 名字本身就承载「可随时清空」语义）；`config/`、`state/` 深分层（否：破坏现有名册位置，违背零迁移）。

**D3 config 缺省进 home，secret 继续锚定 config。** `-c` 缺省从 `./config.toml` 改为 `<SEBAS_HOME>/config.toml`；core.secret 的「与 config 文件同目录」规则原样保留——显式 `-c` 时 secret 跟着显式目录，缺省时一起落 home 根。机制上 clap 的 `default_value` 是静态的，解析不了 env：改为去掉 clap 缺省、收敛到一个 `state_paths` 侧的 `default_config_path()` 解析器（`-c` 缺席 → home 派生），12 个子命令共用。`SEBAS_ROUTER_CONFIG` 缺席时的 reload 回落从写死的 `~/.sebas/config.toml` 改走同一解析器。备选：新增 `SEBAS_CONFIG` env（否：变量增生，Non-goal）。沙箱菜谱随之简化：config 移进 `<SB>/config.toml`（本来就在），钉 `SEBAS_HOME=<SB>` 后连 `-c` 都可省——但菜谱仍写显式 `-c`，防 home 误指。

**D4 socket 离开 `XDG_RUNTIME_DIR`。** 两个 socket 缺省 `<SEBAS_HOME>/run/<name>.sock`，不再查 `XDG_RUNTIME_DIR`/`$TMPDIR`；显式覆盖照旧（`SEBAS_CORE_SOCKET`、`SEBAS_CONTROL_SOCKET`、`--socket`、`[service.core] channel_path`）。server 侧已有 stale socket 收复，`run/` 随 core/watchdog 启动创建（0700）。备选：保留 XDG 优先、home 仅作回落（否：两套隐式规则，「exactly one rule」的既有 spec 立意被打破）。已知代价见 Risks。

**D5 node 状态与升级数据收编，优先级链原序保留。** sebas-node：`--state-dir` > `SEBAS_NODE_DIR` > `[node] state_dir` > `<SEBAS_HOME>/node/`；升级数据：`[watchdog.storage] data_dir` > `<SEBAS_HOME>/upgrade/`。只换最底层的缺省锚，链上其余不动。node 身份存于状态目录内，整目录手动搬移即保身份（写进迁移指引）。

**D6 派生族总表与机械断言同源扩面。** 新逻辑名（config 缺省、secret 缺省、两 socket、media cache、node state、upgrade data）进 `StatePath` 枚举；既有三个机械断言（`defaults_converge_under_the_default_state_dir`、`derivation_covers_every_logical_name_inside_pinned_dir`、`dir_variable_alone_relocates_every_logical_name`）按新族扩列。config/secret 因「显式 `-c` 锚定」是映射表的特例：枚举项表示**缺省派生**，显式锚在消费点覆盖——断言只对缺省形态成立。逐文件 env 与 config 键的既有逐点优先级一概不动，本 change 只在「派生缺省」一层换锚。

**D7 迁移 = 无自动迁移 + 别名 warn 期不设截止。** 已有 `~/.sebas` 安装原位继续（缺省未变）；操作员想把数据搬去新 home：停机 → `mv` 目录 → 起机（socket 不搬，`run/` 重建；upgrade artifacts 可重新下载；node 身份整目录搬即保）。不做「目标目录为空但 `~/.sebas` 有数据」的提示性 warn——额外行为需 spec 背书，收益不成比例，记录为有意不做。`SEBAS_STATE_DIR` 别名无限期保留，移除留给未来 change（warn-first 先例：`SEBAS_STATE_DB` 从 warn 到退休走了独立 change）。

## Risks / Trade-offs

- [config 缺省变化咬到依赖 cwd 缺省的既有脚本/部署] → **BREAKING** 显式标注；tasks 里核对 `sebas service` unit 渲染与 watchdog spawn 是否都带显式 `-c`（若依赖 cwd 缺省则一并钉成绝对路径）；迁移说明写清。
- [NFS/网络主目录上 Unix socket 不可靠] → 运维指引：此类部署用既有 `SEBAS_CORE_SOCKET` / `[service.core] channel_path` 显式指走；spec 场景不承诺网络文件系统上的 socket 行为。
- [socket 路径总长超 Unix 104/108 字节上限] → 深层沙箱目录两方案同险；`run/` 短名已尽量压深；命中时报错应给出可读的路径过长信息（tasks 里核对现有 bind 错误路径）。
- [同设两变量的部署因优先级反转改道] → warn 明说「`SEBAS_HOME` 赢」；此类部署本就是配置事故。
- [socket 双端发现不同步（core 换位、客户端仍在旧位找）] → 双端共用同一解析函数、同仓同改；既有跨进程 e2e（webui↔core reachability、watchdog control）兜底。
- [`XDG_RUNTIME_DIR` 语义丧失（登录期 tmpfs 自清理）] → socket 文件本就有 stale 收复；`run/` 残留无害且随 home 删除。

## Migration Plan

无数据迁移。发布说明列 BREAKING 两项（config 缺省、socket 缺省）与别名 warn；操作员三条路径：(a) 什么都不做——缺省 `~/.sebas` 未变，唯一可感知是 config 必须放 home 或显式 `-c`；(b) 想用新变量——`SEBAS_HOME` 指向现有目录即刻生效；(c) 想搬目录——按 D7 手动三步。回滚 = 换回旧变量名/旧目录，无格式变化。

## Open Questions

（无——拷问未答的两问已按推荐答案落档为 D1 与收编范围，翻案即改工件；其余次要细节（子目录命名、warn 措辞）不构成歧义。）

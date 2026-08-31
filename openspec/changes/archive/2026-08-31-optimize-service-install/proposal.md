## Why

`sebas service --install` 目前生成一个直接运行裸 core（`sebas run`）的 systemd
unit。但 watchd.dog 才是完整的运行模型（监督 core/gateway/webui、控制面 socket、
自升级、`services.json` 期望态持久化）——用 systemd 装成服务后这些能力全部缺失。
此外 unit 烘焙的是安装时的 `current_exe()` 路径，而 `sebas update` 把新版本装到
`data_dir/versions/` 并翻 `current` 软链，两条路径互不相认：升级后重启机器，
systemd 又跑回旧版本。本次改动让 `service --install` 产物对齐 watchdog 运行模型，
并修复路径漂移、缺少加固、restart 语义不明确等问题。

## What Changes

- **BREAKING**: unit 的 `ExecStart` 从 `sebas run --config …` 改为
  `sebas watchdog --config …`。systemd 只监督 watchdog 一个进程；core/gateway/webui
  由 watchdog 拉起，控制面、自升级、期望态持久化随之可用。
- **BREAKING**: `ExecStart` 的二进制路径从「安装时的 `current_exe()`」改为固定安装
  路径 `<data_dir>/bin/sebas`。`data_dir` 由 `config` 的 `[watchdog.storage].data_dir`
  解析，空则按 `--user` 的 home 推导（而非 root 的）。安装时把当前二进制 seed 到
  `<data_dir>/bin/sebas`；`sebas update` 原地替换该文件 → systemd 重启即新版本，
  机器重启也一致。
- 保留 CLI 便利性：best-effort 在 `/usr/local/bin/sebas` 建指向固定路径的软链。
- 新增 `--log-level` 覆盖项：控制烘焙进 unit 的 `RUST_LOG`；未设置时从安装环境
  继承（空则回落 `info`）。
- 加固 unit：`NoNewPrivileges`、`ProtectSystem=full`、`ProtectHome=read-only`、
  `PrivateTmp`；因自升级需替换自身二进制，明确**不**设 `ProtectSystem=strict`。
- 幂等 restart：重写 unit 后恒 `daemon-reload`；unit 已在运行时显式 `restart`，
  使重复 `install` 都是「重载配置并生效」的确定性操作。
- `--user` 校验增加账户存在性检查（`getent passwd`），非 root/非空之外拒绝不存在的
  账户。
- ExecStart 路径按 systemd 转义规则引用（空格/特殊字符不再破坏 unit）。

## Capabilities

### New Capabilities

（无新增能力路径——本次仅修改既有 `cli-service` 能力的行为。）

### Modified Capabilities

- `cli-service`: `service --install` 的 unit 内容、二进制路径解析、日志级别来源、
  systemd 加固、restart 语义、`--user` 校验与 Exit code 集合发生变化。

## Non-goals

- 不改造 `upgrade::install_version` 的 versions/current 目录结构（保留作历史/回滚源），
  只新增「seed 到固定路径」这一步。
- 不引入 macOS launchd 支持（维持 exit 6 + 手写 plist 提示）。
- 不改动 watchdog 内部 spawn 逻辑（`CoreSpawner` 仍用 `current_exe()`——运行中的
  watchdog 本身即固定路径，自洽）。
- 不为 `/usr/local/bin` 软链的创建失败报错（best-effort，CLI 便利非关键路径）。
- 不做单元内容以外的配置模板化（用户配置段仍以 `--config` 单个绝对路径烘焙）。

## Impact

- 代码：`src/service.rs`（run_install、render_unit、校验、exit codes）、
  `src/cli.rs`（`ServiceArgs` 增 `--log-level`）、`src/upgrade.rs`（暴露/distill 一处
  seed 固定路径的 helper）、`src/upgrade.rs`/`src/watchdog/updater.rs`
  （`data_dir` 解析复用）。
- 规格：`openspec/specs/cli-service/spec.md` 的
  `Service unit generation` / `Service install validation and exit codes` /
  `Service start and uninstall` 需求随之更新。
- 依赖：systemd（不变）、`getent`（新校验依赖 `libc getpwnam`，可不用外部命令）。
- 测试：`src/service.rs` 单元测试（渲染/校验）、`cli-service` spec 对照。
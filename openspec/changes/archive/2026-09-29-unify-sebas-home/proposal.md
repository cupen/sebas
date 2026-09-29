## Why

single-state-dir 把 db 与三份 json 统一进了状态目录，但名不正（正名是 `SEBAS_STATE_DIR`，`SEBAS_HOME` 只是 legacy 别名），且仍有成片 sebas 自有落点在它之外、各按各的规则解析：config.toml 缺省挂在进程 cwd（无安全缺省，沙箱必须显式 `-c`）、core.secret 跟 config 目录、channel/control socket 走 `XDG_RUNTIME_DIR`、media 缓存在 `~/.cache/sebas`、sebas-node 状态在 XDG data、watchdog 升级数据在 `dirs::data_dir()/sebas`。「一个变量钉住 sebas 全部落点」今天做不到，漏钉一处就写进操作员真实主目录。

## What Changes

- **`SEBAS_HOME` 升为唯一正名**，语义从「状态目录」扩为「sebas 主目录」：全部自有落点的缺省都从它派生。**BREAKING**：`SEBAS_STATE_DIR` 降为兼容别名——照常生效但启动 warn，二者同设时 `SEBAS_HOME` 赢（优先级反转）。
- **收编剩余自有落点**（显式 config 键 / env 覆盖一律照旧优先）：
  - config.toml 缺省：`./config.toml`（cwd 相对）→ `<SEBAS_HOME>/config.toml`。**BREAKING**：无 `-c` 时不再看 cwd；`SEBAS_ROUTER_CONFIG` 缺席时的 reload 回落同步改。core.secret 维持「与 config 同目录」规则，随缺省一起进 home。
  - channel socket → `<SEBAS_HOME>/run/core.sock`，control socket → `<SEBAS_HOME>/run/control.sock`，不再查 `XDG_RUNTIME_DIR`。**BREAKING**（socket 易逝，无迁移问题）。
  - media 下载缓存 → `<SEBAS_HOME>/cache/downloads`；sebas-node 状态 → `<SEBAS_HOME>/node/`；watchdog 升级数据 → `<SEBAS_HOME>/upgrade/`。
- **既有名册原位不动，零迁移**：四库与 archive/services/nodes json 仍平铺在 home 根，文件名不变；逐文件 env 覆盖（`SEBAS_SETTINGS_DB` 等 6 个）全保留。缺省值不变：未设 `SEBAS_HOME` 时仍是 `~/.sebas`。
- 逻辑名映射表（`sebas-domain/src/state_paths.rs`）扩族收编上述新落点，机械断言随族扩面；不做自动数据迁移，文档给手动迁移指引。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `cli-service`: 「Config discovery」缺省路径改为 `<SEBAS_HOME>/config.toml`；「State directory derives every state path」改为 SEBAS_HOME 正名、派生族扩到 socket/cache/node/upgrade/config、`SEBAS_STATE_DIR` 降别名。
- `state-store`: 「Database location and single-writer ownership」的「单一状态目录」措辞与派生规则更新为 SEBAS_HOME 语境（分层、单写者、WAL 等语义全部不变）。
- `core-session-channel`: 「Channel transport and authentication」通道端点缺省改为 `<SEBAS_HOME>/run/core.sock`。
- `watchdog`: 「Control RPC transport and authentication」控制面 socket 缺省改为 `<SEBAS_HOME>/run/control.sock`。
- `deployment`: 「Uninstall removes service, binaries, data, and user」数据清理面收编为「删除 sebas home」。

## Impact

- **代码**：`sebas-domain/src/state_paths.rs`（映射表扩族 + 正名/别名反转 + warn 检出）、`src/cli.rs`（`-c` 缺省）、`src/config.rs`（secret 路径、media 缺省）、`src/core_channel/server.rs`、`src/watchdog/control_rpc.rs`、`src/upgrade.rs`、`sebas-node/src/config.rs`、`sebas-router/src/config.rs`（reload 回落）。
- **测试与文档**：`tests/support/mod.rs` 与 `tasks.py` 的沙箱钉法、`tests/testsuite_e2e_test.rs` 的 pinned journey 扩面、AGENTS.md 沙箱菜谱、`config/config.toml.example`。
- **运维**：NFS 类网络主目录部署若不宜放 socket，用既有 `SEBAS_CORE_SOCKET` / `[service.core] channel_path` 显式指走（design 记录）。

## Non-goals

- **不收跨工具共享面**：skills 仓 `~/.agents/skills`、sync 落点 `~/.claude/skills` / `~/.codex/skills`、ACP `sessions_dir`（claude CLI 生态路径）、`[workspace] root`、ACP `work_dir`（用户项目数据）维持现状。
- **不做自动数据迁移**：已有 `~/.sebas` 安装的原位不动；指到新目录由操作员手动搬（文档指引）。
- **不引入新 env 变量**：不新增 `SEBAS_CONFIG` 之类；一个 `SEBAS_HOME` + 既有逐文件覆盖与 config 键，变量不增生。
- **不改 XDG 化布局**：缺省仍是 `~/.sebas` 平铺名册，不做 `~/.local/share/sebas` 式重排。

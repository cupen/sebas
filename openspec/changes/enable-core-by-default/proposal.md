## Why

core 是 sebas 的会话权威（session authority），webui 与 im 都是它的通道客户端；`extract-im-service` 把飞书适配器迁出后，「core = 飞书 bot、可选」的历史理由已不成立。既然 core 理应永远在场，`[watchdog.core] enabled` 这个开关本身就是多余的——保留一个「能关掉会话核心」的入口只会制造 webui 半残（session 不可达）的合法但无意义形态。core 应恒启动；作为核心进程，其启动失败必须立刻可见并走 fail-fast 终态。

## What Changes

- **删除 `[watchdog.core] enabled` 键（BREAKING 配置面）**：core 不再可经配置停用，watchdog 恒注册并拉起 core。`WatchdogCoreConfig.enabled` 字段删除；`[watchdog.core]` section 保留 `channel_path` 与 `secret_file`（仍被 core session channel 使用）。`services.json` 中历史 `core: off` 覆盖不再生效（core 恒启）；webui 服务页 / `sebas ctl` 对 core 的 enable/disable 入口移除（重启仍走既有 `RestartCore` 确认路径）。
- **core 启动失败立刻警告并 fail-fast**：core spawn 连续失败达上限（默认 3，`[watchdog] max_spawn_failures`）即进入 `failed-startup` 终态——每次失败写结构化错误日志并经既有汇报面曝光（systemd 状态、`sebas ctl status`），watchdog 停全部子进程并以 EX_TEMPFAIL (75) 退出、输出 startup-failure 摘要，绝不静默续跑。（链路随 `fail-fast-on-startup-errors` 已落地，本次固化为 core 的显式 spec 要求。）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `deploy-mode`: 「Webui-primary deployment shape」重写——core 恒启动（无开关）；「默认部署只起 webui」scenario 反转；「通过 webui 服务页启用 core」scenario 删除（无此入口）。
- `watchdog`: 「Managed service table」——core 为恒启 entry（无 config 开关、无 ServiceSet off）；新增 core fail-fast requirement。
- `feishu-option`: 删除「watchdog 默认只启动 webui 服务，core 停用」的过时描述（core 启停与飞书开关解耦）。
- `webui`: 服务页对 core 仅呈现状态与重启，无 enable/disable 按钮（Services 分区措辞同步）。

## Impact

- 代码：`src/config.rs`（删 `WatchdogCoreConfig.enabled` 与相关断言）、`src/watchdog.rs`（core spec 恒 enabled，不读 config）、`src/watchdog/services.rs`（core 的 ServiceSet off 拒绝/忽略）、webui 服务页前端（core 行去启停按钮）。
- 测试：`tests/support/mod.rs::enable_supervised_core` 与 `tasks.py` 沙箱配置注入 `[watchdog.core] enabled = true` 的逻辑删除（core 默认即起）；依赖「无 core 起 webui」的 e2e 用例需改写为恒有 core。
- 配置兼容：既有部署的 `[watchdog.core] enabled = false` 升级后**失效并告警**（unknown/ignored key），core 照常拉起；`services.json` 的 `core: off` 被忽略。
- 读者：`deploy-mode`、`feishu-option`、`docs/architecture/process-ipc-subcommands.md` 的默认值表。

## Non-goals

- 不删除 `[watchdog.core]` 的 `channel_path` / `secret_file` 键（core session channel 仍用）。
- 不改变 router（默认关）与 im（跟随飞书）的默认判定与开关。
- 不改变 core 的 `RestartCore` 确认路径与 New-binary auto-rollback。
- 不新增 core 失败的通知渠道（复用既有日志 / ctl status / systemd 面）。

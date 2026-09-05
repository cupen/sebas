# 提案：webui 鉴权开关（默认开，测试环境可关）

## Why

webui 登录鉴权落地后，沙箱/联调环境每次 GUI 测试都要先登录，拖慢迭代节奏；而生产（尤其公网部署）又必须默认带鉴权。需要把「是否启用鉴权」做成配置开关：默认打开保证安全底线，测试环境一键关闭。

## What Changes

- 新增配置项 `[watchdog.webui] auth`（默认 `true`）：
  - `true`（默认）：维持现状——凭据存在时 `/api/*`、`/gateway/api/*`、`/ws` 全部需要登录；
  - `false`：即使凭据文件存在也不启用鉴权门，所有路由免登录，`/api/auth/me` 报告 `enabled: false`（前端不弹登录页）。
- 非 loopback bind 的安全门收紧为：`auth = true`（缺省即 true）**且** 凭据存在才放行；开关关闭时无论是否有凭据都拒绝非 loopback bind（防止误关开关 + 公网暴露）。
- 测试沙箱脚本 `scripts/test_webui_sandbox.sh` 默认写 `auth = false`（免登录联调）；`SANDBOX_AUTH=1` 时打开开关并创建统一测试账户 admin/admin（用于测登录流本身）。

## Non-goals

- 不改变凭据存储、PBKDF2 哈希、会话/限速机制本身。
- 不新增 env 覆盖（config 一个字段即可；沙箱脚本直接改写自己的沙箱配置）。
- 不在本变更内补写登录鉴权自身的完整 spec（另行同步）。
- 不改动 admin 控制面（`/api/admin/*`）的 env 密码机制。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：新增「鉴权开关」需求——`watchdog.webui.auth` 的默认值、关闭时的免登录行为、`/api/auth/me` 的语义，以及非 loopback bind 与开关的联动约束。

## Impact

- `src/config.rs`（WatchdogWebUiConfig 加字段 + 默认值测试）
- `src/webui_cmd.rs` / `src/run.rs`（开关为 false 时跳过凭据引导、注入 disabled 态 AuthHandle；非 loopback 门联动）
- `sebas-webui/src/server.rs`（disabled 态行为已存在，需路由级测试覆盖「有凭据但开关关」）
- `config/config.toml.example`、`scripts/test_webui_sandbox.sh`（文档与沙箱默认值）

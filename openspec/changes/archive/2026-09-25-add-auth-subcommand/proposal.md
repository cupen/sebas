## Why

WebUI 账户管理目前只有一个一次性命令 `sebas webui-passwd`（create-or-update 一体），名字不表意、建户与改密靠输出区分，且无法查看库里已有哪些账户。把它重组为 `sebas auth` 组命令（add / passwd / list），CLI 账户管理面才有清晰的动词边界与可发现性。

## What Changes

- 新增 `sebas auth` 组命令，语义按动词拆开：
  - `sebas auth add <USER>`：建户；同名（大小写不敏感）已存在则报错并提示用 passwd。首户缺省 root、其后 member，`--role` 显式覆盖。
  - `sebas auth passwd <USER>`：改密；用户不存在则报错。不携带 `--role`（改角色归 WebUI root 管理面）。
  - `sebas auth list`：只读列表（用户名 / 角色 / 启用 / 时间戳，不含哈希）。
- **BREAKING** 移除 `sebas webui-passwd` 子命令（无别名、无过渡期），所有引用面同步改为 `sebas auth`：cli-service spec 子命令清单、webui spec 鉴权开关需求文案、`webui_cmd.rs` 非 loopback 拒启错误文案、AGENTS.md 沙箱配方、tasks.py 沙箱引导、CLI 集成测试改造。
- 密码语义不变：`--password-stdin` / `--password` 互斥，空密码拒绝，<8 字符仅告警不拦截，明文不落盘。
- auth.db 路径解析不变：`SEBAS_WEBUI_AUTH_DB` env（默认 `~/.sebas/auth.db`），与 webui 运行时同源。

## Capabilities

### New Capabilities

- `auth-cli`: `sebas auth` 组命令（add / passwd / list）的 CLI 账户管理面：各子命令的参数、缺省角色规则、成功/失败语义、密码来源与弱密码告警、auth.db 路径解析、与运行中 webui 的生效时机。

### Modified Capabilities

- `cli-service`: 子命令清单需求——`webui-passwd` 移除，替换为 `auth` 组命令。
- `webui`: `[service.webui] auth` 开关需求中「`sebas webui-passwd` 在开关关闭时仍可管理用户」一句改为指向 `sebas auth`。

## Impact

- 代码：`src/cli.rs`（Cmd 枚举与参数结构）、`src/main.rs`（分发）、`src/webui_cmd.rs`（`run_passwd` 拆分为 add/passwd/list 核心 + 错误文案）、`sebas-webui/src/user_store.rs`（只复用，不改 schema）。
- 测试：`tests/webui_passwd_cli_test.rs` 改造为 `sebas auth` 集成测试；`tests/support/mod.rs`、`tests/testsuite-webui/tests/auth.spec.ts` 注释性引用更新。
- 部署/文档：`tasks.py`（webui-sandbox `--auth` 建号路径）、AGENTS.md。
- 不影响：WebUI 用户管理 HTTP API、RBAC 执法、首启引导（设置页 / env）、用户库 schema 与存量 auth.db 数据。

## Non-goals

- 不在 CLI 增加 set-role / 启停 / 删除（继续由 WebUI root 管理面承载，见 `webui-user-management`）。
- 不引入 `--auth-db` flag 或 config.toml 键，路径维持 env-only。
- 不改变密码强度策略（CLI 保持 <8 字符 warn-only；首启设置页的 ≥8 硬门槛不动）。
- 不做旧命令兼容别名或迁移逻辑（clean-break）。

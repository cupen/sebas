## Context

`sebas webui-passwd`（`src/cli.rs` Cmd 枚举 + `src/webui_cmd.rs::run_passwd`）是
现存的唯一 CLI 账户入口，create-or-update 一体；核心逻辑（stdin 读密、短密码
warn、缺省角色、UserStore 写入）可直接复用。用户库存储层
`sebas_webui::user_store::UserStore`（SQLite、PBKDF2、大小写不敏感唯一名、
UsernameTaken 错误）与路径解析 `sebas_webui::auth::default_auth_db()`（env-only）
均不动。组命令在项目内有先例：`sebas skills`（skills_cmd.rs 薄壳 + sebas::skills
核心）、`sebas node-link`。动机见 proposal.md。

## Goals / Non-Goals

**Goals:**

- `sebas auth` 组命令（add/passwd/list），语义按动词拆分，位置参数用户名
- 彻底移除 `webui-passwd`，代码、测试、脚本、spec、文档引用面全部同步
- 核心账户写入逻辑与密码校验在 add/passwd 间共享，行为与现 webui-passwd 等价
  （除拆分带来的报错语义变化）

**Non-Goals:**

- 不动 UserStore schema、RBAC、WebUI 用户管理 HTTP API、首启引导
- 不做旧命令别名/迁移（clean-break，见 proposal Non-goals）
- 不给 list 做 JSON 输出（如需机器可读，后续 change 再议）

## Decisions

- **D1 组命令放独立模块 `src/auth_cmd.rs`**（与 skills_cmd.rs 并列，一命令一
  模块的项目风格）：`AuthArgs { config-less, subcommands }`——auth 不读
  config.toml（路径 env-only，与现 webui-passwd 一致）。备选：塞进
  webui_cmd.rs——否，该模块已承载 webui 服务进程职责，账户 CLI 面与其无关。
- **D2 `run_passwd` 拆分删除**：建户（缺省角色规则 + create + UsernameTaken
  映射）与改密（set_password）各自成函数，密码来源校验（stdin/flag 互斥、
  空密码、<8 warn）抽共享 helper；`Cmd::WebUiPasswd` 与 `WebUiPasswdArgs`
  整体删除。备选：保留 create-or-update 单函数复用——否，语义拆分是本次
  需求本体。
- **D3 `passwd` 不设 `--role` flag**：clap 层天然拒绝（unexpected argument），
  不需要在业务层报错；spec 场景只钉「参数错误非零退出」，不钉文案。
- **D4 `list` 直读 `store.list()`**：输出人读表格（用户名/角色/启用/时间戳，
  对齐现有字段名），零用户如实输出空；实现为纯格式化函数便于单测。
- **D5 错误文案与退出码沿用现状**：配置类错误走 `SebasError::Config`（现有
  run_passwd 同款），非零退出；不引入新退出码类别。
- **D6 引用面同步清单**（tasks 逐项落地）：
  - `src/cli.rs`：删 `WebUiPasswd`/`WebUiPasswdArgs`，增 `Auth(AuthArgs)`
  - `src/main.rs`：分发点（~123、~591）改接 auth_cmd
  - `src/webui_cmd.rs`：删 run_passwd；`ensure_non_loopback_bind_allowed`
    错误文案与模块注释改指 `sebas auth add <name>`
  - `tests/webui_passwd_cli_test.rs` → `tests/auth_cli_test.rs` 改造
  - `tests/support/mod.rs`、`tests/testsuite-webui/tests/auth.spec.ts`：注释性
    引用更新
  - `tasks.py`（~460、~637）：webui-sandbox `--auth` 建号改
    `sebas auth add admin --password-stdin`
  - `AGENTS.md`：4 处沙箱配方文案
  - 主 spec（cli-service、webui）不动——archive 时由 delta 合并

## Risks / Trade-offs

- [外部脚本/肌肉记忆引用 webui-passwd 直接失效] → clean-break 既定代价（项目
  一贯风格）；tasks 收尾做全库 grep 确认仓库内引用清零
- [沙箱 `admin/admin` 短密码依赖 warn-only] → spec 钉死「短密码告警不拦截」
  场景，集成测试覆盖，防止后续误收紧
- [add/passwd 拆分后，原幂等脚本（重复跑 webui-passwd）第二次会报错] → 报错
  文案明确指引改用对应子命令；沙箱配方只在新建库上跑一次，不受影响
- [大小写不敏感重名判定依赖 store 既有行为] → 集成测试覆盖 `Alice`/`alice`
  场景钉住行为

## Migration Plan

无数据迁移（auth.db schema 不变，存量用户原样可用）。部署即换二进制；
若部署脚本引用 webui-passwd 需手工改为 `sebas auth`。回滚 = 回退二进制
（旧命令恢复，新库数据兼容——schema 未动）。

## Open Questions

无。

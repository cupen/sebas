## 1. auth 组命令核心（src/auth_cmd.rs）

- [x] 1.1 新建 `src/auth_cmd.rs`：共享密码 helper（stdin 读一行去 CR/LF、`--password`/`--password-stdin` 互斥、空密码拒绝、<8 字符 warn）+ 库打开（`auth::default_auth_db()`）+ 用户名位置参数校验；对 spec「来源互斥/缺密码/空密码/短密码告警」场景写单元测试（tempdir + `SEBAS_WEBUI_AUTH_DB` 钉沙箱）
- [x] 1.2 实现 `add <USER> [--role] [--password|--password-stdin]`：缺省角色（零用户→root、否则 member）、`--role` 四档解析（非法报错）、`UsernameTaken` 映射为「已存在，提示用 passwd」；单测覆盖首户 root/后续 member/显式覆盖/重名大小写不敏感
- [x] 1.3 实现 `passwd <USER> [--password|--password-stdin]`（无 `--role` flag）：不存在报「用户不存在，提示用 add」；单测覆盖改密后 `verify_password` 通过
- [x] 1.4 实现 `list`：`store.list()` 纯格式化（用户名/角色/启用/时间戳，无哈希列），零用户输出空；单测断言输出不含哈希与明文

## 2. CLI 接线与旧命令移除

- [x] 2.1 `src/cli.rs`：删 `Cmd::WebUiPasswd` 与 `WebUiPasswdArgs`，新增 `Auth(AuthArgs)` 组（doc 注释指向 auth-cli spec）；`cargo build` 通过
- [x] 2.2 `src/main.rs`：分发改接 `sebas::auth_cmd`，删 `From<cli::WebUiPasswdArgs>`；`sebas auth add/passwd/list --help` 各自可出、`sebas webui-passwd` 报未知子命令（clap 快照断言或手验）
- [x] 2.3 `src/webui_cmd.rs`：删 `run_passwd`/`WebUiPasswdArgs`；`ensure_non_loopback_bind_allowed` 错误文案与模块注释改指 `sebas auth add <name>`；`cargo clippy` 无新告警

## 3. 集成测试与引用面同步

- [x] 3.1 `tests/webui_passwd_cli_test.rs` 改造为 `tests/auth_cli_test.rs`：按 auth-cli spec 场景重排（add 成功/重名、passwd 改密/缺户、list 含字段无哈希、env 覆盖路径、明文不落盘扫描）；`cargo test --test auth_cli_test` 全绿
- [x] 3.2 `tasks.py`（~460、~637）：webui-sandbox `--auth` 建号改 `sebas auth add admin --password-stdin`；`invoke testsuite-webui-sandbox --auth` 起服后 admin/admin 可登录（命令改写已落地且 auth_cli_test 以同形 stdin 管道验证 `auth add --password-stdin`；沙箱实跑登录留待主会话合并后统一回归）
- [x] 3.3 注释性引用清理：`tests/support/mod.rs`（~404、~471）、`tests/testsuite-webui/tests/auth.spec.ts` 头注释；全库 `grep -rn "webui-passwd"` 仅剩 openspec 归档历史（代码/脚本/文档为零）

## 4. 文档与收口

- [x] 4.1 AGENTS.md 4 处沙箱配方改 `sebas auth`（webui-passwd 小节、sandbox 快捷方式段、debug 配方等），语义不变仅换命令
- [x] 4.2 跑 `invoke testsuite-acceptance`（或至少 webui auth 相关用例）确认既有验收不被 breaking 移除波及；`openspec validate add-auth-subcommand` 通过（validate 已过；验收套件留待主会话合并后统一回归——沙箱端口由主工作区并行占用）

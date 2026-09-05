## 1. 配置字段

- [x] 1.1 `src/config.rs`：`WatchdogWebUiConfig` 加 `auth: bool`（`#[serde(default = "default_webui_auth")]`，缺省即 `true`），同步 `Default` impl；新增单测覆盖「缺省 = true / 显式 false 优先」，`cargo test -p sebas config` 相关用例通过
- [x] 1.2 `config/config.toml.example` 的 `[watchdog.webui]` 注释补 `auth` 说明（缺省或 true；测试联调可设 false）

## 2. 接线（design D1/D3/D4）

- [x] 2.1 `src/webui_cmd.rs`：`auth = false` 时跳过 `bootstrap_auth()`、传 `AuthHandle::disabled()`；启动日志在开关关闭时 warn「鉴权未启用」；非 loopback 门改为 `!(auth && auth.enabled())` 拒绝——单测/集成路径断言「0.0.0.0 + 开关关 + 有凭据 → 配置错误退出」
- [x] 2.2 `src/run.rs`（`run --webui` in-process 路径）同样按开关注入 disabled handle，行为与独立进程一致——代码走查 + 现有 run 测试不回归
- [x] 2.3 确认开关关闭时 `/api/auth/me` 返回 `enabled:false`、`/api/auth/login` 返回 401（disabled 语义），`cargo test -p sebas-webui` 全绿

## 3. 路由级测试（specs 场景映射）

- [x] 3.1 `sebas-webui/src/server.rs` 测试：构造「凭据存在 + disabled handle」router，断言 `/api/summary`、`/ws` 免登录 200、`/api/auth/me` 报 `enabled:false`（对应 Scenario「测试环境关闭」）
- [x] 3.2 回归：默认（无开关）+ 凭据存在仍 401（对应 Scenario「默认打开」）——已有 `auth_guard_tests` 保持通过

## 4. 沙箱脚本

- [x] 4.1 `scripts/test_webui_sandbox.sh`：默认写 `auth = false` 且跳过 `webui-passwd`；`SANDBOX_AUTH=1` 时写 `true` 并创建 admin/admin——实跑两种模式各一次，免登录模式 curl `/api/summary` 200、鉴权模式 admin/admin 登录 200
- [x] 4.2 清理验证：Ctrl-C/TERM 退出后进程结束、沙箱目录删除、端口释放

## 5. 收尾

- [x] 5.1 沙箱全流程回归：`bash scripts/test_webui_sandbox.sh`（默认免登录）+ `SANDBOX_AUTH=1` 两种模式过一遍 GUI/curl，`openspec validate` 通过

## 6. 字段重命名（update-change 修订：auth_enabled → auth）

- [x] 6.1 代码与测试中的 `auth_enabled` 全量改名为 `auth`（`src/config.rs` 字段与 `default_webui_auth`、`src/webui_cmd.rs`、`src/run.rs`、`sebas-webui` 相关测试、`config/config.toml.example`、`scripts/test_webui_sandbox.sh`），不保留兼容别名；`cargo test`（config + auth_gate + sebas-webui auth_guard）与沙箱两种模式回归通过

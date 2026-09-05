# 设计：webui 鉴权开关（add-webui-auth-switch）

## Context

登录鉴权已落地（sebas-webui `auth` 模块）：凭据文件存在 → `/api/*`、
`/gateway/api/*`、`/ws` 全部过 `auth_guard`；`AuthHandle::disabled()` 态
（无凭据）即全放行。开关要做的只是给「disabled 态」一个显式的、可配置的
入口。见 proposal.md。

## Goals / Non-Goals

**Goals:**

- 一个配置字段控制鉴权总开关，默认 `true`，生产行为零变化。
- `false` 时复用现有 disabled 语义：免登录、`/api/auth/me` 报 `enabled:false`。
- 非 loopback bind 安全门与开关联动，关开关不能制造公网裸奔路径。
- 沙箱脚本默认免登录联调。

**Non-Goals:**

- 不动凭据文件格式、PBKDF2、SessionStore、限速（见 proposal Non-goals）。
- 不为开关加 env 覆盖——config 字段足够，避免第三种配置来源。

## Decisions

### D1：开关在进程边界生效——`false` 时注入 `AuthHandle::disabled()`

**选择**：`webui_cmd::run` / `run.rs` 在 `auth = false` 时跳过
`bootstrap_auth()`，直接传 `AuthHandle::disabled()` 给 `build_router_full`。

**理由**：`auth_guard`、`/api/auth/me`、`/api/auth/login` 全部已经按
「凭据是否存在」分支，disabled 态即天然免登录且 `me` 报 `enabled:false`。
零新增中间件逻辑，一处接线全局生效。

**备选**：在 `auth_guard` 里读开关逐请求判断——要多传一份配置进
`WebUiState`，且 `me` 端点还要单独处理，两条路径容易漂移。否决。

**注意**：disabled 态下 `/api/auth/login` 会返回 `Disabled` 错误
（`LoginError::Disabled` → 当前映射为 401）。前端开关关闭时根本不会展示
登录页，直接调 login 仅出现在误用场景，401 可接受，不为它加特判。

### D2：字段名与位置：`[watchdog.webui] auth`，serde 默认 `true`

**选择**：`WatchdogWebUiConfig` 加 `#[serde(default = "default_webui_auth")] pub auth: bool`，
`default_webui_auth() -> true`，并实现/更新 `Default`。

**理由**：挂在 webui 配置节下与 `host`/`port` 同级，语义内聚；serde
default 保证「缺省即 true」。字段名直接用 `auth`：bool 字段读作「鉴权
开/关」最短形式；出正式版前旧配置作废，不做 auth_enabled 兼容别名。

### D3：非 loopback 门收敛到「开关 on 且凭据存在」

**选择**：`webui_cmd::run` 的判断改为
`!endpoint.is_loopback() && !(cfg.watchdog.webui.auth && auth.enabled())` → 配置错误。
开关关闭时即使凭据存在也拒绝非 loopback bind。

**理由**：关开关的意图就是「我要免鉴权」，此时公网 bind 只能是误配——
在启动时硬失败比运行时静默放行安全。loopback 沙箱不受影响。

**备选**：开关关闭仅告警不拦截——公网 + 无鉴权的组合是真实事故形态，
不值得赌 warn 会被看到。否决。

### D4：`run --webui`（in-process 路径）同样接线

`run.rs` 与独立 webui 进程读同一配置字段，行为一致；该路径恒绑
127.0.0.1，开关关闭时行为不变。

### D5：沙箱脚本默认 `auth = false`

`scripts/test_webui_sandbox.sh` 默认写入关闭行并跳过 `webui-passwd`；
`SANDBOX_AUTH=1` 时写 `auth = true` 并创建 admin/admin（测登录流）。
GUI 免登录联调是常态路径，登录流验证是显式 opt-in。

## Risks / Trade-offs

- [开关关闭 + 凭据存在，用户误以为还有登录保护] → `/api/auth/me` 明确报
  `enabled:false`；webui 启动日志在开关关闭时打 warn 提示鉴权未启用。
- [disabled handle 下 `webui-passwd` 热重载会把开关「假打开」？] → 不会：
  disabled handle 的 path 为空，`reload_if_changed` 探测不到文件，凭据
  永远为 None；重新启用开关只能靠重启进程，行为确定。
- [旧配置文件缺字段] → serde default true（缺省即 true）；config.example 补注释。

## Migration Plan

正式版前旧配置作废、不考虑兼容：缺省即为 `true`，不保留 auth_enabled 别名。回滚 = 删字段或设回 `true`。
无数据迁移。

## Open Questions

（无）

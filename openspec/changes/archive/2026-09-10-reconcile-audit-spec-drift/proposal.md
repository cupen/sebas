## Why

双轮审计（架构驱动 + 逐行）在 34 个 spec 中发现大量「spec 落后于有意的代码演进」的漂移。其中代码侧 bug 已在代码修复（beads：sebas-489/539/a7s/cgq/cac 部分/kwf 部分/034/zer）；本 change 记录**纯 spec 文本修正**批次——每处都是代码行为被确认为有意设计（安全加固、架构搬迁、sebas-ixv 交互改动）后，spec 文本向现实对齐。

## What Changes（直接修 openspec/specs/，逐处列出）

- **router-auth-rate-limit**：「Open router when unconfigured」改为 loopback-gated（commit f323eba 有意加固：非 loopback + 无 token → 401；仅 `--debug` 全开放）；access log 401 行 `-@-` → `-`（并消除 spec 自身矛盾）；admin/metrics 请求不记 access log（axum merge 语义）。
- **webui**：`POST /api/sessions/{key}/model`（spec 误写 PUT；server+frontend+testsuite 全用 POST）；route 表补 `/api/sessions/{key}/model`、`/api/agent-kinds`、`/api/auth/logout`、`/router/api/presets`；删除过时的「非 loopback + 凭据缺失 → 拒绝启动」场景（凭据自动引导使其不可达）；「Optional admin authentication」对齐现实（无 /admin/login 重定向，未认证 → JSON 401 + SPA fallback）。
- **cli-service**：startup-failure 摘要目标去掉不存在的 `--log-file`（design D2 契约只有 `SEBAS_STARTUP_ERROR_FILE` + stderr）；子命令树补 `im` 与 `status`/`services` 别名；`--log-level` 「validated non-empty」改为「空值按未设置回退」；webui 缺 `SEBAS_CONTROL_SECRET` 是 warn+只读降级而非 exit 75。
- **watchdog**：`ServiceSet{core}` 对 CLI/WebUI actor 已放行（sebas-2ty；仅 Feishu actor 拒绝）；webui 默认启（watchdog 唯一默认拉起的服务）；spawn-failure 重试退避 5s（非 1s）；core 子进程 stderr inherit（非全 piped）；pipe 协议 Ready-only（无 early-fatal 行）；rollback 事件走 `ctl events`；up-to-date update 也重启 core（现状）。
- **dispatch-commands**：无会话的 `/status` `/cost` `/compact` `/cancel` 从静默改为显式提示（sebas-ixv）；`/switch` `/resume` `/cd` 已解析但回复「暂未支持」（不再无响应）；IM 路径 `/settings` 走状态库 settings 域（输出 JSON 快照，不显示文件路径）；IM 路径 `/new` = close+ensure（allowlist 由 close 清理，语义不变）；`/help` 交互卡与 `/compact` 进度卡为引擎路径能力，IM 前端为简化版。
- **replay-debug**：`sebas core --dump-inbound` → `sebas im --dump-inbound`（CoreArgs 无该 flag）；录制（原始飞书 envelope）与回放（中立 ChannelEvent）形状分离——回放跳过 pre-neutralization dump；「shared frame handler」「filter divergence」「dedup during replay」requirement 按 im-service 后现实删除/重写。
- **agent-driver**：AcpEvent 词表补 `ModelChanged`。
- **agent-bench**：bucket 名对齐代码（`core`/`web-tooling`/`apply_patch`/`subagent`）；「sebas agent-bench 子命令」在代码侧补齐（见 tasks）。

## Capabilities

（全部为 spec 文本对齐已实现行为，无新增/修改 requirement 语义的 change delta——直接修主 spec 并以此 skip_specs change 记账。）

## Impact

- 纯 spec 文本 + 一处小代码补齐（`sebas agent-bench` 子命令）。
- 与 operator 在途 refactor（workbench 拆分等）不重叠：触及的 spec 均不在其变更集内。

## Non-goals

- 不处理 agent-core 的真特性缺口（long-running pool、policy 用户可配、`permission_decision` outcome）——已在 beads 记录，待立项。
- 不动 operator 在途的 spec（workbench/acp-* 拆分、feishu-option）。

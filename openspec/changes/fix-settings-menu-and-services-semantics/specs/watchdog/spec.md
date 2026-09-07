## MODIFIED Requirements

### Requirement: Managed service table
The watchdog SHALL supervise all child processes through one declarative table of managed services — core, webui, router, and im — where each entry declares its spawn specification (argv, env), its desired state (from config or `ServiceSet`), and its restart policy. **补充**：受管服务名在 RPC/REST 接口与 control RPC 中 SHALL 统一使用 `core` / `webui` / `router` / `im` 四个字符串（产品对外 IM 即「飞书」，但内部字符串 SHALL 为 `im` 以与 `ServiceName::Im` 枚举对齐）；webui 前端 SHALL 通过 `service_from_str("im")` 识别，禁止使用 `feishu` 作为内部名。监督循环 SHALL 维持对全部 entry 的均匀处理（spawn / 退出分类 / 重启决策），New-binary auto-rollback 仍是 core 专属。config 中 disabled 的服务 SHALL 无子进程被拉起、状态报告为 `disabled`。core 子进程的 pipe 协议 SHALL 仅承载 readiness 握手（含 early fatal-error 行），控制操作不再经 pipe，control RPC socket 是唯一命令面。

#### Scenario: router managed when enabled

- **WHEN** the watchdog config enables router management
- **THEN** the watchdog spawns `sebas router --config <path>` as a supervised child and `ServiceStatus` includes a real router entry

#### Scenario: im managed when enabled

- **WHEN** the watchdog config enables IM management (explicitly or via the feishu-enablement default)
- **THEN** the watchdog spawns `sebas im --config <path>` with the control secret and `ServiceStatus` includes a real im entry

#### Scenario: disabled service reports disabled

- **WHEN** `ServiceStatus` is queried while webui management is disabled in config
- **THEN** the webui entry reports state `disabled` and no child exists

#### Scenario: upgrade commands only via RPC

- **WHEN** the core child wants an upgrade or rollback executed
- **THEN** it sends the request over the control RPC socket; the pipe carries only the readiness handshake

#### Scenario: im 字符串命名约定

- **WHEN** webui 通过 `/api/admin/services` 列出受管进程
- **THEN** IM/飞书服务的 name 字段 SHALL 为 `im`（不是 `feishu`）；前端识别 SHALL 经 `service_from_str("im")`，使用 `feishu` 调用将得到 `UnknownService` 错误
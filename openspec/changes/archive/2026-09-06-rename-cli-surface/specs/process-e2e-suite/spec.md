## RENAMED Requirements

- FROM: `### Requirement: gateway debug provider 应答`
- TO: `### Requirement: router debug provider 应答`

## MODIFIED Requirements

### Requirement: 沙箱全隔离

每个用例 MUST 运行在一次性沙箱内：配置文件落在 scratch 目录（含 dispatch state_file、media download_dir、acp sessions_dir/work_dir、watchdog.core channel_path、watchdog.webui host/port），端口 SHALL 不同于 9797；环境变量 MUST 全量覆盖默认值——伪造 `SEBAS_CORE_SECRET`，并显式设置 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_ROUTER_PROVIDER_OVERLAY` 指向沙箱路径。套件 MUST NOT 读写操作员真实 `~/.sebas`、真实凭据或占用其端口。

#### Scenario: 与操作员实例完全隔离

- **WHEN** 套件在存有运行中操作员实例（端口 9797、真实 `~/.sebas`）的机器上执行
- **THEN** 所有进程只绑定沙箱端口、只读写沙箱目录，操作员实例不受任何影响

#### Scenario: 用例结束清理沙箱

- **WHEN** 任一用例结束（无论通过或失败）
- **THEN** 其 scratch 目录被清理（保留给事后排查的除外），不遗留守护进程

### Requirement: detached 拓扑启动可达性

套件 SHALL 验证 detached 形态启动：核心 `sebas core`（router debug 模式）与独立 `sebas webui` 进程经核心通道连接后，webui 的 `/health` 返回 ok，`/api/summary` 的 `reachability.ok` 为 true。

#### Scenario: 双进程启动后 webui 报告可达

- **WHEN** 核心 webui 两个进程按沙箱配置启动且核心通道握手完成
- **THEN** webui `GET /health` 返回 ok，`GET /api/summary` 返回 `reachability.ok = true`

### Requirement: router debug provider 应答

套件 SHALL 验证核心内置 debug 路由可用：对路由 `/v1/messages` 以 debug `test` 模型发起请求返回 200 与固定应答。

#### Scenario: test 模型请求命中内置应答

- **WHEN** 向沙箱路由 `POST /v1/messages` 提交 `model = "test"` 的请求
- **THEN** 返回 200 且应答为 debug provider 的固定内容

# process-e2e-suite Specification

## Purpose

以真实二进制组成 detached 拓扑（独立 core 进程 + 独立 `sebas webui` 进程经核心通道相连）对核心流程做进程级端到端校验，全部路径沙箱化，并提供一键运行入口，让"核心流程是否正确"可以单条命令验证。

## Requirements

### Requirement: 一键运行入口

套件 SHALL 提供单条命令入口 `invoke e2e`：先构建工作区二进制（含 `sebas` 与 `fake-claude`），再运行全部进程级 e2e 用例。套件 MUST 也能不经 invoke 直接以 `cargo test --test core_flow_e2e_test -- --ignored` 运行。命令的退出码 SHALL 如实反映套件通过与否。

#### Scenario: 一条命令完成构建与校验

- **WHEN** 操作员在仓库根执行 `invoke e2e`
- **THEN** 工作区完成构建后套件全部用例被执行，任一用例失败则命令以非零码退出，全部通过则以零码退出

#### Scenario: 不依赖 invoke 也可运行

- **WHEN** 操作员执行 `cargo test --test core_flow_e2e_test -- --ignored`
- **THEN** 套件用例同样全部运行（二进制已构建的前提下）

### Requirement: 沙箱全隔离

每个用例 MUST 运行在一次性沙箱内：配置文件落在 scratch 目录（含 router state_file、media download_dir、acp sessions_dir/work_dir、watchdog.core channel_path、watchdog.webui host/port），端口 SHALL 不同于 9797；环境变量 MUST 全量覆盖默认值——伪造 `SEBAS_CORE_SECRET`，并显式设置 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_GATEWAY_PROVIDER_OVERLAY` 指向沙箱路径。套件 MUST NOT 读写操作员真实 `~/.sebas`、真实凭据或占用其端口。

#### Scenario: 与操作员实例完全隔离

- **WHEN** 套件在存有运行中操作员实例（端口 9797、真实 `~/.sebas`）的机器上执行
- **THEN** 所有进程只绑定沙箱端口、只读写沙箱目录，操作员实例不受任何影响

#### Scenario: 用例结束清理沙箱

- **WHEN** 任一用例结束（无论通过或失败）
- **THEN** 其 scratch 目录被清理（保留给事后排查的除外），不遗留守护进程

### Requirement: 单用例手动运行与现场保留

套件用例 SHALL 使用语义化且稳定的名称，使单个用例可经 cargo 过滤器手动运行：`cargo test --test core_flow_e2e_test <用例名> -- --ignored`；`invoke e2e` SHALL 提供 `--case` 参数把用例名透传为该过滤器（缺省仍运行全部）。任一用例失败时 MUST 保留其沙箱目录（含核心与 webui 日志）并向输出打印路径，供事后排查；保留目录落在 `target/tests/` 下，由 `cargo clean` 兜底清理。

#### Scenario: 按名称单独运行一个用例

- **WHEN** 开发者执行 `invoke e2e --case <用例名>` 或等价的 cargo 过滤命令
- **THEN** 仅该用例被运行，其余用例不执行，退出码如实反映该用例结果

#### Scenario: 失败保留现场

- **WHEN** 任一用例失败
- **THEN** 该用例的沙箱目录（含 core 与 webui 日志）被保留，输出打印沙箱与日志路径，可按同一配置手动复现

### Requirement: detached 拓扑启动可达性

套件 SHALL 验证 detached 形态启动：核心 `sebas run`（gateway debug 模式）与独立 `sebas webui` 进程经核心通道连接后，webui 的 `/health` 返回 ok，`/api/summary` 的 `reachability.ok` 为 true。

#### Scenario: 双进程启动后 webui 报告可达

- **WHEN** 核心 webui 两个进程按沙箱配置启动且核心通道握手完成
- **THEN** webui `GET /health` 返回 ok，`GET /api/summary` 返回 `reachability.ok = true`

### Requirement: 会话往返全链路

套件 SHALL 经 webui 进程的 HTTP 面验证完整会话往返：创建会话（ACP 执行体）→ 消息经核心通道驱动 ACP 子进程（fake-claude 桩）→ 会话状态到达 Done 且应答内容可查。

#### Scenario: 创建会话到回合完成

- **WHEN** `POST /api/sessions` 以 ACP 后端提交一条文本，随后轮询会话状态
- **THEN** 会话最终状态为 Done，会话详情含 fake-claude 的应答文本

### Requirement: gateway debug provider 应答

套件 SHALL 验证核心内置 debug 网关可用：对网关 `/v1/messages` 以 debug `test` 模型发起请求返回 200 与固定应答。

#### Scenario: test 模型请求命中内置应答

- **WHEN** 向沙箱网关 `POST /v1/messages` 提交 `model = "test"` 的请求
- **THEN** 返回 200 且应答为 debug provider 的固定内容

### Requirement: 错误核心密钥拒连

套件 SHALL 验证密钥不匹配时连接被如实拒绝：以与核心不同的 `SEBAS_CORE_SECRET` 启动 webui，webui 不得以"已连接"状态继续服务。

#### Scenario: 密钥不匹配时可达性如实上报失败

- **WHEN** webui 进程携带错误 `SEBAS_CORE_SECRET` 启动并请求 `/api/summary`
- **THEN** `reachability.ok` 为 false 且给出 cause，不出现虚假的已连接状态

### Requirement: 核心重启期间可达性翻转

套件 SHALL 验证核心生命周期变化在 webui 侧如实可见：核心进程退出后 `reachability.ok` 翻转为 false（含 cause），核心重新启动后翻回 true。

#### Scenario: 核心停止再拉起

- **WHEN** 核心进程被终止，随后以同一沙箱配置重新启动
- **THEN** 停止窗口内 webui `/api/summary` 的 `reachability.ok` 为 false，核心恢复后变回 true

### Requirement: 优雅退出清理

在支持 SIGTERM 的平台上（unix 门控），套件 SHALL 验证核心收到 SIGTERM 后优雅退出：核心通道 socket 文件被移除，会话状态落盘。

#### Scenario: SIGTERM 后通道痕迹消除

- **WHEN** 向运行中的核心发送 SIGTERM 并等待其退出
- **THEN** channel_path 的 socket 文件不再存在，状态文件包含退出前会话状态

### Requirement: 有界时间与平台门控

每个用例 MUST 以显式超时为界（禁止无界等待），套件总时长 SHOULD 控制在数分钟内。平台相关用例（如 SIGTERM）MUST 按平台条件编译门控，在不支持的平台上跳过且不判失败。套件用例 MUST 以 `#[ignore]` 标注，不进入默认 `cargo test` 路径。

#### Scenario: 平台不支持时跳过而非失败

- **WHEN** 套件在无 SIGTERM 语义的平台（如 Windows）上运行
- **THEN** 平台门控用例被跳过，其余用例正常运行，套件整体不因此失败

#### Scenario: 挂起子进程不拖垮套件

- **WHEN** 任一外部依赖（核心/webui/ACP 子进程）未在用例超时内给出预期响应
- **THEN** 该用例在超时处失败并留下可诊断的日志/残留路径，套件其余用例继续执行

## MODIFIED Requirements

### Requirement: 沙箱全隔离

每个用例 MUST 运行在一次性沙箱内：配置文件落在 scratch 目录（含 dispatch state_file、media download_dir、acp sessions_dir/work_dir、service.core channel_path、service.webui host/port），端口 SHALL 不同于 9797；环境变量 MUST 全量覆盖默认值——伪造 `SEBAS_CORE_SECRET`，并显式设置 `SEBAS_STATE_DB`、`SEBAS_STATE_FILE`、`SEBAS_ROUTER_PROVIDER_OVERLAY` 指向沙箱路径。套件 MUST NOT 读写操作员真实 `~/.sebas`、真实凭据或占用其端口。

#### Scenario: 与操作员实例完全隔离

- **WHEN** 套件在存有运行中操作员实例（端口 9797、真实 `~/.sebas`）的机器上执行
- **THEN** 所有进程只绑定沙箱端口、只读写沙箱目录，操作员实例不受任何影响

#### Scenario: 用例结束清理沙箱

- **WHEN** 任一用例结束（无论通过或失败）
- **THEN** 其 scratch 目录被清理（保留给事后排查的除外），不遗留守护进程

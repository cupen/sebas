# Delta — core-session-channel

## ADDED Requirements

### Requirement: Channel auto-arm without injected secret

core SHALL 在 `SEBAS_CORE_SECRET` 缺失或为空时仍武装核心会话通道：现场生成随机 secret，并将其实时写入 secret 文件（路径从与 socket 一致的 config 解析，文件权限 0600）。env 显式提供 secret 时 SHALL 优先使用 env 值，且 secret 文件仍须写入（供迟启动的组件发现）。secret 文件写入 SHALL 原子替换（tmp + rename）。

#### Scenario: 无 env 启动时通道自动武装

- **WHEN** core 以不含 `SEBAS_CORE_SECRET` 的环境启动且 config 指向沙箱内 socket 路径
- **THEN** 通道 socket 在解析路径上出现，secret 文件在解析路径上出现且内容为本次启动的随机 secret，权限为 0600

#### Scenario: env 提供时 env 优先且文件仍落盘

- **WHEN** core 以非空 `SEBAS_CORE_SECRET` 启动
- **THEN** 通道以 env 值作握手 secret，secret 文件内容与 env 值一致

### Requirement: Secret file discovery for channel clients

通道客户端（webui、router 订阅、im）SHALL 按以下顺序解析握手 secret：`SEBAS_CORE_SECRET` env 优先；env 缺失时读取 secret 文件；两者皆缺省时以空 secret 连接并在启动时输出 warn（不静默）。当连接因 secret 不匹配失败且 env 未设时，客户端重连前 SHALL 重读 secret 文件——core 重启换钥后，已在运行的客户端 SHALL 在无人工干预下恢复连接。

#### Scenario: 独立 webui 无 env 经文件连上

- **WHEN** 独立 `sebas webui` 与 core 使用同一份 config，webui 环境无 `SEBAS_CORE_SECRET`
- **THEN** webui 读取 secret 文件完成握手，`/api/summary` 的 `reachability.ok` 为 true

#### Scenario: core 重启换钥后运行中的 webui 自愈

- **WHEN** core 被重启（生成新随机 secret 并覆写 secret 文件），webui 进程不重启
- **THEN** webui 在重连退避内恢复 `reachability.ok = true`，期间报过的 cause 如实反映 secret 不匹配或 socket 缺失

#### Scenario: env 与文件皆缺省时启动告警

- **WHEN** 通道客户端启动时 env 缺失且 secret 文件不存在
- **THEN** 客户端输出包含"核心通道 secret 未找到"语义的 warn 日志并继续以空 secret 尝试连接（不崩溃、不静默）

### Requirement: Channel bind failure is a hard startup failure

通道 socket bind 失败（路径被存活进程占用且拒绝回收等）SHALL 使 core 以 bind 失败退出码（与 webui 的 75 语义一致）退出，而不是继续运行一个无通道的"健康"进程。

#### Scenario: socket 被存活进程占用时启动失败

- **WHEN** 目标 socket 路径已被另一个存活 core 占用，新 core 启动
- **THEN** 新 core 以 bind 失败退出码退出，supervisor 可据此标记 Degraded 而非无限重启

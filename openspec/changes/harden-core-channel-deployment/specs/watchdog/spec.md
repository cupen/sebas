# Delta — watchdog

## ADDED Requirements

### Requirement: Readiness implies channel armed

core 子进程的 readiness 信号 SHALL 在核心会话通道武装完成之后发出：watchdog 观察到 ready 时，通道 socket 必已存在于解析路径。由此消灭"进程 Running 但通道永不武装"这一监督盲区。

#### Scenario: ready 之后 socket 必然存在

- **WHEN** watchdog 启动 core 并收到其 readiness 信号
- **THEN** 此时按同一 config 解析的通道 socket 路径上存在可连接的 socket

#### Scenario: 通道武装失败不产生 ready

- **WHEN** core 的通道 socket bind 失败（路径被存活进程占用）
- **THEN** core 以 bind 失败退出码退出且从未发出 ready，supervisor 依据退出码标记 Degraded（对齐 webui bind 失败语义），不进入无限重启循环

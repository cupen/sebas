校对结论（3.1）：docs/architecture.md 已覆盖 proposal.md What Changes 全部五项——
进程全景（§1，含 ASCII 全景图，子命令与 src/cli.rs::Cmd 枚举一一对应）、
watchdog 监督模型（§2，三服务职责 + spawn→readiness 门→崩溃退避生命周期图 + 默认启动策略）、
IPC 语义（§3，管道仅 readiness 握手 + control RPC 全操作表 + core session channel 分工）、
CLI 控制面与 crate 职责速查（§4，与 Cargo.toml members 一致）、
关键结论均带来源标注（§2.4 随机抽查 8 处：cli.rs Cmd、supervisor.rs 常量与状态机、
ipc.rs ready、control_rpc.rs default_socket_path、services.rs initial_desired、
core_channel/server.rs default_socket_path、config.rs channel_path、Cargo.toml members——
全部真实存在且内容相符）。无未来计划或未实现设想混入。

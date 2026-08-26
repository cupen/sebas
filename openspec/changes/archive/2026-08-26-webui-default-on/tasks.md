# Tasks: webui-default-on

## 1. 配置默认值翻转

- [x] 1.1 `src/config.rs`：`WatchdogWebUiConfig::default()` 与 serde
  default 将 `enabled` 翻转为 `true`；同步更新 `WebUiEndpoint::from_config`
  内的 `enabled` 检查（D6）。验证：`cargo test -p sebas config` 通过，
  新增单测验证「无配置段 = 启用」「显式 false = 关闭」
- [x] 1.2 更新 `config/config.toml.example`：`[watchdog.webui]` 段注释
  标注新默认值，并提示端口冲突可设 `enabled = false` 或改 `port`

## 2. 子进程 bind 失败退出码

- [x] 2.1 `src/webui_cmd.rs`：bind 失败时以 `std::process::exit(75)` 退出
  而非返回 `Err`；提取常量 `EXIT_BIND_FAILED = 75` 到 `src/watchdog/`
  共享模块供 supervisor 判断。验证：单测 mock bind 失败，断言子进程
  退出码为 75；`cargo test -p sebas webui_cmd` 通过

## 3. Supervisor Degraded 状态

- [x] 3.1 `src/watchdog/supervisor.rs`：`ServiceState` 新增 `Degraded`
  变体；`supervise()` 的 `Exit::Crashed` 分支新增检查退出码是否为 75：
  - 是 75 → 标记 `Degraded`，记 warning 日志，`cmd_rx.recv()` 等待
    Restart/Stop 命令，不自动重试
  - 非 75 → 维持原 crash backoff 行为
- [x] 3.2 `src/watchdog/executor.rs` + `control_rpc.rs`：`service_status`
  等序列化/反序列化同步支持 `Degraded` 状态输出。验证：
  `cargo test -p sebas watchdog` 通过，包括新增的 degraded 单测
- [x] 3.3 新增单测：degraded 服务不自动重试、restart 命令复位 degraded
  并成功 spawn。验证：`cargo test -p sebas supervisor` 新测通过

## 4. CLI 对称 flag

- [x] 4.1 `src/cli.rs` / `src/main.rs`：新增 `--no-webui`，与 `--webui`
  互斥（同时给出即报错）；`--no-webui` 时不启动 WebUI。验证：
  `cargo test -p sebas webui` flag 解析测试

## 5. 清理

- [x] 5.1 合并 `should_start_watchdog_webui`：将 `src/watchdog/services.rs`
  的辅助函数内联到 `run_watchdog` 中直接读 `config.webui.enabled`（D6）
- [x] 5.2 全量 `cargo test --workspace` + `cargo clippy --workspace` 通过；
  `openspec validate webui-default-on` 通过
# Proposal: cross-platform-e2e-testsuite

## Why

进程级 e2e 套件是 sebas 的主验收手段,但它今天只能在 Linux 上编译:`testsuite_e2e_test.rs` 存在未门控的 `/proc` 依赖,在 Windows(开发机)上 `cargo test` 直接编译失败;harness(tasks.py)还有会误杀进程的 `os.kill(pid, 0)` 探活和 Windows 上不存在的 `signal.SIGKILL` 引用;测试沙箱在 Windows 上不收割子进程树,孤儿进程会污染开发机。而 release 已在官方出 Windows 二进制——验收能力却单平台,跨平台主张无法自证。

## What Changes

- 修复 `testsuite_e2e_test.rs` 平台门控:未门控的 `/proc` 依赖(`find_child_pid` 调用点、`/proc/{pid}/cmdline` 读取)收进 linux 门控,误被门控的 `Arc` 导入解除——套件在 Windows/Linux 双平台可编译、可运行。
- Windows 进程树收割:testsupport `kill_process_groups` 非 unix 分支从"清空列表"改为 `taskkill /PID <pid> /T /F`,消除孤儿进程。
- tasks.py 加固:平台安全探活 helper(Windows 用 ctypes `OpenProcess`,不再 TerminateProcess)、`SIGKILL` 引用改平台分支。
- 散件门控补齐:`sigterm_cleanup_test.rs` 二进制解析补 `.exe` 处理;`state_subscription_test.rs` 维持整文件门控。
- unix 专属 journey(SIGTERM 优雅退出、watchdog 恢复)在 Windows 诚实跳过,沿既有 skip-not-fail 约定。
- 验收证据:Windows 本机 + WSL 双平台实跑 e2e 与 acceptance 套件,记录跳过清单。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `testsuite-process-e2e`:平台门控从"容忍性跳过"升级为双平台 MUST——套件在 Linux 与 Windows 上 SHALL 可编译、可运行、跑绿(平台专属用例门控跳过);新增 Windows 进程树收割与 harness 平台安全探活要求。

## Impact

- 代码:`tests/testsuite_e2e_test.rs`、`tests/support/mod.rs`、`tests/sigterm_cleanup_test.rs`、`tasks.py`。
- 不动产品 `src/`(Windows 优雅退出产品缺口单列 beads issue)。
- 不动 CI。

## Non-goals

- CI 矩阵改造(不加 windows runner、不在 CI 跑 ignored 套件)。
- 产品代码 Windows 优雅退出通道(CTRL_BREAK_EVENT)——单列 change。
- Playwright webui-browser 套件跨平台化。
- macOS 门槛(维持门控跳过、尽力而为)。
- 真实 provider 验收矩阵(保持 `testsuite-real-agents` 手动入口现状)。

# Design: cross-platform-e2e-testsuite

## Context

侦查结论(证据见 change 讨论记录):

- `tests/support/mod.rs` 的沙箱基建(目录、`forward_slash` config、端口探测、`CARGO_BIN_EXE_*`、tokio spawn + kill_on_drop、HTTP 轮询)与 `sebas-ipc` 传输层(Unix socket ↔ Windows named pipe)已跨平台;acceptance 套件已有完整 `#[cfg(unix)]` 门控 + Windows 硬杀回退。
- 断裂点:① `testsuite_e2e_test.rs` 未门控的 `/proc` 依赖(`find_child_pid` 定义 `#[cfg(target_os = "linux")]` 于 :823,调用点 :1317/:1397 未门控,`:1404` 直读 `/proc/{pid}/cmdline`;`Arc` 导入 :17 被 `#[cfg(unix)]` 门控却在 :1289 未门控使用)——Windows/macOS 编译失败;② `kill_process_groups` 非 unix 分支只清空列表(`tests/support/mod.rs:170-179`),`kill_on_drop` 只收割直接子进程,孙进程孤儿化;③ tasks.py `os.kill(pid, 0)` 探活(:57、:703)在 Windows 是 TerminateProcess 语义,`signal.SIGKILL`(:125、:711)在 Windows Python 是 AttributeError;④ `sigterm_cleanup_test.rs` 手动走 `target/debug` 解析二进制无 `.exe` 处理(:53-60)。

## Goals / Non-Goals

**Goals:**

- e2e 套件在 Linux + Windows 双平台可编译、可运行、跑绿(平台专属用例门控跳过)。
- Windows 沙箱拆卸收割整棵进程树,不留孤儿。
- tasks.py 探活/清理在任何平台都不误杀进程、不因缺失信号崩溃。

**Non-Goals:**

- 产品代码 Windows 优雅退出通道(CTRL_BREAK_EVENT)——单列 change/beads issue。
- CI 矩阵、macOS 门槛、Playwright 套件、真实 provider 矩阵——见 proposal Non-goals。

## Decisions

### D1. 门控策略:拆用例而非整测试门控

`core_owned_provider_reaches_router_without_restart` 的路由 journey 本身平台中立,拆成可移植主体(HTTP/路由断言)+ `#[cfg(target_os = "linux")]` 的进程树断言段;`Arc` 导入解除门控。

- 备选:整个测试加 linux 门控 → Windows 上白白丢一条核心 journey 的覆盖,拒绝。
- 备选:给 Windows 实现 Toolhelp32 等价进程遍历 → 为一个测试 helper 引入 Win32 FFI,过度,拒绝。
- `/proc` cmdline 直读同归 linux 门控段。socket 文件存在性断言(:674-685、:722-725)**维持 unix 门控**:named pipe 无文件系统痕迹,Windows 上该断言模式本身不成立(in-crate 已有 cfg-split `wait_channel_gone` 连接探测先例,`src/core_channel/tests.rs:81-97`);本期不为它改写。

### D2. Windows 进程树收割:`taskkill /PID <pid> /T /F`,不用 Job Object

`kill_process_groups` 非 unix 分支改为对每个被记录的组 leader 执行 `taskkill /PID <pid> /T /F`(先于目录删除;"进程不存在"视为成功)。零新依赖、零 unsafe。

- 备选:Job Object(KILL_ON_JOB_CLOSE)→ 崩溃场景更稳,但要给 testsupport 引入 windows-sys FFI;作为 taskkill 实测有漏杀时的升级路径记录在案,本期不做。
- 备选:`wmic`/PowerShell `Stop-Process -Tree` → wmic 已弃用、PowerShell 慢且语义不齐,拒绝。
- `/proc` 孤儿清扫(tasks.py `_sweep_orphan_test_processes`)维持 Linux-only no-op:Windows 侧收割已在 testsupport 源头解决。

### D3. tasks.py 探活:平台分支 helper `_pid_alive(pid)`

POSIX 保持 `os.kill(pid, 0)`(POSIX 上 signal 0 是安全探测);Windows 用 ctypes `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `GetExitCodeProcess == STILL_ACTIVE`,打不开句柄视为已死。`signal.SIGKILL` 引用(:125、:711)收敛进 `_sigkill_if_supported()`:Windows 走 `Popen.terminate()`(硬杀,spec 已允诺 Windows 允许硬终止)。

- 备选:psutil → 环境完全未钉(pip 依赖都无),为一个探活引入第三方依赖,拒绝。
- 备选:`tasklist` 子进程探测 → 慢且输出随 locale 变化,拒绝。
- 顺带(小型):加 `pyproject.toml` 钉 `invoke` 依赖,给"跨平台可跑"一个可声明的 Python 侧安装面;不改任何入口行为。

### D4. 散件处理

- `sigterm_cleanup_test.rs`:补 `.exe` 感知的二进制解析(复用 `sebas-node` 解析的 `cfg!(windows)` 后缀模式),SIGTERM journey 整体维持 unix 门控。
- `state_subscription_test.rs`:`#![cfg(unix)]` 整文件门控维持原样(其主题依赖 unix 语义,无 Windows 移植诉求)。
- acceptance 套件:零结构改动,Windows 实跑只作验证证据(已有硬杀回退,:176-183)。

### D5. 双平台验收协议

- Windows:开发机本机 `invoke testsuite-e2e` + `invoke testsuite-acceptance` 全绿(附跳过清单)。
- Linux:WSL 内同两条命令全绿(接受首次 `cargo build` 的一次性成本)。沙箱全隔离(探测端口 + 全量 env 覆盖),不触碰操作员实例(9797 / 真实 `~/.sebas`)。
- WSL 工具链缺失时的降级:远程 Linux 盒手动跑,并在交付说明里如实记录证据变弱。

## Risks / Trade-offs

- [taskkill 快照与收割之间子进程再派生 → 漏杀] → 收割本就尽力而为;残留由下一次运行的陈旧沙箱清理兜底;实测漏杀再启用 D2 记录的 Job Object 升级路径。
- [CI 不跑这些套件 → 双平台绿可能随时间腐烂] → 已接受的权衡(用户拍板 CI 冻结);本 change 交付时留下双平台证据基线,后续解冻 CI 时可直接复制 matrix。
- [Windows 句柄延迟释放导致沙箱目录删除失败] → 沿用现有 3×200ms 重试 + 尽力而为语义(webui spec 同款允诺),不新增机制。
- [端口探测 TOCTOU(探测释放到子进程绑定之间被抢)] → 既有全平台共性风险,本期不处理,不在范围。
- [WSL 首次构建耗时] → 一次性成本,可接受;不引入共享 target 目录等优化(跨文件系统符号链接坑多)。

## Migration Plan

纯测试侧改动,无部署/迁移。回滚 = revert 对应提交。产品缺口(Windows 优雅退出)以 beads issue 承接,不阻塞本 change。

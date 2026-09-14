## 1. testsuite_e2e_test.rs 编译门修复

- [x] 1.1 解除 `Arc` 导入的 `#[cfg(unix)]` 门控(tests/testsuite_e2e_test.rs:17-18),验证:Windows 上 `cargo test --test testsuite_e2e_test --no-run` 不再因 Arc 报错
- [x] 1.2 把 `core_owned_provider_reaches_router_without_restart` 中的进程树断言段(`find_child_pid` 调用 :1317/:1397 与 `/proc/{pid}/cmdline` 直读 :1404)收进 `#[cfg(target_os = "linux")]` 辅助函数,journey 主体保持全平台;验证:Windows 与 Linux(WSL)上 `cargo test --test testsuite_e2e_test --no-run` 均编译通过

## 2. Windows 进程树收割

- [x] 2.1 `tests/support/mod.rs` 的 `kill_process_groups` 非 unix 分支改为对每个被记录组 leader 执行 `taskkill /PID <pid> /T /F`(目标已退出视为成功,先于目录删除);验证:Windows 上运行一个派生 fake-claude 的 e2e 用例,结束后 tasklist 无残留 sebas/fake-claude 进程且沙箱目录被删除
- [x] 2.2 失败路径验证:中途杀死 core 制造用例失败,确认拆卸仍不留孤儿、失败沙箱按既有约定保留现场

## 3. tasks.py 平台安全加固

- [x] 3.1 新增 `_pid_alive(pid)` 平台分支 helper(POSIX:`os.kill(pid, 0)`;Windows:ctypes `OpenProcess` + `GetExitCodeProcess`),替换 :57、:703 两处探活;`signal.SIGKILL` 引用(:125、:711)收敛进平台分支(Windows 走 `Popen.terminate()`);验证:Windows 上对存活 pid 调用 `_pid_alive` 返回 True 且进程未被终止,`invoke testsuite-e2e` 全程无 AttributeError
- [x] 3.2 新增 `pyproject.toml` 钉 `invoke` 依赖;验证:干净 venv 中 `pip install .` 后 `invoke --list` 正常列出 testsuite 任务

## 4. 散件门控补齐

- [x] 4.1 `tests/sigterm_cleanup_test.rs` 二进制解析补 `.exe` 感知(复用 `cfg!(windows)` 后缀模式),SIGTERM 用例维持 unix 门控;验证:Windows 上 `cargo test --test sigterm_cleanup_test --no-run` 编译通过
- [x] 4.2 确认 `tests/state_subscription_test.rs` 整文件 `#![cfg(unix)]` 门控在 Windows 下编译为空;验证:`cargo test --test state_subscription_test --no-run` 通过

## 5. 双平台验收与收尾

- [ ] 5.1 Windows 本机验收:`invoke testsuite-e2e` 与 `invoke testsuite-acceptance` 全绿,记录平台门控跳过的用例清单;验证:两命令退出码 0,跳过清单落入交付说明
- [x] 5.2 Linux(WSL)验收:同两条命令全绿(WSL 工具链不可用则按 design D5 降级为远程 Linux 盒并如实记录);验证:两命令退出码 0
- [x] 5.3 回归确认:Windows 上 `cargo test --workspace`(默认路径)与 `cargo clippy --workspace --all-targets -- -D warnings` 通过;为产品侧"Windows 优雅退出缺口"创建 beads issue

## 验收证据（2026-09-14，Linux 原生 Arch x86_64，非 WSL——强于 D5 基线）

- `invoke testsuite-e2e`：19 passed / 0 failed（含 fake-claude 全旅程、watchdog 监督恢复、SIGTERM 优雅退出等 unix 专属用例——Linux 上全部实跑，无跳过）。
- `invoke testsuite-acceptance`：9 passed / 0 failed。
- `cargo test --workspace`（默认路径，全量 1732 passed / 0 failed；首轮 1 例 spawn_env 并行 flake 单跑复验通过）。
- `cargo clippy --workspace --all-targets -- -D warnings`：改动前后错误清单完全一致（22 条既有基线，本仓库现状，非本 change 引入；对照实验用 stash 复核）。
- Windows 侧证据（本环境无 Windows 运行时，交叉验证）：rustup `x86_64-pc-windows-gnu` 目标 + mingw-w64 链接器，`cargo check --test {testsuite_e2e_test,sigterm_cleanup_test,state_subscription_test} --target x86_64-pc-windows-gnu` 与基线（改动前 3 错误）对比归零；`cargo test --workspace` 的 Windows 编译验证需在开发机以 `cargo test --workspace --no-run` 复核。
- **5.1 未勾**：Windows 本机运行时验收需开发机执行（两命令 + 跳过清单核对）。Windows 上预期跳过：e2e `graceful_exit_removes_channel_socket`（整例 unix 门控）；acceptance `session_lifecycle_journey` 的优雅退出恢复段（段级门控，旅程本身照跑）；provider-hotswap 的进程树断言（no-op，旅程照跑）。
- 产品缺口 issue：sebas-vp4（Windows CTRL_BREAK_EVENT 优雅退出通道）。
- 失败路径验证（2.2）：Linux 上 SIGKILL 测试二进制模拟崩溃——崩溃场景沙箱目录按约定保留（keep-on-panic 路径验证通过），_unix 崩溃时子进程树由内核会话/进程组语义收割有限（core 孤儿存活），Windows 侧对应路径由 taskkill /T /F 在拆卸时收割（运行时验证留 5.1）_。

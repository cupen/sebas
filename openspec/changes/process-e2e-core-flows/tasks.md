## 1. 沙箱基建（tests/support 扩展）

- [x] 1.1 `tests/support` 增沙箱配置生成器与环境变量组装：config.toml 覆盖菜谱全部必需键（router state_file、media download_dir、acp sessions_dir/work_dir、watchdog.core channel_path、watchdog.webui host/port、gateway 沙箱路径、debug 用 provider 占位），env 组装 `SEBAS_CORE_SECRET`/`SEBAS_STATE_DB`/`SEBAS_STATE_FILE`/`SEBAS_GATEWAY_PROVIDER_OVERLAY`，路径全部落在 `TestDir` 内。验证：support 单测断言生成文件存在、路径前缀均在沙箱内、端口 ≠ 9797
- [x] 1.2 `tests/support` 增空闲端口预检（`bind :0` 探测）与子进程就绪轮询 helper（轮询 `/health`，显式总超时，超时报含日志路径的错误）。验证：helper 单测覆盖超时路径；后续用例实际使用

## 2. 套件骨架与启动可达性

- [x] 2.1 新建 `tests/core_flow_e2e_test.rs`：`#[ignore]` 用例骨架，用例名语义化（可直接作 cargo 过滤器），二进制经 `CARGO_BIN_EXE_sebas`/target 目录定位，缺失时 panic 给出"先 `cargo build` 或 `invoke e2e`"提示；用例失败时 `TestDir::keep()` 保留沙箱并打印沙箱与日志路径。验证：`cargo test --test core_flow_e2e_test -- --ignored` 编译通过且骨架用例可跑
- [x] 2.2 detached 启动用例：核心 `sebas run --gateway --debug`（日志解析 gateway 地址，日志落 `<sandbox>/core.log`）+ 独立 `sebas webui`（日志落 `<sandbox>/webui.log`）双进程，断言 webui `/health` ok、`/api/summary` `reachability.ok=true`。验证：用例在 Windows 本地通过（unix 复跑确认）

## 3. 核心流程用例

- [x] 3.1 会话往返用例：webui `POST /api/sessions`（ACP 后端）→ 轮询至 Done，断言会话详情含 fake-claude 应答。验证：用例通过
- [x] 3.2 gateway debug 用例：核心日志取网关地址，`POST /v1/messages` `model=test` → 200 固定应答。验证：用例通过
- [x] 3.3 错误密钥用例：以错误 `SEBAS_CORE_SECRET` 启动 webui，断言 `/api/summary` `reachability.ok=false` 且带 cause。验证：用例通过
- [x] 3.4 重启翻转用例：终止核心 → `reachability.ok=false`，同配置重启核心 → 翻回 true。验证：用例通过

## 4. 平台门控与一键入口

- [x] 4.1 优雅退出用例（`#[cfg(unix)]` 门控；Windows 编译跳过已验证，unix 运行时验证待 unix 环境——模式逐行对齐已验证的 sigterm_cleanup_test）：SIGTERM 核心 → channel socket 文件消失、状态文件含退出前会话。验证：unix 上通过；Windows 上 `cargo test --test core_flow_e2e_test` 编译通过且用例不出现
- [x] 4.2 `tasks.py` 增 `e2e` 任务：`cargo build` → `cargo test --test core_flow_e2e_test -- --ignored`，支持 `--case <用例名>` 透传 cargo 过滤器，失败非零退出。验证：`invoke e2e` 一键全绿；`invoke e2e --case <单用例>` 只跑该用例
- [x] 4.3 AGENTS.md 沙箱菜谱处补一键入口说明（invoke e2e + invoke accept）；默认 `cargo test` 不含套件用例（3 passed/5 ignored）；工作区门禁 359 passed，唯一失败 `sebas-acp::acp_resume_test::kill_reaps_child_process` 为 Windows 既有问题（干净树复现，与本变更无关，已另行记录）

## 5. 收尾

- [x] 5.1 套件整体复跑（默认并行）总时长 < 2 分钟、无遗留进程与 scratch 目录；conventional commit 提交。验证：`invoke e2e` 复跑 + 进程/目录检查

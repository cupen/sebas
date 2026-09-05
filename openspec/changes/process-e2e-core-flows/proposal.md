## Why

核心流程的验证目前只有两档：进程内集成测试（`tests/*.rs` 拼装生产函数，跳过真实进程边界）和 AGENTS.md 的手动沙箱菜谱（手跑、无沉淀）。"watchdog 正式形态"——独立 core 进程 + 独立 `sebas webui` 进程经核心通道相连——没有任何自动化覆盖，协议/单元层全绿不代表这一形态能跑通。主干上 `state_store` 嵌套 runtime 启动崩溃已修复（8c1cde4），进程级 e2e 的拦路虎已不存在，正是把菜谱沉淀成用例的时机。

## What Changes

- 新增进程级 e2e 测试套件（`#[ignore]` opt-in，不拖慢默认 `cargo test`）：以真实二进制组成 detached 拓扑（`sebas run --gateway --debug` 核心 + 独立 `sebas webui`），全部路径沙箱化（TestDir + 专用端口 + 假凭据），绝不触碰操作员实例
- 用例覆盖核心流程：启动后 `/health` 与 `/api/summary` 可达性、会话往返（webui HTTP → 核心通道 → ACP fake-claude → Done）、gateway debug provider 应答、错误 `SEBAS_CORE_SECRET` 拒连、核心重启窗口 reachability 翻转、优雅退出清理通道 socket（unix 门控）
- 手动运行友好：用例名语义化可按名单跑（cargo 过滤器），失败保留沙箱目录与核心/webui 日志并打印路径
- `invoke e2e` 任务：一键"构建 + 跑套件"，单条命令校验核心流程是否正确；支持 `--case <用例名>` 过滤，方便手动只跑单个用例
- 沙箱目录/配置生成/端口分配等公共步骤沉入 `tests/support`

## Capabilities

### New Capabilities

- `process-e2e-suite`：进程级核心流程 e2e 校验——detached 拓扑的用例清单、沙箱隔离要求、一键入口与通过标准

### Modified Capabilities

（无）

## Impact

- 新增 `tests/core_flow_e2e_test.rs`、`tests/support` 扩展、`tasks.py` 加 `e2e` 任务；不改任何生产代码
- 依赖 `cargo build` 产物：`sebas` 与 `fake-claude` 均为根包 `[[bin]]`，经 `CARGO_BIN_EXE_*`/target 目录定位
- 协议与单元契约测试保持不动

## Non-goals

- `backend=native` 提示、审批面、模型方法等新协议契约的 e2e——属进行中的 `wire-webui-sebas-agent-e2e`
- 浏览器级 workbench UI e2e
- CI 流水线接入（套件先保证本地一键可跑，接 CI 另行立项）

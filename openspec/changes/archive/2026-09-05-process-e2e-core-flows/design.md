## Context

进程级 e2e 的原料都已存在：`fake-claude` 与 `sebas` 同为根包 `[[bin]]`（`cargo build` 一并产出，`CARGO_BIN_EXE_*`/target 目录可定位）、`tests/support` 有 `TestDir` 沙箱原语、AGENTS.md 沙箱菜谱验证过全套环境变量与配置覆盖清单、`sigterm_cleanup_test` 确立了 `#[ignore]` opt-in + 平台门控的惯例。曾挡住沙箱启动的 `state_store` 嵌套 runtime 崩溃已在主干修复（8c1cde4）。未覆盖的形态是 detached：核心 `sebas run` 与独立 `sebas webui` 进程经核心通道相连（watchdog 正式形态）；现有沙箱菜谱只手跑过 in-process（`run --webui`）。debug gateway 绑定随机端口，地址只能从核心日志行读取。

## Goals / Non-Goals

**Goals:**

- detached 拓扑下的核心流程全部有一键可跑的自动化用例（见 specs：可达性、会话往返、debug 网关、错误密钥、重启翻转、优雅退出）
- 套件对操作员实例零接触，且能与其他测试并行运行
- 沙箱搭建步骤沉淀为可复用 helper，为后续 native/approval 用例留扩展位

**Non-Goals:**

- 不测 `backend=native`、审批面、模型方法（`wire-webui-sebas-agent-e2e` 的契约，落地后可追加用例到本套件）
- 不做浏览器级 UI 校验、不接 CI 流水线
- 不改任何生产代码；发现生产缺陷只记录不顺手修

## Decisions

**D1 — 套件形态：根包集成测试文件 + `#[ignore]` opt-in**
单文件 `tests/core_flow_e2e_test.rs`，每个核心流程一个 `#[tokio::test]`（`#[ignore]`），复用 `tests/support`。备选：`scripts/` 独立脚本或 Python 测试——跨语言第二套基建，丢掉 TestDir/CARGO_BIN_EXE/Windows 门控等既有资产，否决。

**D2 — 拓扑：真 detached 双进程**
核心 `sebas run --gateway --debug`（不带 `--webui`），独立 `sebas webui` 子进程经 `[watchdog.core] channel_path` 连核心，HTTP 断言打在 webui 的 `[watchdog.webui] host/port` 上。备选：in-process `run --webui`——那不是被测的 watchdog 正式形态，且协议契约已有单测覆盖，否决。

**D3 — 一键入口：`invoke e2e` 任务（带 `--case` 过滤）**
`tasks.py` 增任务：`cargo build`（保证 sebas/fake-claude 产物在）→ `cargo test --test core_flow_e2e_test -- --ignored`；`--case <用例名>` 透传为 cargo 过滤器，单个用例手动调试不必跑全套（用例名语义化，cargo 过滤命令本身也始终可用）。备选：用例不加 `#[ignore]` 进默认 `cargo test`——进程级用例秒级起步会拖慢日常门禁，且违背 sigterm_cleanup 确立的惯例，否决。

**D4 — 平台策略：SIGTERM 用例 unix 门控，生命周期语义由跨平台用例兜底**
优雅退出清理 `#[cfg(unix)]`（沿用先例）；Windows 上核心生命周期由"强杀核心 → reachability 翻转 → 重启恢复"用例覆盖。备选：Windows 用 `GenerateConsoleCtrlEvent` 模拟优雅退出——需新增 winapi 依赖且依赖共享控制台细节，价值不抵复杂度，第一版不做。

**D5 — 沙箱基建：helper 沉入 `tests/support`**
新增"沙箱配置生成器"（写入菜谱要求的全部配置键 + 全量环境变量覆盖）、空闲端口预检（`bind :0` 探测后写入配置，接受探测与真实绑定间的 TOCTOU——本地并行场景冲突概率低且失败可诊断）、子进程就绪轮询 helper（轮询 `/health`/reachability，带总超时）。`TestDir` 本身不动。手动排查约定：核心与 webui 日志固定落 `<sandbox>/core.log`、`<sandbox>/webui.log`；用例失败时对沙箱调用 `TestDir::keep()` 保留现场并把沙箱与日志路径打到测试输出（保留目录在 `target/tests/` 下，`cargo clean` 兜底）。

**D6 — 断言面：HTTP + 文件系统，不碰内部 API**
只经 `/health`、`/api/summary`、`/api/sessions`、网关 `/v1/messages` 与文件系统（channel socket、state file）断言；gateway 地址从核心日志行解析。会话轮询超时 ~15s（fake-claude 回合秒级完成）。Windows curl 的 GBK 缺陷不适用——用例内是 Rust HTTP 客户端直发 UTF-8。

## Risks / Trade-offs

- [并行用例抢端口/目录] → 每用例独立 TestDir + 端口预检；用例间不共享任何沙箱状态
- [二进制缺失（跳过 invoke 直接 cargo test）] → 用例开头断言产物存在，缺失时 panic 给出"先 cargo build / invoke e2e"提示（沿用 full_e2e 惯例）
- [双进程启动竞速] → 就绪轮询 helper 统一等待，不写裸 sleep（唯一的必要 sleep 沿用 sigterm_cleanup 的预算注释风格）
- [套件耗时增长] → 全套预算 ~2 分钟内；单用例显式超时，挂起时留日志路径可诊断

## Migration Plan

纯新增（一个测试文件 + support helper + 一个 invoke 任务），无生产代码与数据迁移；回滚 = 删除新增文件。

## Open Questions

（无——实现层面的日志行格式、端点字段名以沙箱实测为准，不影响规格与任务拆分。）

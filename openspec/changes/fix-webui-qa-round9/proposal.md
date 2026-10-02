## Why

第九轮 GUI 全量验收（42 测试点，证据在 `gui-test-screenshots/qa-round9/`）与源码复核确认两项实质缺口：会话面（转录条目、泊车审批、会话级 usage）为纯内存态，进程重启即全部丢失（QA 以硬重启实测 23 个会话转录清零、待批审批消失）；审批应答在前端无超时呈现，QA 窗口内 4 次拒绝点击静默无效（当前构建经 API/GUI/网络三层取证已复绿、既有并行拒绝套件在位，但「应答悬挂」仍不可见）。另有一批入口与打磨缺口。

## What Changes

- 会话面持久化（新能力）：转录条目、泊车审批、会话级 usage 周期性 checkpoint 落 SQLite 状态库，core 启动回放——崩溃/强杀不再丢对话史，重启后待批审批卡恢复可决定
- 审批应答加固：前端卡片 `answering` 态加超时可见化（失败标记、可重试）；core 侧审批应答落结构化日志（request_id + 决定 + 会话）
- 项目重命名：补 `POST /api/projects/{id}/rename` 端点与 rail 菜单入口，落库持久
- 工作台打磨：`/router` 退役路径补重定向；停滞看门狗通知文案改为「静默超过 15 秒即判定停滞，最迟约 30 秒内强制收尾」；回合帧显示模型名；设置弹窗补 `role="dialog"`/`aria-modal`；会话重命名弹窗 Enter 提交；重启后恢复会话树展开态与焦点会话
- 文档对齐（无 spec delta）：`AGENTS.md`「graceful exit dumps state」过时表述、`CreateSessionRequest` 0-turn 注释与「聚焦即拉起」实际行为的口径

## Capabilities

### New Capabilities

- `session-transcript-durability`: 会话面（转录/泊车审批/会话级 usage）的周期 checkpoint 与启动回放契约

### Modified Capabilities

- `permission-flow`: 审批应答超时可见与应答留痕（卡片 answering 超时呈现 + core 应答日志）
- `project-session-actions`: 项目重命名（端点 + rail 入口 + 持久）
- `agent-workbench`: /router 重定向、停滞文案、回合帧模型名、设置弹窗 a11y、重命名 Enter、重启后工作台状态恢复
- `session-persistence`: 退役「graceful-shutdown dump session map」残留条款，与 per-mutation 落库现状对齐

## Impact

- `sebas-dispatch`（turn_log/泊车登记/usage 的 checkpoint 写入与启动回放）、根 crate `sebas_state`（projects.db 新表注册，遵守 persistence-runtime 准入：DDL 只在注册表、struct 挂 derive、经 StateWriter 写入）
- `sebas-webui/src`（projects rename 端点；审批应答日志）、`sebas-webui/frontend`（review-card 超时态、project-rail 重命名、router 重定向、设置弹窗 aria、重命名弹窗、工作台状态恢复）
- `tests/`（进程级 e2e 断言重启回放）
- `AGENTS.md`、`sebas-webui/src/api.rs` 注释
- 无 wire 形状破坏性变更；新端点为增量

## Non-goals

- 看门狗 1Hz `set_permission_mode` 探针协议改动（设计内核活探测，D3 复核定为设计内）
- 0-turn 创建「聚焦即拉起」行为变更（spec 化能力，仅修注释口径）
- 未读徽标「无锚 = 已读」语义变更（spec 明文 + 既有浏览器回归在位，QA 观察为新 context 测试假象）
- 转录逐条写穿（写放大不可取，取周期 checkpoint）
- 优雅退出 dump 的恢复（已退休路径不复活）
- 触屏适配与全面 i18n（另有 webui-i18n-sweep 承载）

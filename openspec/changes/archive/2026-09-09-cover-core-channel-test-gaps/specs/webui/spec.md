## MODIFIED Requirements

### Requirement: Session backend seam
The WebUI crate SHALL access sessions through a backend abstraction rather than a concrete `DispatchHandle`, in the same shape as the existing admin adapter, so the crate carries no knowledge of whether the core is in-process or across a socket. The crate SHALL NOT depend on the sebas binary crate to obtain a backend; the binary crate SHALL supply the implementation at startup. **补充**：`SessionBackend` 实现 SHALL 在 `reachability()` 中区分 startup failure / auth rejected / runtime disconnect 三类不可达，并在 `/api/summary` 输出中通过 `reachability.kind` 字段显式区分；webui degradation banner SHALL 根据 kind 渲染不同文案（startup failure banner 含 startup-failure cause；runtime disconnect banner 不含）。**补充**：approval_answer 与 set_session_model SHALL 在 webui 端到端走 fake-claude 沙箱验证（happy-path + typed-rejection 路径）；approval_answer 流程 SHALL 经真实审批通道（acp gated tool call → channel ApprovalRequested 帧 → webui review-card → POST `/api/permissions/{rid}/answer` → channel ApprovalAnswer → acp 子进程以 allow/deny 语义继续）。

#### Scenario: WebUI is testable without a core

- **WHEN** the WebUI's route tests run
- **THEN** they drive routes through a fake backend, with no ACP child, no socket,
  and no state file

#### Scenario: no backend leaks into templates

- **WHEN** a page is rendered
- **THEN** which backend is in use is not visible in the markup except where the
  channel's degradation contract requires stating that the core is not connected

#### Scenario: startup-failure banner via fake-claude sandbox

- **WHEN** 沙箱 backend core 启动时配置错误退出 75（`SEBAS_STARTUP_ERROR_FILE=<tmp>` 写入 `startup-failure: bad config`）、webui 仍连接该 socket
- **THEN** webui banner SHALL 显示 `core startup failed: bad config` 全串（cause 即该全串，与已落地的 enrich 行为一致）；`GET /api/summary.reachability.kind` SHALL 为 `startup_failed`

#### Scenario: runtime disconnect banner distinguishes from startup failure

- **WHEN** 沙箱 backend core 启动后正常运行；测试期间强杀 core 进程；webui 探测
- **THEN** banner 显示 `core is not connected`（不含 startup failure 字样）；`/api/summary.reachability.kind` SHALL 为 `disconnected`

#### Scenario: approval_answer end-to-end (allow path)

- **WHEN** fake-claude 触发 gated tool call（触发词 "perm"）、core 推到 channel ApprovalRequested 帧、webui 渲染 review-card、操作员点 allow
- **THEN** webui POST `/api/permissions/{rid}/answer` with allow；core 转发 ApprovalAnswer 到 acp 子进程；子进程以 allowed 工具结果呈现；transcript 完成（acp 回合 Done）

#### Scenario: approval_answer end-to-end (deny path)

- **WHEN** fake-claude 触发 gated tool call、操作员点 deny
- **THEN** webui POST `/api/permissions/{rid}/answer` with deny；core 转发 ApprovalAnswer 到 acp 子进程；子进程以 denied 工具结果呈现；回合 Done

#### Scenario: approval_answer end-to-end (detached topology)

- **WHEN** 同一流程跑在双进程沙箱（独立 core + 独立 webui，harden 5.4 的可复用 harness）：fake-claude 触发 gated tool call → channel ApprovalRequested 帧跨进程到达 webui → review-card → answer → ApprovalAnswer 回 core → acp 子进程继续
- **THEN** allow / deny 各一条旅程全绿；单进程形态的同名用例保持全绿（两种拓扑不互相代替）。本 scenario 阻塞于 `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）+ harden 5.4 harness 落地，见 tasks B 批

#### Scenario: approval_answer rejects unknown request_id

- **WHEN** webui POST `/api/permissions/{rid}/answer` with `rid` 不存在
- **THEN** 后端回 4xx typed rejection（`UnknownRequestId` 或同类）；前端不渲染成功状态

#### Scenario: set_session_model happy-path via webui

- **WHEN** 沙箱 fake-claude 启动带 `configOptions.model` 多 model、用户 PUT `/api/sessions/{key}/model` with `{model_id: "<另一 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `ModelChanged` 事件；webui snapshot 同步 `current_model`；前端 UI 显示新 model selected

#### Scenario: set_session_model rejects unknown model

- **WHEN** 用户 PUT `/api/sessions/{key}/model` with `{model_id: "<不存在的 model>"}`
- **THEN** 后端走 channel SetSessionModel；acp 子进程回 `Error`（typed rejection）；webui 端呈现内联错误；session state 不变

#### Scenario: cross_uid rejection covers live process

- **WHEN** live fork + setuid 到不同账户进程尝试连接 core socket 并发 Snapshot 请求
- **THEN** 连接被拒；服务端日志写 peer-uid mismatch；不进入 Snapshot 处理路径

#### Scenario: backend push renders faithfully

- **WHEN** backend 推送 spawn-failure 事件
- **THEN** 前端立即渲染该事件为 transcript 内显错误，不等待 Removed 事件；会话状态 SHALL 在 backend 推送 Removed 之前已经标记为 spawn-failed
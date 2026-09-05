# 任务：对话区 composer 会话化 + agent 只读标签

## 1. wire 层带出 agent_kind

- [x] 1.1 `SessionInfo` 增加 `agent_kind: Option<String>`（`#[serde(default)]`），`session_info_for` 从 mapping 的 `pending_kind` 填充；跑 `cargo test -p sebas-router` 确认 events/router 既有测试不回归。
- [x] 1.2 `MappingDto` 加 `#[serde(default)] pending_kind` 落盘（若 state-store sessions 表已接管 mapping 持久化则同步加 nullable 列）；补一条"落盘→重载后 pending_kind 保留"的持久化往返测试。
- [x] 1.3 `sebas-webui`：`SessionRow` / 会话 detail / summary 的 active_session 序列化带出 `agent_kind`（`models.rs` + `routes.rs` + `api.rs`）；`cargo build` 后 curl `/api/sessions` 断言新字段存在。（沙箱 curl 验证：row/detail/summary 三路均带出 `agent_kind=claude`，默认 kind 行为 `null`。）
- [x] 1.4 （4.2 联调发现的补丁）core session channel wire 补 `kind`：`Spawn`/`CreatePlaceholder` 帧加 `#[serde(default)] kind`，client 解析 backend hint 上送、server 透传给 router（此前 standalone webui 建会话永远钉默认 kind，创建模式的 agent 选择被静默丢弃）；更新 `create_placeholder_wires_a_zero_turn_session` 到新契约。

## 2. composer 双模式重构

- [x] 2.1 重写 `workbench-composer.ts`：新增 `mode`（follow-up/create）状态与 `sessionKey`/`agentKind`/`modelOptions`/`currentModel` properties；跟随模式 `submit()` 改走 `api.sendMessage(sessionKey, text)`；保留 28px 发送、Enter 语义与 IME 防抖；组件测试覆盖"跟随发送不建新会话/创建发送走 createSession"。
- [x] 2.2 底部工具栏按 D5 实现：跟随模式 = agent 名只读小字（0.78rem mono dim）+ 模型下拉（聚焦会话数据源）；创建模式 = agent 下拉（含 `native`、过滤不可达，沿用 agent-driver 约束）+ 模型下拉 + `→ 项目/inbox`；删除顶部 agent 下拉与重复的第二个 backend 下拉；组件测试断言两种模式的可见控件。
- [x] 2.3 "新会话" chips：聚焦时显示，点击切入创建模式（再点取消返回跟随模式）；创建成功沿用现有 `composer-created` 事件跳转；测试覆盖模式往返与聚焦会话 transcript 不受影响。
- [x] 2.4 `dashboard.ts` 把聚焦会话的 `agent_kind`/`available_models`/`current_model` 传给 composer；detail 在途时显示 `· · ·` 占位；联调验证 WS 推送后标签随 refetch 刷新。

## 3. 会话详情页顺带展示

- [x] 3.1 `session-detail.ts` 头部 meta 行加 agent 名只读文本（同一 `agent_kind` 数据源，None → "acp（默认）"）；`api/client.ts` 的 `SessionRow`/`SessionDetail` 类型补 `agent_kind`；组件测试断言渲染。

## 4. 收尾验证

- [x] 4.1 前端单测全绿（`npm test`：composer 17/17，全量 103 通过；3 个失败为既有环境性问题——app-shell 路径拼接 `D:\D:\` 与 a11y CSS 断言，与本 change 无关）；`cargo test -p sebas-router`（88）与 `-p sebas`（216）绿，`-p sebas-webui` 60 绿 + 4 个既有 Windows 环境性失败（fs `/usr`、`sh` PATH）。
- [x] 4.2 沙箱联调（temp 目录 config，端口 9889 ≠ 9797）：`acp:claude` 0-turn 占位 → row/detail/summary 均带 `agent_kind=claude`；默认 kind 行为 null；首条消息触发 spawn（stub 子进程诚实死亡——无凭据环境预期，spawn 失败按 Removed 清理）；SPA 以新 dist 资产服务；webui↔core reachability ok。重启持久化场景由 1.2 的往返单测覆盖（沙箱子进程无法存活到 dormant）。清理：进程已停、socket/目录已删、端口无 LISTEN。

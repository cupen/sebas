## 1. 后端：focus 语义与 ACP 创建时模型下发

- [ ] 1.1 `src/agent_backend.rs`：ACP `spawn_with` 在首条 prompt 注入前，若 `model` 非空且 agent configOptions 暴露模型面，则先下发 `session/set_config_option`；agent typed rejection 时以 `SessionRejection` 上抛且会话不静默回退默认模型。验证：`cargo test -p sebas --lib agent_backend` 新增用例通过（fake ACP 带模型面：ok-model 生效 / bad-model typed rejection；无模型面 agent 跳过下发）
- [ ] 1.2 `src/agent_backend.rs`：ACP `create_placeholder` 把 `model` 记入 mapping，首条消息触发真实 spawn 时走与 1.1 相同的「prompt 前下发」路径。验证：0-turn 占位 + 带 model 首条消息的单元用例通过（断言 set_config_option 先于 prompt 到达 fake agent）
- [ ] 1.3 `sebas-webui/src/api.rs`：`switch_session` / 深链页聚焦的语义注释与文档补齐（行为不变，仅显式化），确认 `create_session` 对占位的 `set_focus` 已有覆盖测试。验证：`cargo test -p sebas-webui` 相关用例全绿
- [ ] 1.4 关闭聚焦会话时 `close_session` 响应的 `active_session_key` 为 null 的路径补一条 webui 路由测试。验证：新测试通过

## 2. 前端：rail 删除入口

- [ ] 2.1 `views/project-rail.ts`：项目行加 hover-reveal 的 remove 按钮 + `wa-dialog` 确认弹窗（文案含项目名与「存活会话迁入 Inbox」说明），确认后调 `api.projects.remove`，行消失无刷新；拒绝时错误内联在弹窗、行保留；取消不发请求。验证：`project-rail.test.ts` 新增用例通过（确认/取消/拒绝三路径），`pnpm --dir sebas-webui/frontend test` 全绿
- [ ] 2.2 `views/project-rail.ts`：会话行在归档按钮旁加 close 按钮，调 `api.closeSession`；inactive（dormant/done/failed）立即执行，active（starting/queued/working）先内联确认；关闭聚焦会话后工作台回到无聚焦空态（经 summary 刷新驱动）。验证：`project-rail.test.ts` 新增用例通过（三状态分级确认 + 聚焦关闭后 active_session_key 清空）
- [ ] 2.3 rail 按钮组的键盘可达性与 aria-label 检查（与既有 row-action 模式一致）。验证：`a11y.test.ts`（如有 rail 覆盖）或手工 sandbox 检查通过

## 3. 前端：占位会话立即可写（focus 同步）

- [ ] 3.1 `views/project-rail.ts`：`+` 创建占位会话后先 `await api.switchSession(key)` 再 `navigate`，保证 focus 指针与 URL 同步到位。验证：单测断言 switch 先于 navigate 调用
- [ ] 3.2 `views/session-detail.ts`：深链页加载时补一次幂等 `switchSession` 调用（覆盖书签/外链直达路径）。验证：`session-detail` 相关单测更新并通过
- [ ] 3.3 `views/workbench-composer.test.ts`：补「聚焦会话存在 → composer 跟随模式 → submit 走 sendMessage 而非 createSession」的回归用例（锁定 3.1/3.2 的端到端语义）。验证：新用例通过

## 4. 前端：创建模式模型选择（ACP 生效）

- [ ] 4.1 `views/workbench-composer.ts`：创建模式提交时把选中的 `model` 随 `api.createSession` 传递（既有参数，确认不再被丢弃），并在 4.x 后端语义落地后验证 ACP 会话首回合 `current_model` 即所选项。验证：`workbench-composer.test.ts` 断言 createSession 收到 model 参数
- [ ] 4.2 创建时模型被 agent typed rejection 的呈现：composer 内联错误 + 不伪造成功态。验证：单测模拟 ApiError 呈现内联错误

## 5. e2e 旅程（testsuite-webui，fake-claude / fakeacp 沙箱）

- [ ] 5.1 `tests/projects.spec.ts`：rail 项目删除旅程（添加 → rail 删除 → 行消失；带存活会话删除 → 会话迁入 Inbox）。验证：`invoke testsuite-webui-server` + `pnpm playwright test projects.spec.ts` 通过
- [ ] 5.2 `tests/session-mgmt.spec.ts`：rail 会话删除旅程（dormant 直删 / working 需确认 / 删聚焦会话回空态）。验证：playwright 用例通过
- [ ] 5.3 `tests/session-roundtrip.spec.ts`：占位会话首条消息旅程——rail「+」→ composer 直接输入 → 消息到达 fake-claude → 无第二会话产生。验证：playwright 用例通过（断言 sessions 总数不变 + 占位会话出现 turn）
- [ ] 5.4 `tests/models.spec.ts`：创建模式带模型创建 ACP 会话（fakeacp 的 ok-model）→ 首回合 `current_model` 为所选；bad-model → typed rejection 内联呈现。验证：playwright 用例通过

## 6. 收尾验证

- [ ] 6.1 全量门禁：`cargo test` + `pnpm --dir sebas-webui/frontend test` + `invoke testsuite-webui`（或等价 playwright 全量）全绿；`cargo clippy` 无新增告警
- [ ] 6.2 沙箱人工巡检（`invoke testsuite-webui-sandbox`）：rail 删除、占位会话发消息、创建带模型三条旅程手工过一遍，截图/记录结果

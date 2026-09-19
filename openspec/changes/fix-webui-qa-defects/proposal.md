## Why

对 sebas WebUI 做的全链路黑盒 GUI 验收（fake-claude 桩驱动的项目管理 / 会话创建 / 消息收发 /
审批 / 模式切换 / 归档恢复 / Skills / Settings 旅程，证据见沙箱截图与 core 日志）发现了一批缺陷：
1 个数据丢失级（归档恢复即丢失转写）、2 个高严重度（占位会话幽灵回合、rail 切换主区不跟随）、
以及若干渲染 / 配置保真 / UX 缺口。其中占位幽灵回合会让**每一个**新建会话在
`turn_stall_timeout` 后凭空出现一条错误——默认配置（600s）下同样命中，只是更晚。

## What Changes

- **归档恢复不再丢数据**：`POST /api/sessions/{key}/restore` 从「从归档删除条目并返回 JSON」
  改为「删除条目 **并重建可写的会话行**」——恢复后会话在 rail / `/api/sessions` 立即可见，
  转写完整保留（现状：三处皆空，数据不可逆丢失）。
- **占位会话不再武装停滞看门狗**：0-turn 占位（`create_placeholder`）不进入「回合在飞」
  相位，停滞看门狗不再对其强收并向 transcript 写入「回合停滞被强制收尾」合成错误。
- **rail 切换会话立即聚焦工作台**：`POST /switch` 成功后前端立即更新工作台焦点
  （不再依赖下一个无关会话事件触发 summary 刷新才「跳」过来）。
- **引擎错误条目如实标注**：错误气泡标签不再一律写死「spawn failed」——按失败原因标注
  （如「回合停滞」「spawn failed」）。
- **refusal / 错误结果必须渲染**：agent 回合以 `result{is_error}`（含 refusal）收尾时，
  transcript 必须出现对应的错误条目，不允许用户消息石沉大海。
- **claude agent 配置 `args` 的 argv 保真**：位置参数不再被静默丢弃——配置解析期直接报错
  并提示键值形式（`--flag value`），杜绝「配置写了、子进程没收到」。
- **UX 打磨**：agent 下拉 display 名缺省回退 agent id（不再三项同名 "Claude Code"）；
  越界路径手填注册时给出禁用原因；归档恢复确认弹窗明确「将重建会话」；会话终止通知
  使用可读会话名而非原始 key。
- **文档修正**：AGENTS.md 沙箱菜谱中已失效的 `[acp.claude]` 配置键更新为现行
  `[acp] default` + `[acp.agents.*]`（driver 标签）形态。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `project-session-actions`: 「Session archive」需求补充恢复语义——恢复必须重建会话行并
  保留全部转写，归档条目消费与会话重建必须同事务语义（不允许删了归档却没重建会话）。
- `session-lifecycle`: 「Lazy spawn on first message」需求补充占位相位约束——占位不构成
  在飞回合，停滞看门狗不得对占位触发，也不得向占位 transcript 注入合成错误。
- `agent-workbench`: 「Workbench is the single conversation surface」补充 rail 切换的
  焦点即时性场景；「Workbench renders the focused session as a conversation」补充
  回合错误结果（refusal / is_error）的渲染场景。
- `acp-driver`: 新增「Agent argv fidelity」需求——配置 `args` 必须无损到达子进程 argv，
  无法表达的位置参数必须在配置解析期显式拒绝。

## Impact

- 后端：`sebas-webui/src/archive.rs` / `api.rs`（restore）、`sebas-dispatch/src/engine/`
  （占位相位与停滞看门狗交互）、`src/config.rs`（args 解析校验）、`sebas-acp/src/claude/driver.rs`。
- 前端：`sebas-webui/frontend/src/views/dashboard.ts`（焦点跟随）、`project-rail.ts`、
  `transcript-view.ts`（错误条目渲染与标签）、`new-session-dialog.ts`（display 回退）、
  `project-rail.ts`（越界提示）、`notice`（可读会话名）。
- 既有测试面：`tests/testsuite-webui/tests/`（回归用例落点）；与 in-progress change
  `session-parallel-liveness-and-unread-polish`（17/18）在同文件域有重叠，实施前需先
  落地或对齐该 change 的 dashboard/WS 改动。

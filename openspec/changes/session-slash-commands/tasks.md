# Tasks: session-slash-commands

## 1. sebas-acp：命令发现事件

- [x] 1.1 `sebas-acp/src/session.rs`：`AcpEvent` 新增 `AvailableCommands { session_id, commands: Vec<AvailableCommand> }` 变体与 `AvailableCommand { name, description, hint: Option<String> }` 类型（`#[serde(default)]` 兼容旧反序列化）；验证 `cargo check` 通过
- [x] 1.2 claude intake：`sebas-acp/src/claude/driver.rs` 会话建立后调 `cc-agent-sdk` `get_server_info()`，防御性映射 `commands` 数组 → 发 `AvailableCommands`；映射函数单测覆盖字段缺失/空数组/正常三种 fixture；验证 `rtk cargo test -p sebas-acp claude`
- [x] 1.3 通用 ACP intake：`sebas-acp/src/acp_driver/codec.rs` 给 `SessionUpdate::AvailableCommandsUpdate` 加 match arm → 发同事件；`fake-acp-agent` 测试桩支持场景化发送 `available_commands_update`；验证集成测试断言事件到达
- [x] 1.4 重新广告刷新：claude `commands_changed`（若 SDK 可得）或通用 ACP 二次通知 → 快照再刷新；至少为通用路径补「二次通知覆盖旧表」用例；验证 `rtk cargo test -p sebas-acp`

## 2. 引擎物化与快照透出

- [x] 2.1 `sebas-dispatch` 引擎 `apply_event`：消费 `AvailableCommands` 写入会话状态；`SessionInfo` 增 `available_commands` 可选字段（serde default + skip_serializing_if，旧 JSON 兼容）；验证引擎单测（事件 → 快照字段）
- [x] 2.2 webui 会话载荷/WS 透出：`sebas-webui/src/session_backend.rs` 快照链路带新字段；旧 core + 新前端、新 core + 旧前端两种组合的字段缺省行为各一用例；验证 `rtk cargo test`
- [x] 2.3 native 路径确认：`AgentEvent` 词汇不动、native 会话快照命令表恒空；验证：native 会话 e2e 或单测断言字段缺省

## 3. composer 命令面板

- [x] 3.1 `workbench-composer.ts`：读会话快照 `available_commands`；输入首字符 `/` 渲染浮层面板（命令名 + 参数提示 + 说明，复用 model 菜单 listbox/aria/键盘模式）；↑↓、Esc、Tab/Enter 两段式补全（补全插入 `name + 空格`、焦点留输入框、面板关闭、再 Enter 提交）；验证 vitest 组件用例
- [x] 3.2 增量过滤：按前缀不区分大小写实时收窄；无匹配 → 空态不渲染；非首字符 `/` 不触发；验证 vitest 过滤用例
- [x] 3.3 透传：提交不做任何改写，走既有 `api.sendMessage`（busy/queued 同路径）；验证既有提交用例不回归 + `/` 前缀提交断言

## 4. 拦截与退化

- [x] 4.1 拦截规则：会话有命令表面时，命令名 ∉（广告表 ∪ {compact}）→ 内联提示「该会话的 agent 不支持此命令：/xxx」并阻止发送；广告表为空（native 等）→ 不拦截；验证 vitest 三分支用例（opencode 手输 `/goal` 拦截、`/compact` 放行、native 放行）
- [x] 4.2 诚实退化 UI：空命令表会话敲 `/` 无面板、按普通文本提交；验证 vitest

## 5. 端到端验收

- [x] 5.1 后端全量回归：`rtk cargo test`（含 sebas-acp / sebas-dispatch / testsuite）；验证：全绿
- [x] 5.2 沙箱 e2e（AGENTS.md 配方 + fake-claude stub）：claude 会话面板列出 stub 广告命令、`/goal`/`/compact` 提交到达 stub（journal 断言原文）、opencode 配置会话手输未广告命令被拦截；验证：`invoke testsuite-e2e` 或沙箱目验
- [x] 5.3 前端全量回归 + 浏览器冒烟：`rtk vitest` 全绿；`invoke testsuite-webui-sandbox` 目验面板交互与无面板退化态；验证：操作员目验通过

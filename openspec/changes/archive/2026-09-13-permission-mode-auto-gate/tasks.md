# Tasks: permission-mode-auto-gate

## 1. Hook 门控（sebas-acp）

- [x] 1.1 `sebas-acp/src/claude/driver.rs`：PreToolUse hook 处理最前置查 `permission_mode` 共享单元，bypass 档（allow/auto）直接返回 allow 输出——不产生 PermissionRequest、不泊车、不发卡；非 bypass 档维持现状流；单测覆盖 bypass 零请求、ask/edit 照常弹、SetMode 后下一次咨询即时生效（无 respawn）

## 2. 点击语义重定义（sebas-dispatch）

- [x] 2.1 `sebas-dispatch`：`AllowSession` 点击处理改为 approval allow（不回退）+ 向执行体发 `SetMode { mode: "auto" }`（与 webui 中程切换同源路径）+ mapping `desired_mode=auto`；mode 切换失败时放行不回滚、失败如实上报卡片事件；删除 `maps.rs` `grant_all` 与 allowlist 存储、`inbound.rs` 旧挂接；单测：新点击语义三态（切成功/切失败放行仍在/ask 会话点击后不再弹卡）、allowlist 路径不复存在

## 3. 飞书卡面前端（sebas-im）

- [x] 3.1 `sebas-im`：`Allow session` 点击后翻面「✅ 已切换自动模式」（audit trail）；mode 切换失败翻面呈现如实失败状态；按钮文案维持「本会话不再询问」；单测覆盖翻面两态

## 4. resume 与生命周期

- [x] 4.1 确认（补齐如有缺口）resume 路径把 mapping `desired_mode` 翻译进 spawn `--permission-mode` argv；`/new` 后新会话无 mode；单测：带 desired_mode 的 resume 首个工具调用 hook 静默
  - 核实结论：翻译链已存在——根 crate `src/dispatch.rs` `handle_spawn_resume_without_feishu` 读映射 `desired_mode` 后经 `resume_command_with_mode`（本次从内联块提取，补单测）拼 `--permission-mode` argv → `acp_resume_and_activate` → driver `connect` 解析回 mode 共享单元；`/new` 回默认档在 engine `inbound.rs::spawn_new`（重建映射，他人地盘，已就位未动）。

## 5. 收尾

- [ ] 5.1 `cargo test -p sebas-acp -p sebas-dispatch -p sebas-im` 全绿、全工作区 build；`openspec validate permission-mode-auto-gate --strict` 通过
- [x] 5.2 沙箱端到端：fake-claude 会话点「本会话不再询问」→ 当前请求放行 + 第二个工具调用零弹卡（hook 门控）+ 卡面已切换自动模式；webui 侧确认 effective_mode=auto 可见

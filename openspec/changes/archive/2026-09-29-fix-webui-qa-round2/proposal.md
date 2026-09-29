## Why

2026-09-29 第二轮 WebGUI 黑盒验收（四簇共 40 点：管理面 A、会话与流式 B1、消息形态 B2、权限模式 C，报告见本 change `evidence/`）确认核心链路整体健康——发消息→回复、流式增量、thinking、空/错误/慢回合、崩溃 UI 终态、未读徽标回归修复、自动标题、模式切换契约、审批决策持久化全部 PASS，零意外 console 错误。但仍发现 4 个 P2 缺陷与约 10 个 P3 缺陷/缺失/打磨项。延续 fix-webui-qa-findings 先例全部立项，避免验收结论只留报告不落行为。

## What Changes

缺陷（QA 编号保留溯源，均为本轮新增）：
- **D-B11（P2）**：Add-project 路径输入框静默吞掉所有反斜杠（真实键入与 fill 均复现），每次键击触发 HTTP 400 并报误导性「路径不存在」；目录树选择器不受影响——输入归一化 + 校验时机 + 错误文案修正。
- **D-B215（P2）**：agent 进程崩溃后 UI 1.6s 即出明确终态（上轮 600s 僵尸已修），但后端回合仍滞留 running ≤600s，期间重聚焦会话呈现阻塞发送的停止态——崩溃后端回合 SHALL 快速终态化。
- **D-C3（P2）**：审批决策后 tool_result 内容不可达（点嵌套折叠行会折叠父级）；单工具审批回合丢失环后正文。
- **D-C5（P2）**：并行审批一次只渲染一张卡、第二请求无入口；决策后被允许工具的转写条目整体消失。

缺失/可用性（P3）：
- **M-C6**：Allow 与 Auto 行为等价在 GUI 未被如实呈现——现行 `permission-flow` spec 本就定义两档同映射 bypass tier（非语义缺陷），要求模式描述文案如实说明等价性，不捏造差异。
- **M-A4**：New-agent 表单缺 sessions_dir / work_dir / args 字段，GUI 建的 agent 无法配置到可用形态。
- **未读分界线语义统一**（D-B12 + M-B216）：成功回合非聚焦完成有徽标但重聚焦无分界线；分界线又在注视场景出现并驻留——呈现与清除口径统一到 `session-unread-badge` spec。
- **D-B218**：`/compact` 仅 toast 无转写回执、输出并入前段 assistant 段落、Enter 与 Send 按钮对 slash 文本分派不一致。
- **D-B13**：flood（1200 chunks）摄入期间 ~1.2s 主线程卡顿。
- **D-A4 / D-A7 / D-A2 / D-A10**：agent 表单 display name 不回填；About Rust toolchain 行缺「要求 ≥」界限；Add-project 弹窗 `or` 分隔线贴边；Env Vars 表格列挤压。
- **打磨组**：last active 计时器冻结、toast 重叠、perm 场景重名提示、默认模型 chip 显示 Fake。

随带校正与 3c 复审新发现：
- **D-R2A（P2）**：聚焦写锚竞态——第二回合完成后立即聚焦，读锚停在上一回合段数（unread-badge.spec 两条确定性红；服务端计数经活体复现证实正确，竞速在客户端写锚侧）。
- **D-R2B（P3）**：Settings→Models 的 config provider 行确定性不可见（settings.spec S5a 红）。
- fix-webui-qa-findings tasks.md 3.2（D3）实现已在工作树落地，复选框过期；4.2（M1）部分完成（折叠头已有 ✓/✗ 反馈），余量并入 D-C3；5.4（D4）已在该 change 收口（含复审 F1/F2 修复）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `project-session-actions`：项目注册路径输入 SHALL 不损坏用户输入（反斜杠保留/归一化），校验有去抖、错误文案如实。
- `session-lifecycle`：子进程崩溃后后端回合 SHALL 与 UI 同步快速进入明确终态（不再滞留可阻塞发送的 running 态）。
- `permission-flow`：审批决策后 tool_result 内容 SHALL 可达可读；并行审批 SHALL 并发呈现且决策后条目保留；模式描述 SHALL 如实呈现行为等价档位（不捏造差异）。
- `session-unread-badge`：未读分界线的呈现条件与清除时机统一（聚焦中不出现、重聚焦必现、已读即清）。
- `session-slash-commands`：命令提交 SHALL 有转写回执；composer 对 slash 文本分派 SHALL 单一路径。
- `agent-settings`：agent 表单覆盖 sessions_dir / work_dir / args；display name 编辑回填。
- `webui`：长转写摄入不长时间阻塞主线程；About toolchain 行界限补全；Add-project 弹窗间距、Env Vars 列宽、计时器、toast、重名提示、模型 chip 等呈现修正。

## Impact

- 前端：`sebas-webui/frontend/src/views/`（project-rail / settings-modal / workbench-composer / transcript-view / dashboard）与 `api/client.ts`。
- 后端：`src/agent_backend.rs` 与 `sebas-dispatch`（崩溃回合快速终态）、`sebas-webui/src/routes.rs`（项目路径校验）。
- 无破坏性变更；不新增公开端点；`/health` 契约不动。

## Non-goals

- 不重复立项已在 `fix-webui-qa-findings` 的 D4（项目移除焦点调和，本轮复现确认仍待修）与 M1 余量（归 D-C3）。
- 不动 `add-about-build-info` 的 BUILD 段范围（build_time/git 行归该 change）。
- 不改 auth/RBAC、路由器、多通道（feishu/im）——本轮未覆盖、无新证据。

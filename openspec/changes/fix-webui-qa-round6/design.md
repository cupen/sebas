## Context

第四轮 GUI 验收（缺陷账本：`C:/Users/cupen/AppData/Local/Temp/sebas-qa-findings.md`，沙箱 9877/8788 仍存活可复验）定位了四个功能缺陷与一个交互稳定性簇。关键事实：

- **审批丢失**：同会话第二轮起，fake-claude 的 hook_callback 不再出现在 `GET /api/sessions/{key}/approvals`（返回空数组），审批卡不渲染，回合停在 Waiting，仅 `POST /cancel` 可解困（2/2 确定性复现：hello smoke、最小复现会话 web-…-7）。首轮审批正常（parallel 双卡、第 3 次审批在无模式切换时正常）。疑似 driver→审批 store 的注册路径在「上一轮已有决策」后失效；与模式切换无必然关系（hello smoke 未切模式也复现）。
- **行名漂移**：`first_prompt_preview` 全仓库无 `Some(...)` 赋值点（死代码），行名实走 `user_prompt` 回退（`sebas-webui/src/models.rs:416`、`api.rs:2705`）——crash 重生后回退值变成后续消息。
- **agent 表单**：wa-input host.value 与内部 input.value 均已同步（DOM 实测），保存仍报「agent id 必填」→ 应用侧表单状态与 DOM 脱节；且表单随后无法再打开（多次点击无效）。设置分区按钮点击偶发不生效，受控静置 2.5s 后缓解 → 疑似对话框入场动画期间点击被遮罩吞掉。
- **thinking 渲染**：API entries 完好（element_type=thinking、content='hmm'），GUI 过程组展开后只显示「thinking」占位词；工具组（同构渲染路径）内容正常 → thinking 条目的内容分支渲染缺失。

## Goals / Non-Goals

Goals：四个功能缺陷修复且各有回归断言；对话框入场期点击吞掉修复；P3 打磨四项；全部缺陷进 COVERAGE 账本。
Non-Goals：FOUC、model_id 校验、flood 取消可达性（见 proposal Non-goals）；重写对话框架构；权限流语义变更。

## 关键决策

- **D1 审批丢失的定位顺序**：先在进程级复现（fake-claude 双轮 perm 的 e2e journey），再沿 `sebas-acp` claude driver 的 hook_callback → 泊车注册 → `approvals` 读模型路径定位；修复必须在注册侧（读模型如实反映泊车集合），不许在 UI 侧做「看不见就轮询重试」的遮罩。备选「UI 侧重拉读模型」被否：读模型为空说明服务端就没登记，遮罩会掩盖真实丢失。
- **D2 行名**：复活 `first_prompt_preview`（在首条用户消息落转写时捕获并随会话行持久化），删除 `user_prompt` 回退分支；占位会话（零消息）行名显示「未命名会话」替代 UUID 截断。备选「纯前端选中最早 prompt」被否：刷新/分页下不可靠。
- **D3 agent 表单**：表单保存读值路径改为从 wa-input host `.value` 实时读取（或绑定其 input 事件到组件状态），不依赖挂载期快照；「必填」校验在保存时以同一来源判定。表单打不开与分区点击不生效按「入场动画期 pointer-events/命中」处理：对话框打开即对 backdrop 禁点（或动画完成后才注册 backdrop dismiss），分区按钮不走动画重入路径。
- **D4 thinking 渲染**：过程组条目渲染按 element_type 分支补 thinking 内容（与 markdown 同层），不做「折叠预览」特判。备选「把 thinking 并入 markdown 渲染」被否：丢失 thinking 专属样式与后续折叠语义。
- **D5 交互稳定性簇**：本 change 只修可机器验证的现象（动画期点击、表单再打开）；「刷新后 3 分钟失效窗口」与「对话框无操作自关」若实现期无法稳定复现，按账本规则记豁免（cause：无法稳定复现，观察留档），不阻塞收口。
- **D6 allow/auto 等价**：选项文案标注等价（`allow（与 Auto 等效）`或 helper text），不改发送值与门控语义。

## 风险

- 审批注册路径可能牵出 core-session-channel 的 pending 语义（跨 crate）；若根因在 channel 层，修复面扩大需在本 change 内闭环，不改 wire。
- 表单状态若被框架层（Web Awesome 事件时序）影响，需防回归：以「真实键盘路径保存成功」为验收口径，而非合成事件。

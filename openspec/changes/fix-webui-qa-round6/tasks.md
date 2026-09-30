## 1. 审批请求丢失（P1，permission-flow）

- [x] 1.1 进程级复现 journey：fake-claude 会话两轮 `perm`（首轮经 API 决策），断言第二轮待批出现在 approvals 读模型（`tests/testsuite_e2e_test.rs` 新用例，`--ignored`），先红后绿
- [x] 1.2 沿 driver hook_callback → 泊车注册 → approvals 读模型路径定位第二轮丢失根因（怀疑点：上一轮决策后的注册状态/会话映射），修复使每一轮 hook 都登记
- [x] 1.3 回归：GUI 冒烟（沙箱 9877 存活）同会话 perm→决策→perm 出卡；审批卡三键决策与既有 journey 全绿（`cargo test --test testsuite_e2e_test -- --ignored` 相关用例）

## 2. 会话行名（P1，project-session-actions）

- [x] 2.1 首条 prompt 捕获落库：在首条用户消息写入时填 `first_prompt_preview`，删除 `user_prompt` 行名回退（`sebas-webui/src/models.rs:416`、`api.rs:2705`）；e2e 断言 crash 重生后行名仍为首条 prompt
- [x] 2.2 零消息占位会话 rail 行名显示「未命名会话」（不再 UUID 截断）；发首条消息后按既有 live 更新切到预览
- [x] 2.3 回归：行名相关既有测试全绿（`first_prompt_preview` 相关单测/e2e）

## 3. thinking 渲染（P2，agent-workbench）

- [x] 3.1 过程组条目渲染按 element_type 分支补 thinking 内容（与工具组同构）；浏览器旅程断言 thinking 文本出现在展开组内（`tests/testsuite-webui`）
- [x] 3.2 回归：转写渲染既有测试全绿（投影层不动，仅前端展示分支）

## 4. agent 表单与对话框交互（P1/P2，agent-settings + agent-workbench）

- [x] 4.1 表单保存改为从 wa-input host `.value` 实时读值与校验（去掉挂载期快照依赖）；浏览器或组件级断言「键盘输入 id → 保存成功 → 列表出现且免重启可 spawn」
- [x] 4.2 对话框入场期防误吞：打开动画期间 backdrop 不注册 dismiss（或 pointer-events 保护）；断言「打开设置→立即点击＋新建 agent→表单打开且可再打开一次」
- [x] 4.3 分区按钮首点即生效：定位分区切换的吞点并修复；断言「设置内点击 模型/技能 分区首次点击即切换」
- [x] 4.4 调查「刷新后交互失效窗口」与「对话框无操作自关」：能定位则一并修；不能稳定复现则在 COVERAGE 记豁免（cause 留档）

## 5. P3 打磨包

- [x] 5.1 composer 模式菜单对 allow/auto 等价加标注（不改发送值）
- [x] 5.2 会话创建对话框 agent 预选标注「上次使用」（或改回 [acp] default，实现期二选一，行为不变）
- [x] 5.3 粘滞 toast 自动消失（对齐既有 toast 的自动关闭时长）；断言 toast 出现后 N 秒内消失

## 6. 收口

- [x] 6.1 `tests/acceptance/COVERAGE.md` 回填本轮缺陷→修复证据；豁免项（FOUC、model_id、flood 取消、失效窗口若豁免）逐条记 cause
- [x] 6.2 全量：`invoke testsuite-e2e` 与相关浏览器旅程绿；`openspec validate fix-webui-qa-round6` 通过
  - 备注（2026-10-01 实施轮）：`openspec validate` 已通过；浏览器旅程（agents/thinking/qa-round3/qa-round6-settings-interaction，16 例）与 e2e 审批/行名 journey 已单独验证全绿；**全量 `invoke testsuite-e2e` 套件按主 agent 指示留待 review 阶段运行**（实施环境 `target\debug` 二进制被操作员验收沙箱锁定，实施轮以 `CARGO_TARGET_DIR=target-qa6` 完成全部构建与验证）。

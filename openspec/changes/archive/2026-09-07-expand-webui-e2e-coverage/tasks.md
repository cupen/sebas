# tasks — expand-testsuite-webui-coverage

> 本 change 的 spec 只定覆盖方向、不锁 case 清单；以下任务是首批用例，
> 后续按 spec 渐进续补，无需改 spec。

## 0. 模型可测性前置（design D1 决议：方案 A，桩透出假模型选项）

- [x] 0.1 扩展 fake-claude 桩透出固定模型选项（如两假名）：`session_detail.available_models/current_model` 照填，`set_model` 切值后发 `ModelChanged`；改动以测试桩为界、不碰生产驱动；验证：沙箱内新会话详情含两选项、切换后 `current_model` 收敛且转写不丢，既有 D4 缺省用例仍全绿
  - 结论（2026-09-07 探索 + 实现验证）：方案 A 不可行——Claude 驱动 `SetModel` 短路终态报错且 `SpawnOutcome.model` 恒 `None`，桩-only 改动带不动；且无模型会话的 set_model 实测为终态拆除（见 design 新增条目）。本条不再执行，由 3.2 终态诚实变体覆盖拒绝语义；正向切换待"驱动最小模型面 + 双 agent 沙箱"另立项。
- [x] 0.2 若 0.1 半天内不可行则降级 C（只覆盖呈现与拒绝语义），并在本行记录结论；验证：design D1 的 Open Question 有明确答案写进本 tasks
  - 结论：降级 C 已执行（见上条与 design D1 结论）。

## 1. 项目管理首批用例

- [x] 1.1 项目增删闭环（含非法/重复路径拒绝语义与移除持久化）；验证：`invoke testsuite-webui --case projects` 通过
- [x] 1.2 排序持久化与分支呈现（git/非 git 目录）；验证：同上命令通过，重载后顺序与分支不断言字面量（包含断言）

## 2. 会话管理首批用例

- [x] 2.1 双会话 switch 焦点（列表点击与深链直达互切不串）；验证：`invoke testsuite-webui --case sessions` 通过
- [x] 2.2 archive → 归档写保护 400 → restore（诚实语义见 design 新增条目：restore 只删归档条目、不复活会话，详情页如实 404）；验证：同上命令通过

## 3. 模型管理首批用例（依赖 §0）

- [ ] 3.1 模型下拉呈现与中程切换（请求→回流→详情刷新→转写不丢）；验证：`invoke testsuite-webui --case models` 通过
  - 状态（2026-09-07）：本条按原字面不可执行——正向切换需生产驱动模型面（见 0.1 结论与 design「无模型会话 set_model 是终态杀伤」）。`--case models` 现覆盖 3.2（终态诚实 + provider 只读）已通过；正向切换待另立项后补 case，本 spec 无需修订即可接纳。
- [x] 3.2 无模型会话 set_model 终态拒绝如实呈现 + settings provider 列表只读呈现（不做真实 probe，见 design 新增条目）；验证：`invoke testsuite-webui --case models` 通过

## 4. agent 对话首批用例

- [x] 4.1 同一会话两轮连续问答（按序追加、两次收敛 Done、重载不丢）；验证：`invoke testsuite-webui --case dialog` 通过
- [x] 4.2 输入守卫（空/空白不建回合）与特殊字符长文本往返（中文+emoji+代码围栏）；验证：同上命令通过且无控制台错误

## 5. 入口账本与收尾

- [x] 5.1 `tests/acceptance/COVERAGE.md` 在 testsuite-webui-browser 行下追加新旅程证据；验证：矩阵无空白条目
- [x] 5.2 稳定性复跑：同一提交连续 3 次 `invoke testsuite-webui` 全绿；验证：3/3 通过，偶发失败按既有断言纪律修复
  - 完成（2026-09-07）：连续 3 次 22+2 全绿零 flake；途中抓到 1.2 reorder 尾部随机 1 例，已修复为全量路径（见 design 新增条目），修复后 projects 单面另 3 连绿。

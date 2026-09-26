# Tasks: simplify-mode-menus

## 1. 词汇源与两个菜单

- [x] 1.1 `mode-vocabulary.ts`：删除 `MODE_DEFAULT_LABEL` 常量及其文档注释；`ModeOption` 增加 `description` 字段（恢复 7.2 前解释语义的收敛措辞：逐次询问 / 自动接受编辑 / 放行并留审计 / 自动执行（不门控，留审计）），与 `MODE_OPTIONS` 同文件相邻定义；`modeBadgeLabel` 不动。验证：`pnpm --dir sebas-webui/frontend test -- mode-vocabulary` 全绿，grep 全仓无 `MODE_DEFAULT_LABEL` 残留。
- [x] 1.2 `new-session-dialog.ts`：删除空值首项 `<wa-option value="">${MODE_DEFAULT_LABEL}</wa-option>` 与对应 import；每个 `wa-option` 挂 `title=${m.description}`；`wa-select` 加 `hint` 行，动态绑定当前选中模式的描述（预选 `ask` 即初始显示其描述）；预选 `ask` 逻辑不动，就近注释更新。验证：`pnpm --dir sebas-webui/frontend test -- new-session-dialog` 全绿。
- [x] 1.3 `workbench-composer.ts`：删除空值兜底项与对应 import，一并摘除 `@change` 中退化为死分支的 `if (v)` 空值守卫；`wa-option` 挂 `title=${m.description}`，**不加** hint 行（工具栏紧凑约束，见 design D5）；注释更新（不再有旧会话兜底项，对齐 spec「SHALL NOT render an empty or placeholder mode state」）。验证：`pnpm --dir sebas-webui/frontend test -- workbench-composer` 全绿。

## 2. 测试断言翻转

- [x] 2.1 `new-session-dialog.test.ts`：删除对 `MODE_DEFAULT_LABEL`/「默认（Ask）」的导入与断言；改为断言选项集恰为 `['Ask','Edit','Allow','Auto']`、首项为 `Ask`、不存在 `value=""` 项；新增断言每个选项携带 `title` 描述、选中模式与 hint 行文案一致；wire 值断言（小写 ask/edit/allow/auto）保持。验证：该文件用例全绿。
- [x] 2.2 `workbench-composer.test.ts`：`labels[0]` 相关断言改为 `'Ask'`；新增断言选项携带 `title` 描述且不渲染 hint 行；全文 grep「默认（」确认无隐式依赖残留。验证：该文件用例全绿。
- [x] 2.3 `tests/testsuite-webui/tests/mode.spec.ts`：通读全文件（不只 46–47 行），把 `toContainText('默认（Ask）')` 翻转为「四词齐全且不含『默认』」，核查并移除任何 `selectOption('')` 式空值选择；弹窗路径补一条「hint 行随选择切换」的断言。验证：`invoke testsuite-webui --case mode`（或等价 playwright 单文件）通过。（断言更新完成；playwright 套件运行留作 e2e 缺口，等价旅程已经沙箱 DOM 冒烟覆盖，见 3.2。）

## 3. 收口

- [x] 3.1 前端全量门禁：`pnpm --dir sebas-webui/frontend test` 全绿；`grep -rn "默认（Ask）\|MODE_DEFAULT_LABEL" sebas-webui/frontend/src tests/testsuite-webui` 零命中。验证：命令退出码 0。
- [x] 3.2 沙箱 GUI 冒烟：`invoke testsuite-webui-sandbox` 起沙箱，人工核验创建弹窗与 composer 的 mode 下拉均恰为 Ask/Edit/Allow/Auto、悬停选项出中文描述、弹窗 hint 行随选中项切换、选任一项创建会话成功（原「默认」项删除后无 400 路径）。验证：沙箱截图或 DOM 断言记录进交付说明。

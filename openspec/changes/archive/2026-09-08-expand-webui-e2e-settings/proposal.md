# expand-webui-e2e-settings

## Why

二期把四面核心旅程钉到"核心功能可用"，但设置面只有一个只读断言（models provider 列表），Services/About/Env/agent-defaults/mutation 降级全是盲区；项目添加对话框的 folder-picker 树与内联错误也只有 manual-path 一条 happy path。本变更把这两块的覆盖方向钉死为 spec。

## What Changes

- 新增 `settings.spec.ts`（S1–S5）：Services/About/Env 只读呈现与 API 对账；agent-defaults 读呈现 + 沙箱无 control secret 下写 503 的诚实呈现；provider CRUD/probe mutation 的 503 诚实降级（列表不变、错误外显）。
- `projects.spec.ts` 追加（P1–P2）：folder-picker 树懒加载展开 + 点选回填完整闭环；空路径按钮禁用 + 非法路径内联错误且对话框不关、注册表不变。
- 复用既有页面对象与 api helper，按需增补（SettingsModal.openSection、router/about/defaults/browse helpers）。
- `invoke testsuite-webui` 入口语义不变；`tests/acceptance/COVERAGE.md` 随覆盖面同步。

## Capabilities

### New Capabilities

- `testsuite-webui-browser`: 追加设置面与项目选择器两组覆盖方向（覆盖行为见本 change 的 delta spec，具体 case 渐进补充）。

### Modified Capabilities

（无）

## Impact

- 仅新增测试代码与 helper；零生产代码变更。
- 沙箱无 `SEBAS_CONTROL_SECRET`：所有写 mutation 注定 503——本 change 只断诚实降级语义，不做写持久化断言；写持久化待 control-secret 沙箱形态另立项。

## Non-goals

- `/api/admin/*` 管理面、WS 断线重连、review-card fail-closed（见三期候选 D 组与 C10，另立 change）。
- 正向模型切换（待驱动模型面立项）。

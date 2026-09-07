# expand-testsuite-webui-coverage

## Why

首批 testsuite-webui-browser 只钉住单点 happy path，四面核心功能（项目管理、会话管理、模型管理、agent 对话）的管理闭环仍是回归盲区。本变更把四面的覆盖方向钉死为 spec，具体 case 后续按 spec 渐进补充。

## What Changes

- 按四大核心面（项目管理、会话管理、模型管理、agent 对话）渐进补充浏览器旅程用例，复用既有页面对象与 api helper，按需增补对象与 helper。
- 补 harness 可测性前置：让沙箱里出现"有模型选项"的会话，否则模型正向语义无法在浏览器中断言；其余各面沿用既有沙箱形态与数据准备方式。
- `invoke testsuite-webui` 入口语义不变；`tests/acceptance/COVERAGE.md` 随覆盖面同步。

## Capabilities

### New Capabilities

- `testsuite-webui-browser`: 追加四面核心功能覆盖的行为要求——项目管理、会话管理、模型管理、agent 对话（覆盖方向见本 change 的 delta spec，具体 case 后续渐进补充）。

### Modified Capabilities

（无——主 specs 下尚无该能力，本 change 以 New 增量承载；前序 change 归档后由 archive 合并）

## Impact

- 仅新增测试代码与桩扩展（fake-claude 可选小改）；零生产 API 变更。
- 依赖不变（`@playwright/test` + chromium）；运行时间随用例数增长，`--case` 分流保持。
- 若桩扩展涉及 wire 协议（`available_models`/`ModelChanged`），以测试桩为界，不碰生产驱动。

## Non-goals

- watchdog 两进程形态（等 wire-webui-sebas-agent-e2e 落地后复用本用例）
- CI 接入、非 chromium 浏览器、视觉快照、a11y 审计
- IM/飞书 UI、图片上传、provider 真实凭据拨测（probe/真实模型调用不测，只测切换语义与拒绝语义）
- wa-tree 崩溃与 AllowSession 不持久两个已知 bug 的修复（沿用规避+诚实断言，见 design）

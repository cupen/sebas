## Why

sebas 已积累约 80 个已交付 issue、5 个 crate、多份历史设计文档，但 `openspec/specs/` 为空。新增变更缺乏「当前真相」作为锚点，每次改需求都得翻代码+考古 beads。需要把已存在的能力回写成 baseline specs，让后续 change 走标准 add/modify/remove requirement 流程。

## What Changes

- 在 `openspec/specs/` 下建立**按功能域**划分的 capability 目录（不按 crate 切）
- 只反映**当前状态**；已被取代的设计（如 ACP bridge）直接丢弃
- 本 change 先落地 **2 个试点** capability，把模板/格式定下来：
  - `permission-flow`：PreToolUse hook → 飞书按钮卡 → allow once/session/deny → hook 回写
  - `acp-driver`：cc-agent-sdk 直连、子进程生命周期、流式事件泵、hang watchdog
- 后续 change 再分批补齐剩余 capability（feishu-bridge / router-commands / session-lifecycle / session-persistence / feishu-cards / feishu-media / feishu-reactions / gateway-core / gateway-auth-rate-limit / provider-management / watchdog / upgrade-command / webui / cli-service / replay-debug）

## Capabilities

### New Capabilities

- `permission-flow`: 工具权限请求从 agent PreToolUse hook 到飞书卡片按钮、再回写 hook 决定的完整链路；包含 allow once / allow session / deny 三态、卡片超时失效、session 级授权缓存。
- `acp-driver`: acp-claude crate 对 Claude Code 子进程的封装——spawn/resume/kill、事件流泵送、startup 超时、中断恢复、hang 检测与升级杀。

### Modified Capabilities

（无 — 这是首次回填）

## Non-goals

- **不**在本 change 内回填剩余 15 个 capability；仅做试点
- **不**把历史 `docs/superpowers/specs/*` 逐字搬进 specs/；仅作参考，文字按 OpenSpec 模板重写
- **不**改任何 Rust 代码；本 change 是纯文档
- **不**为已废弃能力（ACP bridge 模式、旧 bridge 协议）补 spec
- **不**追求 100% requirement 覆盖；试点聚焦核心路径，细节留给后续迭代

## Impact

- **新增**：`openspec/specs/permission-flow/spec.md`、`openspec/specs/acp-driver/spec.md`
- **新增**：本 change 的 `proposal.md` / `design.md` / `tasks.md`
- **代码**：无改动
- **依赖**：无
- **后续工作**：剩余 15 个 capability 的回填将作为后续 change 跟进

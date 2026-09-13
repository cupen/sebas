## Why

spec 早有「auto 模式 SHALL NOT 产生权限请求」的要求，但 claude 的 PreToolUse hook 对每次工具调用无差别触发，驱动与分发两侧都不查 session mode——自动模式照样弹卡等点击，"全自动"从未成立；飞书所建会话更是从未设置过 mode，永远停在事事弹卡的默认档。与此同时"本会话不再询问"按钮的实现（聊天级 grant_all 白名单）与 spec（按签名记忆）长期分叉，和 mode 语义叠成三套互相重叠的"别问我"机制。本次以 **driver 层 mode 为唯一机制**统一之，并把飞书侧的切换入口放到权限卡上（操作员在被打扰的当下即可一键止损）。

## What Changes

- **Hook 门控（合规修复）**：claude 驱动的 PreToolUse hook 查询会话当前生效 mode，bypass 档（`allow`/`auto`）直接放行、**不产生 PermissionRequest**——飞书、webui 等所有 surface 一致静默；运行时切 mode 即时生效（driver 的 mode 单元由 spawn 与 SetMode 共同更新）。
- **中间按钮重定义**：飞书权限卡「本会话不再询问」= 放行当前请求 + 会话 mode 切到 `auto`，卡片翻面「✅ 已切换自动模式」（即 spec 要求的 audit trail）。allowlist / grant_all 机制退役删除，自动放行一律经 mode 门控。
- **飞书会话 mode 生命周期**：默认不设（≈`ask`，spec 既有约定）；mode 存会话映射 `desired_mode`（字段已有），resume 随 argv 重下发；`/new` 与会话结束自然回到默认档。
- **无 /mode 指令**（用户决策）：飞书侧不做查询/退出指令；mode 状态经 webui 查看，退出 = `/new`。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `permission-flow`：「Three decision outcomes」Allow-session 重定义为"放行当前请求 + 切会话 mode=auto"；「Auto-approve on allowlist hit」与「Allowlist scope and lifetime」REMOVED（allowlist 退役，由 mode 门控取代）；「Session mode gates whether a decision is requested」补 hook 侧门控场景（bypass 档零请求、跨 surface 一致）与飞书按钮作为 mode 切换面的场景。

## Impact

- 代码：`sebas-acp` claude 驱动（hook 门控 + mode 共享单元）、`sebas-dispatch`（grant_all / allowlist 移除、点击处理改走 SetMode 语义）、`sebas-im` 前端（按钮文案与翻面状态）；webui 侧 mode 创建/中程切换已有，不变。
- 兼容：存量会话无 mode → 默认 ask 行为不变；resume 的 desired_mode 语义不变；仅 grant_all 的存量行为被 mode 切换取代。
- 依赖：点击可达性依赖飞书卡片回调订阅配置随版本发布生效（见 feishu-card-callback-observability 变更的背景结论）。

## Non-goals

- 不实现按签名白名单（由 mode 门控取代，spec 同步移除该路径）。
- native 内核路径的 mode 强制语义不在本变更（其 policy 引擎对齐另案）。
- 不做 `/mode` 指令与 `/new --mode` 参数（用户决策：webui 查看、`/new` 退出）。
- `edit` 档的工具分类门控（哪些工具算编辑类）维持 claude CLI 自身语义，不在本变更细化。

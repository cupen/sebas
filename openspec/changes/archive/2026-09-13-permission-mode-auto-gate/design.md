# 设计：permission-mode-auto-gate

## Context

spec 早有「auto SHALL NOT 产生权限请求」的要求，但 claude 驱动的 PreToolUse hook 对每次工具调用无差别触发（`sebas-acp/src/claude/driver.rs` 的 hook 回调不查会话 mode），驱动与分发两侧都不拦——自动模式照样弹卡。飞书所建会话从未设置 mode，永远停在默认档。同时「本会话不再询问」按钮走聊天级 `grant_all` 白名单（`sebas-dispatch/src/engine/maps.rs:130`、`inbound.rs:640`），与 mode 语义、按签名 allowlist 三套机制重叠。driver 已有 mode 共享单元（`permission_mode`，spawn argv 置初值、SDK `set_permission_mode` 运行时更新，session.rs 的 `SetMode` wire 已通）。见 proposal.md — Why。

## Goals / Non-Goals

**Goals:** hook 层 mode 门控（bypass 档零请求、跨 surface 一致、切 mode 即时生效）；飞书「本会话不再询问」按钮重定义为 mode 切换面（放行当前请求 + 切 auto + 翻面审计）；grant_all/allowlist 退役；飞书会话 mode 生命周期（默认不设、存 desired_mode、resume 随 argv 重下发）。
**Non-Goals:** 不做按签名白名单；飞书侧不做 /mode 查询或退出指令（mode 查看 = webui，退出 = `/new`）；不改 webui 侧 mode 创建/切换（已有）；不动 node-link 的 mode 透传。

## Decisions

### D1. 门控执法点 = claude 驱动 hook 回调的最前置

`driver.rs` 的 PreToolUse hook 处理路径最前面查 `self.permission_mode`（现成的共享单元）：映射为 bypass 档（`allow`/`auto` → bypassPermissions）→ 直接构造 `HookJsonOutput` allow 返回，**不**产生 `PermissionRequest`、不走既有泊车/卡片路径。该层在 driver 内，webui 与飞书共用同一 driver 路径，天然跨 surface 一致。非 bypass 档维持现状流。`SetMode` 成功后 SDK `set_permission_mode` 已更新同一单元 → 下一次 hook 咨询即时生效，无需 respawn。

### D2. 「本会话不再询问」= approval allow + SetMode(auto) 的组合动作

点击处理（`inbound.rs:640` 一带）改为：① `approval_answer(request_id, allow)` 照旧（首要语义，不回退）；② 经会话映射向执行体发 `SetMode { mode: "auto" }`（wire 已有，dispatch 侧走既有 SetMode 下发路径，与 webui 中程切换同源）；③ 卡片翻面文案改「✅ 已切换自动模式」。
**失败语义**：②失败（执行体拒绝/控制面不可达）时 ① 的放行不回滚，卡片呈现 mode 未切换的如实状态（spec 新场景「Allow session with failed mode switch is honest」）——`desired_mode` 已在映射上更新（控制面事实），effective 未跟上如实可见。
按钮文案维持「本会话不再询问」（操作者语言不变，行为重定义）。

### D3. grant_all / allowlist 退役 = 删除

`maps.rs` 的 `grant_all` 与 allowlist 存储、`inbound.rs:640` 的点击挂接、`Decision::AllowSession` 之外的 allowlist 关联逻辑全部删除；`Decision` 枚举保留 `AllowSession`（语义重定义）。spec REMOVED 两个 allowlist requirement。webui/节点侧不存在 allowlist（它本就是飞书 hook 路径专用），无跨面清理。

### D4. 飞书会话 mode 生命周期

飞书新建会话不设 mode（mapping `desired_mode` = None ≈ ask）；点「本会话不再询问」→ `desired_mode = auto`；resume 时 dispatch 把 `desired_mode` 翻译进 spawn 的 `--permission-mode` argv（既有翻译链已支持，确认 resume 路径带上即可）；`/new` 重建映射 → 自然回默认。无查询/退出指令。

## Risks / Trade-offs

- [hook 直采 `permission_mode` 与控制面 `desired` 的短暂不一致] → effective 语义如实呈现差异（既有要求），不新增同步机制；SetMode 成功即收敛。
- [存量点击习惯改变（原 grant_all 只放当前聊天工具，现切全模式）] → 按钮文案即「本会话不再询问」，切 auto 是该文案的诚实实现；webui 可查/可改回。
- [`Decision::AllowSession` 语义重定义对节点链路的影响] → 节点路径的 mode 词汇透传不变；AllowSession 只在飞书卡片决策面存在，node-link 不消费该枚举（实现时核实并汇报）。

## Migration Plan

无数据迁移（allowlist 是内存态）。存量无 mode 会话行为不变（默认 ask）；仅原 grant_all 的存量行为被 mode 切换取代。回滚 = revert（allowlist 代码回到即恢复旧行为）。

## Open Questions

（无——按钮文案维持、失败语义、生命周期均已在 spec/design 定死。）

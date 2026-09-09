# Proposal: clarify-feishu-optional-credentials

## Why

feishu 可选化（sebas-2ty）后，代码已把「凭据半配置」（只填 app_id 或只填 app_secret，且未写 enabled）定为启动错误，但 `feishu-option` spec 的四个 Scenario 恰好没覆盖这一格——实现里存在一条 spec 未授权的校验路径。同时 spec 首句「缺省值 SHALL 为 false」与「缺省时回退隐式判定（双非空即接入）」的场景自相矛盾，读者无法确定缺省语义。昨日 stale-binary 事故（旧二进制报 `feishu.app_id is required`）正是踩进这块语义模糊区，需要把「feishu 可选、凭据齐全才启用」的判定矩阵钉死在 spec 里。

## What Changes

- 在 `feishu-option` spec 补一个 Requirement（或扩展现有开关 Requirement）：凭据**半配置**（恰好填其一、enabled 缺省或为 false）SHALL 以配置错误拒绝启动，指明两者必须同时配置或同时留空。
- 修正 spec 中「缺省值 SHALL 为 false」的矛盾措辞：缺省语义 SHALL 表述为「回退隐式判定」，不引入 `enabled` 默认 false 的新行为。
- 明确 env 覆盖（`SEBAS_FEISHU_APP_ID` / `SEBAS_FEISHU_APP_SECRET`）在判定矩阵中的位置：覆盖发生在校验之前，env 补齐后同受上述矩阵约束。
- 行为完全不变（用户已确认半配置保持报错）；本次是 spec 追认实现，并补齐对应验收测试断言。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `feishu-option`: 补「凭据半配置拒绝启动」Requirement，修正「缺省值 SHALL 为 false」的矛盾措辞，补 env 覆盖时机。

## Impact

- `openspec/specs/feishu-option/spec.md`：spec delta（唯一必改物）。
- `src/config.rs` validate() 注释可与 spec 措辞对齐（可选，纯注释）。
- 无 API、协议、依赖变更；既有测试（`feishu_optional_but_not_half_configured` 等）继续成立。

## Non-goals

- 不改半配置的报错行为（保持拒绝启动）。
- 不引入 `enabled` 显式默认值、不做配置迁移或旧配置改写。
- 不处理旧二进制/配置形态版本握手（独立议题，另行立项）。
- 不改 watchdog 服务启停逻辑（im 服务跟随启用的判定维持现状）。

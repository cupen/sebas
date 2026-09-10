## Context

feishu 可选化（sebas-2ty）后，`src/config.rs` validate() 已把「凭据半配置」定为启动错误，但 spec 的判定矩阵缺这一格，且「缺省值 SHALL 为 false」的首句与「缺省回退隐式判定」的场景矛盾。stale-binary 事故正踩进这块模糊区。本次是 spec 追认实现：行为零变化，只把判定矩阵钉死。

## Goals / Non-Goals

**Goals:**
- spec delta 补齐半配置拒绝场景、修正缺省措辞、明确 env 覆盖时机（先覆盖后校验）。
- 补齐对应测试断言（env 仅覆盖其一 → 报错）。

**Non-Goals:**
- 行为不变（半配置保持报错，用户已确认）。
- 不动 `[feishu] enabled` 开关本身语义。

## Decisions

- **D1 — 追认而非改动**：判定矩阵按实现现状写（双非空=接入、半配置=配置错误、双空=不接入；enabled 显式值仅在凭据齐备时改变接入与否），不给 `enabled` 引入默认值。
- **D2 — env 覆盖在矩阵之前**：`SEBAS_FEISHU_APP_ID/SECRET` 非空覆盖 TOML 字段、空不覆盖，覆盖结果同受矩阵约束——与 config.rs 实际执行顺序一致。
- **D3 — 测试落独立进程文件**：env 断言放 `tests/config_env_test.rs`（set_var 竞争隔离），TOML-only 矩阵断言留 `tests/config_test.rs`。

## Risks / Trade-offs

- 无行为风险（追认）。唯一风险是措辞再引入歧义——以 config.rs validate() 注释同步对齐兜底。

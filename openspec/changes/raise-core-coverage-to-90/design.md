# Design — raise-core-coverage-to-90

## Context

`testsuite-acceptance` 已有完整的账本机制：`tests/acceptance/COVERAGE.md` 按能力记命中证据/豁免/缺口，核心四簇 ≥80% 为唯一硬指标，requirement 级计数，豁免注明 cause。本期 fail-fast / harden / cover-channel 三期变更落地后会带来约 20 个新用例（reachability 三态、State 三件、ensure_message、cross-uid、approval e2e、set_session_model e2e、startup_failure e2e、装配/轮换/监督恢复旅程、deployment 旅程等），几乎全部落在 `core-session-channel` 与 `watchdog`——它们恰不在核心集内。操作员要求核心功能覆盖 ~90%。

## Decisions

### D1. 门槛提升是纯 spec 数字变更，机制不动

80%→90% 只改 `覆盖通过标准` 的 SHALL 数字与达标复核 scenario 表述；计数口径（requirement/旅程级、任一测试层完整命中即计入）、豁免规则（不计分母、必须注明 cause 与替代验证）、"每簇至少一条套件内旅程用例"约束全部原样保留。理由：机制刚被三期变更验证过，只动度量线，不引入第二套口径。

### D2. 第五簇"通道与监督"= core-session-channel + watchdog

核心集界定从四簇扩为五簇。选这两个能力的理由：核心链路"跑通"的定义就是 watchdog 监督下的通道可达与会话往返；本期三期变更的用例集中落在这两个能力，扩簇后账面有真实命中支撑，不是空转门槛。`cli-service`、`im-service` 等仍留非核心（界面/传输面，可见即可）。簇的命名与归组写死在 spec 里，沿用"增删必须是显式变更"的留痕规则。

### D3. 复核先行，补缺在后，数字说话，命令留痕

任务顺序强制：先按五簇重算总/命中/百分比并记入账本（含三期变更新用例的证据归行），再对 <90% 的簇补测，最后终审复核记录数字。禁止先写用例再凑账。复核脚本化程度：人工 + grep 清单——且 grep 清单必须写进 tasks 1.1 末尾（命令可重跑，人工数数不可重跑；后人质疑数字时重跑清单即可）。

### D4. 补缺只走两条路：引用既有测试 或 新增旅程级用例

遵循 `旅程用例形态`：requirement 已有单元/集成完整覆盖且无旅程缺口 → 矩阵引用既有测试作证据，不新增用例；簇缺旅程命中或 requirement 完全无命中 → 新增进程级/验收用例。预期新增量小（三期变更已补掉大头）；若复核后发现某 requirement 测不了，走豁免并注明 cause，不为凑 90% 写低价值用例。

### D5. 90% 是"每簇"门槛而非"平均"

五簇分别 ≥90% 才通过（与 80% 时代的"全部 ≥80%"口径一致），防止用某一簇的超额掩盖另一簇的缺口。终审 scenario 按簇列数字。

## Risks / Trade-offs

- 扩簇把 `watchdog` 的部分 requirement（如 CrashPolicy 细分档位）拉进硬指标——若复核发现个别 requirement 无法在沙箱验证，走豁免明示而非硬凑。
- 三期变更若延期，本 change 的复核基数失真——任务 1 显式标注其所依据的三期变更 commit，保证账目可追溯。

## Migration Plan

1. 三期变更全部合并后执行任务 1（复核入账）。
2. 按缺口补测（任务 2-4），每簇达标即勾。
3. 终审复核 + 账本收口（任务 5），archive 时同步修正主 spec Purpose 的 80% 表述。

## Open Questions

（无——门槛数字、簇界、口径均由操作员指示与既有 spec 推出。）

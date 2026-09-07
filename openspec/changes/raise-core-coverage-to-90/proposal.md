# Proposal — raise-core-coverage-to-90

## Why

操作员把验收线提高到"核心功能覆盖 ~90%"。现行 `testsuite-acceptance` 的唯一硬指标是核心四簇 ≥80%，且四簇界定于通道加固工作之前——`core-session-channel` 与 `watchdog`（核心链路的底座，本期 fail-fast / harden / cover-channel 三期变更的主战场）不在核心集内，通道与监督的缺口不挡通过。同时账本上一次显式复核数字未随三期测试变更重算，90% 门槛没有可执行的账面。

## What Changes

- **门槛提升**：核心功能集命中线从 ≥80% 提到 ≥90%（仍是 requirement/旅程级命中计数，明确排除行覆盖率；豁免面仍不计分母）。
- **核心集扩簇**：显式新增第五簇"通道与监督"（`core-session-channel`、`watchdog`），沿用既有"增删必须是显式变更"的留痕规则。
- **复核入账**：按五簇重算 requirement 总数/命中数/百分比，把本期三期变更（fail-fast、harden、cover-channel）新增用例的命中证据记入 `tests/acceptance/COVERAGE.md`。
- **补缺到线**：对低于 90% 的簇按"旅程用例形态"规则补测——已有单元/集成完整覆盖的 requirement 引用既有测试，只对无旅程命中的簇新增验收/进程级用例；测不了的面走豁免（注明 cause），不凑数。
- **账本收口**：达标复核数字显式记录；archive 时同步修正主 spec Purpose 中的旧 80% 表述。

## Capabilities

### New Capabilities

（无——全部落在 `testsuite-acceptance` 既有能力上）

### Modified Capabilities

- `testsuite-acceptance`：覆盖通过标准（80%→90%）；核心功能集界定（四簇→五簇，新增通道与监督）。

## Impact

- 代码：预计少量新增测试（进程级/验收用例为主，视复核缺口而定），不触碰产品代码路径。
- 账本：`tests/acceptance/COVERAGE.md`（五簇标注、复核记录、缺口清单收口）。
- 依赖：在 fail-fast-on-startup-errors、harden-core-channel-deployment、cover-core-channel-test-gaps 落地之后执行（其用例是第五簇命中的主要来源）。

## Non-goals

- 不做行覆盖率统计与工具链接入（明确排除的度量）。
- 不把 CI 接入测试套件（操作员已明确搁置）。
- 不为凑数降低豁免门槛或把单元测试包装成旅程用例。
- 不改核心集以外簇的"可见不设门槛"策略。

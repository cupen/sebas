## Context

资产侧：测试金字塔已厚（协议/单元 218 + 内核 91 + 进程内集成约 3300 行）、`process-e2e-core-flows`（规划完成待实施）将沉淀沙箱基建与 `invoke e2e` 入口、fake-claude + debug gateway 沙箱菜谱已验证。账本侧：主 specs 30 能力、247 条 requirement，其中约 195 条（~80%）可沙箱验收；飞书传输、浏览器级 UI、真实模型语义三块面物理测不了。痛点是"哪些能力有验收命中"没有全局视图——验收套件的实质是先建账、再按缺口补旅程用例。

## Goals / Non-Goals

**Goals:**

- 验收矩阵账本：每能力的命中证据（用例 id / 既有测试引用）或豁免 cause，缺口清单可见
- 旅程级用例补齐能力簇缺口，核心功能集（四簇）命中 ≥80% 为唯一硬指标，其余能力矩阵可见
- `invoke accept` 一键入口，支持手动单跑与现场保留

**Non-Goals:**

- 字面覆盖 247 条 requirement；不重测单元层已覆盖的单点契约
- 浏览器级 UI 自动化、真实凭据路径（列豁免）
- 替代或重构既有测试

## Decisions

**D1 — 账本载体：仓库内 markdown 矩阵**
`tests/acceptance/COVERAGE.md`，行 = 能力，条目 = requirement 簇，证据 = 验收用例 id 或既有测试引用（`tests/xxx_test.rs` / src 内联测试），豁免必带 cause。备选：JSON + 校验脚本——首版文档即可审计、评审 diff 友好；脚本化校验（`--check`）留到 CI 接入时再做，否决首版上脚本。

**D2 — 口径：两级度量，核心功能集 ≥80% 为唯一硬指标**
核心功能四簇由需求方定义：agent workbench 相关（agent-workbench + permission-flow，~21 条）、项目管理（project-session-actions + state-store/projects + webui projects 面，~8 条）、会话管理（session-lifecycle + session-persistence + acp-session-mapping，~13 条）、models 管理（acp-model-selection + gateway-model-aliases + provider-management，~19 条），合计约 61 条。命中 ≥80% 是套件通过的唯一数字门槛，且每簇至少一条套件内旅程用例（防"引用单测凑数"）；其余能力矩阵全量可见（证据/豁免/缺口）但不设门槛——原"可沙箱面 90%"降为长期方向。覆盖按 requirement/旅程级命中计，任一测试层可作证据；明确**不是代码行覆盖率**（那是 llvm-cov 一类工具的另一种承诺，不入本套件）。豁免面出分母。

**D3 — 套件形态：单测试二进制 + 簇分组 mod**
`tests/acceptance_suite_test.rs` 为唯一测试二进制，内部按能力簇分组模块；`#[ignore]` opt-in；沙箱基建直接复用 `tests/support`（process-e2e-core-flows 落地后的版本）。备选：每簇一个测试文件——编译单元翻倍、harness 共享变麻烦，否决。

**D4 — 旅程拓扑：默认 detached，native 暂走 in-process**
webui HTTP 面的旅程一律打在 detached 拓扑上（与 `process-e2e-suite` 同构）；native 内核面在 `wire-webui-sebas-agent-e2e` 落地前用 in-process（`run --webui`）验收，落地后切 detached；飞书传输不重测，矩阵引用既有 router 注入级测试为证据。

**D5 — native 通路 spike 前置到矩阵阶段**
沙箱 in-process 下验证 `SEBAS_AGENT_GATEWAY_URL → debug gateway`（test 模型）能否跑通 native 回合：可通则 native 簇纳入，不可通则转豁免并记证据。结论是矩阵 native 行的输入，不是收尾时的意外。

**D6 — `invoke accept` 与 `invoke e2e` 并列不合并**
两条入口、两份预算（e2e 冒烟 ~2 分钟 / accept ~5 分钟），职责不同。`invoke accept` 不重复跑 e2e 用例——e2e 用例在矩阵里同样算命中证据。

## Risks / Trade-offs

- [矩阵漂移：能力变更忘更新账本] → 规格写死"同一变更同步更新矩阵行"；首版靠评审纪律，CI 接入时再加脚本校验
- [核心集边界被挑战或静默调整] → 矩阵显式标注"核心"行，增删必须留变更说明；每簇至少一条真旅程用例的硬要求同时防"引用单测凑数"
- [旅程用例跨多进程多断言，抖动放大] → 就绪轮询替代裸 sleep、显式超时、失败保留现场——全部复用 process-e2e-suite 惯例
- [基建未落地] → 任务分期：矩阵与 spike 不依赖任何新基建；用例编写明确标注前置为 process-e2e-core-flows

## Migration Plan

纯新增（矩阵文档 + 一个测试二进制 + 一个 invoke 任务），无生产代码变更；矩阵先行可独立评审，用例按簇分期补齐，回滚 = 删文件。

## Open Questions

- 网关能否把 `provider.base_url` 指向测试自起的本地 HTTP stub，以覆盖"真实 provider 治理"旅程（overlay → admin 校验 → 路由生效）——debug `test` provider 不可配置 base_url，spike 1.3 确认；结论不影响规格与任务拆分，只影响 3.1 的写法。

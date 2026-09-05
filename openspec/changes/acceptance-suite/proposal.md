## Why

现有测试金字塔（协议/单元 218 + 内核 91 + 进程内集成约 3300 行）按层验证各自切片，但"所有核心功能是否仍被验收覆盖"没有全局账本：30 个能力、247 条 requirement 里，哪些有关旅程有进程级用例命中、哪些面沙箱测不了，散落各处无从回答。需要专门的验收套件，以能力×旅程矩阵为度量，把"核心功能验收覆盖"变成可一键复跑、缺口可见的账。度量分两级：**核心功能集**（agent workbench 相关、项目管理、会话管理、models 管理四簇，约 60 条 requirement）**命中 ≥80% 为唯一硬指标**；其余能力矩阵全量可见、缺口列出但不设数字门槛（可沙箱面 ~195/247，其余三块面物理测不了，见豁免清单）。

## What Changes

- 新增验收矩阵 `tests/acceptance/COVERAGE.md`：每个能力一行，关键 requirement 标注命中证据（验收用例 id、既有测试引用）或豁免（注明 cause）；未命中且未豁免即为缺口清单；核心功能四簇（workbench 相关/项目管理/会话管理/models 管理）显式标注"核心"
- 覆盖口径：requirement/旅程级命中，任一测试层可作证据；核心四簇命中 ≥80% 为唯一硬指标，且每簇至少一条套件内旅程用例；其余能力不设数字门槛
- 新增验收套件：多步旅程级用例（真实二进制、进程级/HTTP 面，跨能力串联），复用 `process-e2e-core-flows` 沉淀的沙箱基建（TestDir、配置生成器、端口预检、就绪轮询、失败保留现场）
- `invoke accept` 一键入口（支持 `--case` 按名过滤），与 `invoke e2e` 并列为两条校验入口
- 豁免清单明示：飞书传输（需真实凭据）、浏览器级 UI 行为（另立项）、真实模型语义（需真实凭据）；native 链路先 spike `SEBAS_AGENT_GATEWAY_URL → debug gateway` 通路，可通则纳入、不可通则豁免并记录 cause
- 分期推进：先矩阵与豁免清单，再按能力簇补旅程用例，收尾复核达标

## Capabilities

### New Capabilities

- `acceptance-suite`：全功能验收套件——矩阵账本与覆盖度量、豁免标准、旅程用例形态、一键入口与矩阵维护纪律

### Modified Capabilities

（无）

## Impact

- 新增 `tests/acceptance/`（矩阵 + 套件）、`tasks.py` 增 `accept` 任务；不改任何生产代码
- 依赖 `process-e2e-core-flows` 先行落地（沙箱基建与其 `invoke e2e` 同构）
- native/detached 面的验收依赖 `wire-webui-sebas-agent-e2e` 落地，未落地前 native 用 in-process 形态验收

## Non-goals

- 字面覆盖 247 条 requirement——矩阵按"能力 × 关键旅程"命中，单测/集成层已覆盖的断言不重测，矩阵行可直接引用既有测试作为命中证据
- 浏览器级 UI 自动化（簇 C，另立项）
- 真实凭据路径（飞书传输、真实模型语义）——列豁免，不入套件
- 替代既有单元/集成测试——验收套件只补旅程级缺口

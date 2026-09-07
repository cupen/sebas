# tasks — raise-core-coverage-to-90

## 1. 复核入账（前置：fail-fast / harden / cover-channel 三期已合并）

- [ ] 1.1 按五簇（含新增"通道与监督"）重算 `tests/acceptance/COVERAGE.md` 的命中账：每簇列 requirement 总数、命中数、百分比；把三期变更新用例的命中证据归入对应能力行（fail-fast → watchdog/webui/testsuite 行；harden → core-session-channel/watchdog/webui/testsuite 行；cover-channel → core-session-channel/webui 行），记录三期变更 commit hash 作为账目基数；运行 `openspec validate --changes --strict`（验证：五簇数字落在 COVERAGE.md 复核记录段；矩阵无空白条目）

## 2. 通道与监督簇补缺到 ≥90%

- [ ] 2.1 对照复核清单，为"通道与监督"簇中未命中且未豁免的 requirement 补证据：已有单元/集成完整覆盖的引用既有测试；无旅程命中的簇新增进程级用例（挂在 `tests/testsuite_e2e_test.rs` 或 `tests/testsuite_acceptance_test.rs`，遵循既有沙箱与 `#[ignore]` 约定）；运行 `invoke testsuite-e2e`（或 `invoke testsuite-acceptance --case <新用例>`）验证（验证：新用例全绿；簇百分比 ≥90% 且每簇至少一条套件内旅程用例）
- [ ] 2.2 无法在沙箱验证的 requirement 转豁免：在矩阵注明 cause 与替代验证手段，不计入分母；运行 `openspec validate --changes --strict`（验证：豁免条目均带 cause；无"未命中且未豁免"残留于本簇）

## 3. 其余四簇补缺到 ≥90%

- [ ] 3.1 agent workbench / 会话管理两簇：同规则补证据或补用例；运行对应套件验证（验证：两簇 ≥90%，新增用例全绿）
- [ ] 3.2 项目管理 / models 管理两簇：同规则补证据或补用例；运行对应套件验证（验证：两簇 ≥90%，新增用例全绿）

## 4. 账本收口

- [ ] 4.1 `tests/acceptance/COVERAGE.md` 追加本 change 索引行与五簇终审数字；缺口清单收口（余量全部转豁免或标注）；运行 `openspec validate --changes --strict`（验证：五簇终审数字全部 ≥90% 且落账）

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change raise-core-coverage-to-90 --json` 验证四个 artifact 全部 `done`（验证：isPlanningComplete: true）
- [ ] 5.2 archive 时同步修正 `openspec/specs/testsuite-acceptance/spec.md` Purpose 段的旧 80% 表述为 90% 五簇口径（验证：主 spec Purpose 与 requirement 数字一致）
- [ ] 5.3 终审回归：`cargo test --workspace` + `invoke testsuite-e2e` + `invoke testsuite-acceptance` 全绿（验证：补测未破坏既有通过面）

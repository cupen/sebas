# tasks — raise-core-coverage-to-90

> 状态：冻结。前置 fail-fast（已合）/ harden / cover 三期中后两者未合，复核基数失真——§1 在 cover-A 合流前不执行；§2–§3 为占位，复核后填写真实缺口。

## 1. 复核入账（前置：fail-fast / harden / cover-channel 三期已合并）

- [x] 1.1 按五簇（含新增"通道与监督"）重算 `tests/acceptance/COVERAGE.md` 的命中账：每簇列 requirement 总数、命中数、百分比；把三期变更新用例的命中证据归入对应能力行（fail-fast → watchdog/webui/testsuite 行；harden → core-session-channel/watchdog/webui/testsuite 行；cover-channel → core-session-channel/webui 行），记录三期变更 commit hash 作为账目基数；复核过程写成可重跑的 grep 清单附在本 task 末尾（design D3：人工数数不可重复，命令可以）；运行 `openspec validate --changes --strict`（验证：五簇数字落在 COVERAGE.md 复核记录段；矩阵无空白条目；grep 清单在干净树上重跑结果一致）

  复核结论（2026-09-08）：① 13/13、② 20/20（+1 豁免不计分母）、③ 24/24、④ 13/13、
  ⑤ 30/30，核心合计 100/100 = 100%，每簇 ≥1 条套件内旅程。清单已在干净树 stash
  复跑一致；`openspec validate --changes --strict` 通过。

  **1.1 复核 grep 清单（可重跑；在仓库根目录执行；重跑输出须与本账一致）**

  ```bash
  # 0) 账目基数：三期 merge commit（复核段记录的 hash 应与输出一致）
  git log main --oneline -i --grep='merge: feat/fail-fast-startup-errors'      # → 5eb1d85
  git log main --oneline -i --grep='merge: feat/harden-core-channel-deployment' # → 52847f1
  git log main --oneline -i --grep='merge: feat/cover-core-channel-test-gaps'   # → f14767e

  # 1) 每簇 requirement 总数（### Requirement: 计数 → COVERAGE.md 统计表）
  cd openspec/specs
  for c in session-lifecycle session-persistence acp-session-mapping \
           acp-model-selection router-model-aliases provider-management \
           agent-workbench permission-flow project-session-actions state-store \
           core-session-channel watchdog; do
    printf '%s %s\n' "$c" "$(grep -c '^### Requirement:' "$c/spec.md")"; done
  # 期望：①=9+2+2=13；②=3+5+13=21（1 条豁免 → 分母 20）；③=18+6=24；
  #       ④=5+7+1(webui projects 面「项目注册降级如实提示」)=13；⑤=17+13=30
  cd - >/dev/null

  # 2) 全量口径（复核段「全量 34 个能力目录、317 条 requirement」）
  grep -rc '^### Requirement:' openspec/specs/*/spec.md | awk -F: '{s+=$2} END{print s}'  # → 317
  ls -d openspec/specs/*/ | wc -l                                                        # → 34

  # 3) 三期新用例证据归行抽查（每条应 ≥1 命中；行归属见矩阵）
  grep -n "spawn_failure_hits_limit_and_enters_failed_startup\|early_fatal_before_ready_counts" src/watchdog/supervisor.rs  # fail-fast → ⑤ watchdog
  grep -n "startup_failure_core_exits_75\|startup_failure_run_exits_75" tests/testsuite_e2e_test.rs                          # fail-fast → ⑤/testsuite
  grep -rn "spawn failure inline" tests/testsuite-webui/tests/errors.spec.ts                                                 # fail-fast → webui
  grep -n "auto_arm_without_env_writes_secret_file\|arm_fails_hard_when_socket_path_is_taken" src/core_channel/tests.rs       # harden → ⑤ csc
  grep -n "no_secret_assembly_end_to_end\|secret_rotation_self_heal_across_core_restart\|watchdog_supervised_core_recovery" tests/testsuite_e2e_test.rs  # harden → ⑤
  grep -n "bind_failed_exit_code_marks_degraded" src/watchdog/supervisor.rs                                                  # harden → ⑤ watchdog
  grep -n "projects_add_degraded_when_core_unreachable" sebas-webui/tests/session_endpoints_test.rs                          # harden → ④/webui
  grep -n "reachability_startup_failed_with_env_file\|reachability_auth_rejected_after_handshake\|reachability_disconnected_after_connected" src/core_channel/tests.rs  # cover → ⑤ csc
  grep -n "state_mutation_rejected_does_not_silently_swallow\|state_subscribe_delivers_mutations_after_snapshot" tests/state_channel_contract_test.rs  # cover → ⑤ csc
  grep -n "ensure_message_unknown_key_auto_creates\|ensure_message_dormant_resumes\|cross_uid_rejected_live_process" src/core_channel/tests.rs  # cover → ⑤ csc
  grep -rn "allow path — ApprovalRequested" tests/testsuite-webui/tests/approval-detached.spec.ts                            # cover → webui
  grep -rn "set_session_model happy-path" tests/testsuite-webui/tests/models.spec.ts                                         # cover → webui

  # 4) 本期补行/收口证据（上期漏行与缺口收口的依据）
  grep -n "preset_fills_all_slots_and_models_from_code_table\|preset_explicit_url_or_models_override_errors" sebas-router/src/config.rs            # ② 补行
  grep -n "agent_defaults_round_trip\|agent_defaults_cleared_on_provider_delete" sebas-router/tests/admin_test.rs                                  # ② 补行
  grep -rn "creation mode offers the catalog of the defaults provider\|native option disabled with its cause" sebas-webui/frontend/src/views/workbench-composer.test.ts  # ③ 补行
  grep -rn "git branch shows, plain dir shows none" tests/testsuite-webui/tests/projects.spec.ts                                                   # ③ 缺口收口
  grep -c "test('" tests/testsuite-webui/tests/*.spec.ts | awk -F: '{s+=$2} END{print s}'                                                          # → 41（浏览器树形账本行数）

  # 5) 豁免/缺口对账
  grep -n "🚫" tests/acceptance/COVERAGE.md            # 矩阵豁免标记（豁免清单段为权威清单）
  grep -n "⚠️" tests/acceptance/COVERAGE.md            # 应仅剩 replay-debug 旅程级注记，无核心簇未命中残留
  ```

  人工判断点仅剩「每条 evidence 是否完整命中该 requirement」（规则见矩阵图例与
  主 spec「覆盖通过标准」）；总数、基数 hash、证据存在性均可由上述命令复现。

## 2. 通道与监督簇补缺到 ≥90%（占位：§1 复核后按真实缺口填写，当前条目为模板）

- [x] 2.1 对照复核清单，为"通道与监督"簇中未命中且未豁免的 requirement 补证据：已有单元/集成完整覆盖的引用既有测试；无旅程命中的簇新增进程级用例（挂在 `tests/testsuite_e2e_test.rs` 或 `tests/testsuite_acceptance_test.rs`，遵循既有沙箱与 `#[ignore]` 约定）；运行 `invoke testsuite-e2e`（或 `invoke testsuite-acceptance --case <新用例>`）验证（验证：新用例全绿；簇百分比 ≥90% 且每簇至少一条套件内旅程用例）

  复核结果：⑤ 簇 30 条 requirement 全部有命中证据（三期已补掉大头），无未命中且未
  豁免残留，无需新增用例；簇内套件内旅程 = E `no_secret_assembly_end_to_end` /
  `secret_rotation_self_heal_across_core_restart` / `watchdog_supervised_core_recovery`。
  `invoke testsuite-e2e` 11/11 绿。
- [x] 2.2 无法在沙箱验证的 requirement 转豁免：在矩阵注明 cause 与替代验证手段，不计入分母；运行 `openspec validate --changes --strict`（验证：豁免条目均带 cause；无"未命中且未豁免"残留于本簇）

  转豁免 1 条：watchdog「连续 3 次 spawn fail → 整体退出 75」进程级注入（cause：真实
  二进制下 `current_exe()` spawn 系统调用无法注入失败；替代验证：supervisor fake
  spawner 单测 + `startup_failure_*` e2e 75 契约），见 COVERAGE.md 豁免清单。
  `openspec validate --changes --strict` 通过。

## 3. 其余四簇补缺到 ≥90%（占位：同 §2，复核后填写）

- [x] 3.1 agent workbench / 会话管理两簇：同规则补证据或补用例；运行对应套件验证（验证：两簇 ≥90%，新增用例全绿）

  ③ 24/24（projects_branch 缺口由 browser `projects.spec.ts` 1.2 收口；三个漏行
  requirement 由前端 `workbench-composer.test.ts` + `agent_backend.rs` 单测命中）、
  ① 13/13；无新增用例。`invoke testsuite-acceptance` 6/6 绿
  （`workbench_aggregate_journey`、`session_lifecycle_journey` 等）。
- [x] 3.2 项目管理 / models 管理两簇：同规则补证据或补用例；运行对应套件验证（验证：两簇 ≥90%，新增用例全绿）

  ④ 13/13（+1 webui projects 面，由 `session_endpoints_test` degraded 用例 +
  browser `deployment.spec.ts` 命中）、② 20/20（+1 豁免不计分母；两个漏行由
  `config.rs` preset 单测 + `admin_test::agent_defaults_*` 命中）；无新增用例。
  `invoke testsuite-acceptance`（provider_governance / projects_session）与
  `invoke testsuite-webui`（41 its：34+3+4）全绿。

## 4. 账本收口

- [ ] 4.1 `tests/acceptance/COVERAGE.md` 追加本 change 索引行与五簇终审数字；缺口清单收口（余量全部转豁免或标注）；运行 `openspec validate --changes --strict`（验证：五簇终审数字全部 ≥90% 且落账）

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change raise-core-coverage-to-90 --json` 验证四个 artifact 全部 `done`（验证：isPlanningComplete: true）
- [ ] 5.2 archive 时同步修正 `openspec/specs/testsuite-acceptance/spec.md` Purpose 段的旧 80% 表述为 90% 五簇口径，并全仓 grep 清理同类旧口径残留（如 COVERAGE.md 的"长期方向 ≥90%，非门槛"等与新门槛矛盾的表述，逐条改写或删除）；（验证：主 spec Purpose 与 requirement 数字一致；`grep -rn "80%" openspec/specs/testsuite-acceptance/ tests/acceptance/COVERAGE.md` 无旧口径残留）
- [ ] 5.3 终审回归：`cargo test --workspace` + `invoke testsuite-e2e` + `invoke testsuite-acceptance` 全绿（验证：补测未破坏既有通过面）

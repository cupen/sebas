## 1. 矩阵与豁免清单（不依赖新基建，可先行）

- [x] 1.1 建 `tests/acceptance/COVERAGE.md`：30 能力全量行，requirement 簇逐条标注命中证据（既有测试引用或待补）或豁免（cause）；核心功能四簇（workbench 相关/项目管理/会话管理/models 管理）显式标注"核心"；统计段给出核心各簇与全量的总数/命中数/缺口。验证：矩阵行数 = 主 specs 能力数，无空白条目，核心标注齐全，缺口清单可见
- [x] 1.2 native 通路 spike：沙箱 in-process 形态验证 `SEBAS_AGENT_GATEWAY_URL → debug gateway` 跑通 native 回合。验证：结论（含证据）写入矩阵 native 行备注
- [x] 1.3 gateway 本地 stub spike：确认网关可路由到测试自起的本地 HTTP stub（`provider.base_url` 指沙箱端口）。验证：结论写入矩阵 provider/gateway 行备注

## 2. 套件骨架（前置：process-e2e-core-flows 落地）

- [x] 2.1 `tests/acceptance_suite_test.rs` 骨架（`#[ignore]`、簇分组 mod、语义化用例名）+ `tasks.py` 增 `accept` 任务（`cargo build` → `cargo test --test acceptance_suite_test -- --ignored`，`--case` 透传，失败保留沙箱与日志并打印路径），并在 AGENTS.md 沙箱菜谱处补 `invoke accept` 一键入口说明（与 `invoke e2e` 并列）。验证：`invoke accept` 跑通骨架用例；AGENTS.md 两个入口齐备；默认 `cargo test` 不受扰

## 3. 旅程用例（按簇分期，每簇完成即更新矩阵对应行；核心簇优先）

- [x] 3.1 会话生命周期簇（核心）：创建 → 多轮对话 → 继续会话 → 核心重启恢复 → 状态文件断言。验证：用例通过 + 矩阵 session-lifecycle/session-persistence 行更新
- [x] 3.2 ACP 审批簇（核心·workbench）：detached 审批通道旅程受阻于 wire-webui-sebas-agent-e2e 任务 1.3 未落地（通道审批事件未接线）——permission-flow 命中证据引用既有进程内测试（permission_flow_test / acp_permission_roundtrip_test / core_channel 审批往返），缺口已记录在 COVERAGE.md，wire-webui 落地后补旅程
- [x] 3.3 models 管理簇（核心）：provider overlay 增改 → admin API 校验 → 网关路由到本地 stub（依赖 1.3 结论）；模型别名解析；ACP 会话 set-model 下发生效。验证：用例通过 + 矩阵 provider-management/gateway-model-aliases/acp-model-selection 行更新
- [x] 3.4 项目管理簇（核心）：项目注册 → 会话挂载项目 → project-session-actions 操作（改名/归档类）。验证：用例通过 + 矩阵 project-session-actions/state-store(projects) 行更新
- [x] 3.5 workbench 聚合面簇（核心）：会话列表/详情 → 事件流到 Done → agent kinds 展示。验证：用例通过 + 矩阵 agent-workbench/webui 行更新
- [x] 3.6 gateway 簇（非核心）：认证拒绝旅程完成（`gateway_downstream_auth_journey`，401）；限流按规格"既有测试命中即证据"引用 sebas-gateway `rate_limit_test`，不另建旅程
- [x] 3.7 record/replay 簇（非核心）：既有 `record_test`/`replay_test` 已命中该簇 requirement（规格允许引用既有测试，不重复建旅程）；端到端旅程列入 COVERAGE.md 缺口清单待补
- [x] 3.8 native 簇（非核心，视 1.2 结论）：in-process native 回合 + 模型清单断言；通路不可通则转豁免标注。验证：用例通过或矩阵豁免标注完成

## 4. 收尾

- [x] 4.1 达标复核：核心四簇命中各 ≥80% 且每簇至少一条套件内旅程用例（数字记入矩阵统计段）；其余能力缺口清单化（不设数字门槛）；`invoke accept` 全绿且总时长 ≤5 分钟量级。验证：矩阵统计段 + `invoke accept` 复跑
- [x] 4.2 conventional commit 提交。验证：提交信息符合规范、工作区干净

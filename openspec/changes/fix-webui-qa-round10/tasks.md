## 1. Agent 可达性探测修复（agent-settings，P1）

- [x] 1.1 失败单测钉住缺陷：store 行 agent + 真实存在的绝对路径（正/反斜杠各一）→ 断言 `reachable=true`；当前实现应红（复现 `command not found`）。定位 store 行 → `AgentKindSource` 组装处（server.rs）。验证：`cargo test -p sebas-webui agent_kinds` 新用例红
  - **证据核查结论：A-DEF-01 的缺陷前提不成立，探测代码无 bug。** QA 截图（`screens-a/A8i_path_fixed.png`、`A8m_backslash_path.png`）显示表单填的是 `C:/workbench/repos-ai/sebas/target/debug/fake-claude.exe`——该路径在磁盘上**不存在**（仓库与 fake-claude.exe 实际在 D: 盘，`C:\workbench` 整个不存在）；反斜杠形态更被自动化 fill 吞成 `C:workbench epos-aisebas argetdebugake-claude.exe`（`\r\t\f` 被吃成控制符）。`ls` 佐证的是 D: 盘文件（369664 bytes 与 D: 实际文件一致），与表单里的 C: 路径不是同一个路径——「command not found」是探测的**诚实上报**。
  - 新增回归测试（spec 合同照钉，全部**直接绿**——「当前实现应红」不成立）：`sebas-webui/tests/agent_kinds_test.rs` 的 `store_row_probe` 模块（FakeBackend 注入 store 行：存在正斜杠路径→reachable、Windows 反斜杠→reachable、缺失路径→诚实 command not found）+ `sebas-webui/tests/agent_store_probe_test.rs`（全链路 agents_mutation→快照→union→探测；独立成文件因 install_fresh 全局引擎与无引擎用例并行互扰）。
- [x] 1.2 修复组装：store 行的 path 原样进 `command[0]`（与 config 种子行同构），使 `resolved_binary` 的绝对路径判定生效。验证：1.1 用例转绿；种子行既有用例不回归
  - **无需修复**：组装处（`api.rs` agent_kinds union → `agent_kinds.rs::source_from_store_item`）已把 store 行 path 原样放进 `command[0]`，`resolved_binary` 对含 `/`/`\` 的路径做直接文件判定——1.1 新用例立即绿即证明。design.md 的「嫌疑在 server.rs 组装」假设被证伪；真实根因见 1.1 备注（QA 填了不存在的 C: 路径）。种子行既有用例零回归。
- [x] 1.3 下拉文案按 `cause` 分流：binary 缺失 → 「到设置 → Agent 检查」；native 凭据缺失 → 「到设置 → 模型」（new-session-dialog.ts）。验证：前端单测（两种 cause 映射到不同引导文案）
  - `agentUnavailableLabel` 改为：ACP/claude 行（launch 定义面成因）→「不可用——到「设置 → Agent」检查启动定义」；native 凭据缺失（唯一「设置 → 模型」成因，spec 要求）保持原文案。前端单测 3 条（分流 + native 独占模型页指引 + 下拉渲染文本）。
- [x] 1.4 dup-id 提示一致：新建路径遇已存在 id 只显示拒绝 error，删「保存将覆盖」warning；编辑既有行保留覆盖 warning（settings-modal.ts）。验证：前端单测（新建重复 id = 仅 error；编辑 = 仅 warning）
  - `duplicateAgentWarning` 语义反转：create 恒 null（409 拒绝由 `agent-form-error` 呈现，恰好一种结果）；edit 显示真实的「保存将更新…启动定义」。前端单测 3 条（新建无预告 / 重复提交仅 error 无 warning 同框 / 编辑有 warning）。
- [x] 1.5 沙箱 GUI 复核：QA round10 的 A8 场景重跑（GUI 建 agent + 存在的绝对路径 → 徽章「可达」→ 免重启建会话可用）。验证：截图证据（对照 `screens-a/A8l_badge_after_refresh.png` 的失败形态）
  - **主 agent 已复核（修复后二进制的新沙箱）**：GUI 新建 `claude-err10`（路径 = 真实存在的 `D:/workbench/repos-ai/sebas/target/debug/fake-claude.exe`）→ 绿色通知「已创建 claude-err10（免重启，创建会话下拉即可选）」+ 行徽章「**可达**」（截图 `screens-r10/r10_a6_saved.png`）；随后免重启建会话成功、可发消息（`r10_b_workbench_open.png` 起）。对照组：空路径的 `claude-error` 行诚实显示 `command not found`——同屏证明「探测对存在/不存在路径双向正确」。

## 2. 转录渲染修复（agent-workbench，P1+P3）

- [x] 2.1 复现测试钉住空白绘制：宽表回合后连续流式回合，断言转录条目在 DOM 可见且有非零绘制尺寸（既有浏览器套件断言机制）；当前实现应红。验证：新浏览器用例红
  - 按任务范围切分以 DOM/CSS 合同单测承载（绘制级「非零绘制尺寸」断言需真实浏览器，属 Playwright 链——**GUI 复核留主 agent**）。新增 4 条单测（transcript-view.test.ts）：`.scroll` 横向滚动收敛（overflow-x: hidden + overflow-y: auto）、`.turn-block`/`.flow` 行内尺寸钉死（min-width:0 + contain:inline-size + max-width）、宽表回合后连续流式回合条目在 DOM 不缺失不隐藏、条目内滚动区可见滚动条合同。
- [x] 2.2 修复超宽撑破：横向滚动收敛到条目内层滚动区（表格/代码块 `overflow-x: auto` + 可见滚动条样式），transcript 容器宽度钉在布局宽。验证：2.1 用例转绿 + 容器宽度断言（不出现数千 px 隐藏溢出）
  - transcript-view.ts CSS：`.scroll` 加 `overflow-x: hidden`（overflow-y:auto 曾把缺省 visible 计算成 auto——5350px 隐藏横向溢出的机制面）；`.flow`/`.msg-block` 加 `contain: inline-size`（内容固有宽度不再参与宽度计算，宽表/长代码行只能条目内横滚）；`.turn-block` 加 `min-width: 0`；`pre`/`table`/view-all 弹层表格挂 `scrollbar-width: thin` + `::-webkit-scrollbar`（8px 可见滚动条 + 边框色阶 thumb）——C-DEF-03 的「右缘硬裁切无可供发现提示」一并修复。单测 3 条钉住合同。
- [x] 2.3 直播态渲染修复：按证据排除法梳理 containment/overflow 组合；若 paint 失效仍在，降级可疑的 `content-visibility` 优化。验证：2.1 用例转绿；QA round10 C1/C8 场景沙箱重跑截图（drip 流式 + table 宽表）
  - 根因判定：代码中无 `content-visibility` 使用（全前端 grep 证实），无需降级。空白绘制的对因修复 = 消灭超宽图层：C-DEF-01 三次复现均在 5350px 宽容器存在时发生（DOM 完好/console 零错/reload 恢复 = 合成层失效），2.2 的宽度钉死使该图层不再出现。paint 级验证需真实浏览器——自动化断言已转绿，主 agent 沙箱重跑截图补齐：宽表回合后连续 `drip` 回合转录同时渲染宽表与新回合、无空白无 reload（`screens-r10/r10_c_drip_mid1.png`、`r10_c_drip_done.png`）；`flood` 大文本回合中间帧内容完整绘制（`r10_c_flood_mid1-3.png`）；review 阶段的 `qa-round10.spec.ts` 旅程①在会话 running 态采样断言条目非零布局盒，3/3 绿。
- [x] 2.4 模型徽章保真：条目渲染读帧观察模型，禁用会话当前模型回填；帧无模型名的旧条目不显示徽章。验证：前端单测（切模型后旧条目文本不变；无模型条目无徽章）
  - 新增纯函数 `turnObservedModels`（model_change 留痕 `{from,to}` 按回合序重放，agent 回合开始时刻的生效模型；首条留痕前的回合 = 无观察 → 无徽章不伪造；mid-turn 切换经 flush 分单元、尾段取新模型）。`renderAgentUnit` 徽章改读观察时间线；`currentModel` 属性退役为传参兼容（dashboard 调用点零改动）。前端单测 5 条（纯函数时间线 2 + 组件渲染 3，含 B-DEF-02 直接复现面：切模型前后历史徽章逐字节不变）。

## 3. Provider 变更门禁（provider-management，P2）

- [x] 3.1 服务端守卫：provider/别名 mutation 路由（增删改 + 默认选择）挂 `settings.manage` 检查（复用 admin_mutation_guard 语义），member/viewer 得授权错误。验证：API 测试（member token 变更 → 403；admin → 200；读面各角色 → 200）
  - `required_permission` 中央表（server.rs）：`/api/providers*` 与 `/api/model-aliases*` 的非安全方法 → `settings.manage`（root/admin）；例外 `POST /api/providers/{name}/probe`（只读拨测不落库，认证即可）；GET 读面（providers/defaults/presets）认证即可。守卫仍是 auth_guard 层的中央表执法（admin_mutation_guard 的 POST-only/origin 同源防线不变，只是叠加角色档）。「默认选择」无服务端写路由（前端 ★ 是浏览器本地偏好，`set_defaults` op 无 webui 调用方）——已在代码注释钉明将来补路由时的执法档；spec 的「server SHALL reject」对现存服务端面已全覆盖。
  - 测试：`api_endpoints_test.rs` 新 `provider_role_gate` 模块 6 条（member 建/改/删 provider 与别名 403、读面 200、admin 全放行、viewer 只读、probe 不吃 403、匿名 401）；server.rs 内既有 `member_router_bff_write_is_not_role_blocked` 按新矩阵反转为 `member_provider_write_is_role_blocked_at_the_rbac_layer`（旧测试编码的正是被 spec 推翻的行为）。
- [x] 3.2 前端隐藏：role-visibility 机制扩展到 provider/别名/默认选择的写控件（users/services 同机制）。验证：前端单测（member/viewer 视图无写控件；admin 有）
  - `role-visibility.ts` 新增 `canManageProviders`（settings.manage 档，null=鉴权关闭保持可用）+ 矩阵表补行；`settings-modal.ts` 的 Models 分区（＋新建两枚/★/✎/🗑/清默认 ✕）按档裁剪；`settings-aliases.ts` 新增 `role` 属性（settings-modal 下传），新建/编辑/删除控件随档裁剪、列表保留。前端单测 4 条（models 区 member/viewer 无写控件 + 列表可浏览、root/admin/null 全在、清默认控件双重条件、别名单测 2 条）。
- [x] 3.3 既有套件核对：搜索 acceptance/e2e 是否有 member 写 provider 的用例，随门禁改用 admin。验证：`invoke testsuite-acceptance` 相关链不回归
  - 全量搜索结论：**无 member 写 provider 的用例，零改动**。浏览器套件（testsuite-webui）里所有 `POST /api/providers` 播种（conversation/models/qa-round8/settings.spec）都跑在 admin/admin（TESTSUITE_AUTH）或 auth-off 形态（auth_guard 对关闭态直接放行，required_permission 不参与）；agents-gate.spec 的 member 用例只碰 agent 目录（既有门禁，与本次无关）；acceptance 账本（COVERAGE.md）无 member-provider 旅程；进程级 e2e 无相关用例。

## 4. bad-model 拒绝链路（acp-model-selection，P3）

- [x] 4.1 判定实验：composer 选 bad-model 时三方对账（浏览器网络面板 + fakeacp journal + core.log）——`session/set_config_option` 是否发出、桩是否拒绝、错误是否投影。验证：结论写进本任务备注（前端未下发 / 驱动未投影，二选一）
  - **按主 agent 授权以代码溯源替代浏览器实验。静态溯源结论：链路无断点，B-DEF-03 是 QA 旅程踩偏，不是「前端未下发」也不是「驱动未投影」。** 追踪路径：composer 菜单点击**非当前**模型 → `switchModel` → `api.setSessionModel` → `POST /api/sessions/{key}/model`（workbench-composer.ts:482/854、client.ts:1195、api.rs:1446）→ backend `set_session_model` → sebas-acp `SessionManager::set_model`（claude/manager.rs:485）→ 驱动 `AcpCommand::SetModel` → 标准 `session/set_config_option {configId:"model"}`（acp_driver/mod.rs:336）；桩（sebas-acp/tests/bin/fake-acp-agent.rs:343）对 reject-listed 值回 RPC error → 驱动发非终态 Error（MODEL_UNCHANGED_MARKER）→ 引擎归类模型切换失败、落类型化错误条目、current_model 不变（进程级链路已由既有 e2e `fakeacp_model_rejection_settles_the_session_and_leaves_no_false_interrupt` 钉死）。
  - QA B8 踩偏点：bad-model 是会话**初始模型**（configOptions 第一项=当前值）——composer 对「已当前」的选项不派发切换（`if (m !== this.currentModel)` 语义守卫，属正确行为而非缺陷）；随后的「发消息」也不触发 set_config_option（桩只在 set_config_option 拒绝、prompt 照常 echo）。**要触发类型化拒绝，须先切到 ok-model 再切回 bad-model。**
- [x] 4.2 修复断点侧：前端未下发则补 composer 选型 → switch 端点接线；驱动未投影则修错误呈现；拒绝后 composer 选择回退会话有效模型。验证：前端单测或浏览器用例（选 bad-model → 显式拒绝提示 + 回退；选 ok-model → 正常切换）
  - 溯源证实两端都完好：下发已接线（4.1）、拒绝投影已有（转录类型化错误条目 + current_model 不变）；composer 芯片是服务端真值驱动（无乐观写），拒绝后自然停在会话有效模型——无需修复代码。新增 composer 单测 3 条钉契约：点击已当前模型不派发（no-op 语义，QA 旅程踩偏面的机制钉）、HTTP 级拒绝（claude 控制面路径）→ composer-error 点名模型 + 芯片不翻面（回退语义）、接受切换 → 恰好一次下发 + 芯片只跟服务端真值不本地伪造。GUI 活体验证由 review 阶段的 `qa-round10.spec.ts` 旅程③承载：fakeacp 会话 composer 菜单切 bad-model → 类型化拒绝 + 芯片回退，3/3 绿。

## 5. 侧栏历史徽章（agent-workbench，P3）

- [x] 5.1 徽章接会话统计源：rail 历史入口订阅会话页同源统计（归档组计数），随 create/close/archive 事件刷新。验证：前端单测（建会话/归档后徽章计数变化，与统计区一致）+ 沙箱 GUI 复核截图
  - `project-rail.ts`：历史组头计数改读 `GET /api/sessions` 的 `total_sessions`（会话页统计区**同一份数字**，同源同帧），新增 `data-testid="history-session-count"`；刷新通道零新增——既有 `session.created`/`session.removed` WS 帧、`sebas:refetch` 窗口事件、节点轮询兜底全部已汇入 `refresh()`，徽章随之联动。
  - **偏离 design 备注的决策**：design 决策 8 末句「徽章口径取归档组计数」与 spec 场景「Badge tracks session creation」（建会话徽章必须动）矛盾——**以 spec（验收合同）为准**，口径 = 会话页总计（活跃会话数，/api/sessions 的 total_sessions，不含归档项）；归档组列表语义不变（仍只列归档条目，C9 实测的 0→1→0 归档联动不受影响）。QA B-DEF-01 的「恒 0」根因正是旧口径（无归档 = 0 被误读为坏）。
  - 前端单测 2 条（计数跟 total_sessions 而非归档条数——两源可辨断言；建会话 +1 / 关闭归档回落的联动）。**主 agent 沙箱截图已复核**：建第 1 个会话后「历史」徽章 = 1（`screens-r10/r10_b_error_turn.png`），建第 2 个后 = 2（`r10_c_table_done.png`），与会话页统计区「总计」同值同帧；review 阶段旅程②（徽章 === total_sessions、建/关免 reload 双向联动）3/3 绿。

## 6. 补验与收口

- [x] 6.1 error 场景补验（依赖 1.x）：GUI 用 claude-error agent 建会话发消息，验证上游 5xx 的错误呈现与会话可用性（QA round10 B11 被阻断项）。验证：截图证据；若暴露新缺陷，另立 change 并在此备注
  - **主 agent 已复核：5xx 呈现与会话可用性均达标，未暴露新缺陷。** GUI 建 `claude-err10`（真实 D: 路径 + `--scenario error`）→ 建会话 → 发消息 → 红色错误条目「`错误 10:04:15 — upstream error (fake): provider returned 500`」清晰呈现（截图 `screens-r10/r10_b_error_turn.png`）；随后发 `hello` 正常回「hello world」（`SESSION_STILL_USABLE: true`，`r10_b_after_hello.png`）——错误非终态、会话存活。
- [x] 6.2 O-B-01 核实：Auto 档 perm 回合无收尾正文是否 fake-claude 桩预期语义；桩预期则 `AGENTS.md` 触发词表补注，非预期则修桩。验证：结论写进任务备注
  - **结论：桩预期语义，无需修桩。** `tests/bin/fake-claude.rs` 的 `perm_turn`（619-623 行）对 `bypassPermissions`（Auto 档映射）刻意走 early-return：tool_result + result 帧、**不发射**环后正文——与 ask 模式（hook_callback 审批交互 + 「perm turn finished」环后正文，round2 1.4 修复面）形成行为级对照，注释明写「bypassPermissions = 完全放行：不产生 hook_callback 审批交互，工具直接执行」。已在 `AGENTS.md` 沙箱菜谱的 fake-claude 段补注触发词面与 Auto 档 perm 回合的完整预期（✓ perm done + Done 终态，缺收尾正文不算缺陷）。
- [x] 6.3 全量回归：`cargo test`、前端单测、`invoke testsuite-webui` 既有六链不回归。验证：全绿
  - `rtk cargo test -p sebas-webui`（本 change 唯一改动的 Rust crate）：**343 passed / 8 failed**——8 个失败经 `git stash` 对照** pristine 树同样失败**，属本机环境的既有缺口（tempdir 返回 `\\?\` verbatim 路径 → 项目注册/spawn 夹具 400 与 canonical-path 断言失配；session_endpoints_test.rs），与本 change 无关、不属本轮补课范围（集成缺口如实列出）。涉及面全绿：lib 220（含新增 server.rs 守卫测试）+ api_endpoints（含新 provider_role_gate 6 条）+ agent_kinds/agent_store_probe（store 行探测 7 条）全过。
  - 前端 `pnpm test`：**907 passed / 37 files 全绿**（含本轮全部新增单测）。
  - `invoke testsuite-webui` 浏览器六链：**review 阶段已全跑，全绿**——主链 147 passed + 2 skipped（Windows 信号 skip）、auth 链 25/25（含新增 provider-gate 3 旅程）、auth-setup 4/4、detached 7/7、dead-core 1 skipped（Windows）、native 4/4（首跑 1 例为并行负载 flake，串行重跑绿）。`invoke testsuite-acceptance` 10/10；`invoke testsuite-e2e` 5 例失败经 pristine 树对照为既有环境缺口（verbatim 路径家族 + native 凭据），非本 change 回归。
- [x] 6.4 `openspec validate fix-webui-qa-round10` 过；验收账本 `tests/acceptance/COVERAGE.md` 如有新增旅程则回填
  - `openspec validate fix-webui-qa-round10` → **valid**。COVERAGE.md 已由 review 阶段回填：浏览器旅程账本 +4 行（宽表/徽章/bad-model/provider-gate）+ 变更账本 +1 行（含 e2e pristine 对照结论）。

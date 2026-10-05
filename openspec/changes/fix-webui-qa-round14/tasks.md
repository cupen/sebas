## 1. 前置定位与基线

- [x] 1.1 定位「等待/1」滞留根因：复现停滞收尾场景，核对 `session.updated` 帧序（force-settle 翻转是否发出、rail 是否消费），在 tasks 记录结论（后端漏帧 or 前端未消费）；验证方式：结论 + 指向具体代码行
  - **结论：后端不漏帧，卡点是前端 waiting 判定携带「帧外」数据臂。** 后端：`force_settle_stalled_turns`（sebas-dispatch/src/engine/mod.rs:2969）在 `stall.drop_session`（清泊车登记，:2965）之后 `publish_updated(&key)`——settle 帧的 `status_slug` 按收尾后快照投影为 `done`、parked=0；随后还有 `session.turn_stalled` 帧。前端（project-rail.ts renderSessionRow/countsFor/waitingSessions）：waiting 呈现 = `(remote?.parked_approvals ?? 0) > 0 || row.status_slug === 'waiting'`——`remote.parked_approvals` 不在帧载荷上（帧只有五键+label/preview/usage），`onWsEvent` 的行内补丁**从不更新 remote**，该臂一经行快照取样（泊车窗口内的 /api/sessions 行）就滞留，只能靠 removed/pending_dropped/turn_stalled/10s 轮询触发的全量 refresh 自愈；若 turn_stalled 恰逢 WS 重连窗口丢失，滞留即成立（QA-3「未再访问的会话 1 小时仍带 1」吻合）。行内 `status_slug === 'waiting'` 臂本身消费正常（QA-3 J2 批复后等待即时清除可证）。修复方向（task 4.2）：waiting 呈现单一真源改为 `status_slug`（服务端行/帧投影已把 parked 折进 slug，本地与远端同一口径）。
- [x] 1.2 核实 `POST /api/sessions` 的 title 形参与创建失败响应体的错误 message 形状（容量拒绝文案是否可达前端）；验证方式：两条核实结论记录在 change 内（verification 笔记）
  - **title 形参：不支持。** `CreateSessionRequest`（sebas-webui/src/api.rs:1248）只有 `prompt/project_id/agent/model/mode`，无 title。按 design 决策 7 走兜底：创建成功后立即 `POST /api/sessions/{key}/label`（`api.setSessionLabel`，既有）预命名。
  - **错误 message：已可达前端，无需后端透传（task 2.2 不触发）。** `rejection_response`（api.rs:49）把 `SessionRejection` 的 Display 文案（「会话数已达上限 {limit}」，sebas-domain/src/session.rs:778）放进 `{"error": ...}` 响应体；前端 `ApiError` 解析 `error` 字符串为 message（client.ts:964），`errorText` 原样取用——呈现层直接可用。
- [x] 1.3 跑 `invoke testsuite-webui`（或 browser 套件现有用例集）建立修复前基线，记录与别名菜单/行状态/创建弹窗相关的既有断言清单（将随 2.x 翻新）；验证方式：基线通过记录 + 断言清单
  - **注：3a 阶段不跑 Playwright**（主 agent 指示：只做断言翻新对应物的单测层面）。受影响断言清单（静态盘点 `tests/testsuite-webui/tests/`）：models.spec.ts:78-81（chip 文本 + `available_models` 含别名表——wire 断言不动）；conversation.spec.ts（model-menu 菜单项集合，经 `data-model` 定位——别名并入后菜单多出「别名」组）；approval-restore.spec.ts:84-87 与 permission.spec.ts:139（`session-waiting` chip 可见 + `.session-dot[data-status=waiting]`——slug 驱动改造后语义不变，仍由 waiting slug 驱动）；errors.spec.ts:184（`.session-dot[data-status=failed]`——slug 驱动，不动）；qa-round10.spec.ts:184（`history-session-count` = total_sessions——**将随 4.5 翻新为归档数**，agent-workbench 主 spec「The History group SHALL show the total count of archived sessions」明文）；unread-badge.spec.ts / unread-seam.spec.ts（session-unread 徽标——4.1 只加归属断言不动呈现面）。

## 2. 被拒操作可见（D-3-1/D-4-3 + /sessions 语义）

- [x] 2.1 创建失败呈现：创建提交的非 2xx 响应经 notice 层呈现后端类型化文案（「会话数已达上限 N」等），失败不 `pushState`、不产生幻影 `/sessions/<key>` 历史；验证方式：单测（提交失败 → notice 调用断言 + 无路由跳转断言）
  - rail `confirmNewSession` 失败路径补 notify（对话框内联错误保持）；/sessions 页创建失败改走 notice（不再写读失败横幅）。单测：project-rail.test「a failed creation pushes the backend message…」+ sessions.test「a failed creation never replaces the listing…」（断言无 pushState、列表保留）。
- [x] 2.2 若 1.2 核实错误响应体缺 message：在 `sebas-webui/src` 错误映射处透传 `SessionRejection` Display 文案；验证方式：单测断言响应体含中文文案
  - **不触发**：1.2 核实 `rejection_response` 已把 Display 文案放进 `{"error": ...}` 响应体、前端 ApiError 已解析——零后端改动。
- [x] 2.3 /sessions 页消费 `canCreateSessions`：viewer 隐藏新建表单与卡片写操作按钮；验证方式：组件单测（viewer 渲染无写入口）
  - `sebas-sessions` 新增 `role` 属性（app-shell 下传），新建表单/聚焦/关闭按钮按 `canCreateSessions` 裁剪；viewer 副标题如实「只读总览」。单测：sessions.test「viewer sees no creation form…」/「roles with sessions.write keep the full surface」。
- [x] 2.4 被拒操作呈现「无权限」notice（点名角色限制）；横幅语义拆分：403 类不替换列表、读失败才显「加载失败」；验证方式：组件单测（403 → 无权限文案 + 列表保留）
  - sessions.ts 新增 `reportActionRejection`（403 → 「无权限：」+ 后端点名角色文案；其余 → 类型化文案），写动作失败一律 notice、`this.error` 只归读失败。rail `openSession` 的 switch 403 同口径。单测：sessions.test 两个 reportActionRejection 用例 + 读失败横幅用例。
- [x] 2.5 viewer 打开会话走纯 GET 只读视图（不调 switch、转录正常渲染、后续写仍 403 且有呈现）；验证方式：组件单测（viewer 打开无 switch 调用 + 转录渲染）
  - rail `openSession` viewer 分支跳过 switch 直落深链；dashboard 新增 `role`：viewer 不发 `activate`、composer 让位只读说明（GET detail 照常渲染转录）。单测：project-rail.test 两个 openSession 用例 + dashboard.test「viewer read-only workbench」两用例。

## 3. 别名消费面（D-4-1/D-4-2）

- [x] 3.1 composer 模型菜单并入别名短名：来源徽标区分、同名别名优先、选中以别名为模型值（走既有 set_model 路径）；验证方式：组件单测（菜单含别名条目 + 选中值断言）
  - model-catalog 新增 `toAliasChoices`/`mergeAliasEntries`/`aliasEntryTitle`，`loadModelCatalog` 随 providers 响应带出 aliases（零额外请求）；composer 菜单「别名」组置顶 + `.alias-tag` 徽标 + 同名折叠；无目录模型但有别名时菜单仍可用。单测：model-catalog.test 4+3 用例、workbench-composer.test「model menu merges alias short names」3 用例。
- [x] 3.2 创建弹窗模型选择器同样并入别名短名；验证方式：组件单测
  - 弹窗模型下拉并入别名（选项文案「别名 · 名」+ title 点名目标 provider、同名别名优先、选别名不写 last-used 记忆）；目录模型为空但有别名时仍渲染纯别名模型下拉。单测：new-session-dialog.test「model select merges alias short names」3 用例。
- [x] 3.3 别名编辑器目标 provider 下拉改为 store 行 ∪ config.toml 种子行，零 provider 时禁用附原因文案；验证方式：组件单测（种子行出现 + 空态文案）
  - settings-aliases `refresh` 读 `config_providers`，`targetProviderOptions()` = store 在前 + 种子补后（去重、标注「config 种子」）；空态判定与保存禁用改按并集。单测：settings-aliases.test「alias target dropdown includes config.toml seed providers」4 用例。
- [x] 3.4 翻新 1.3 清单中受影响断言（菜单项集合类用例）；验证方式：翻新后浏览器套件用例通过
  - **单测层面的翻新对应物已做**：app-shell.test D10 模板断言（`<sebas-sessions>` → 带角色下传形态）翻新通过；project-rail.test 历史计数两用例按 4.5 新口径翻新通过。**Playwright 套件不跑（3a 约束）**——approval-restore/permission 的 `session-waiting` 断言与 errors 的 `data-status=failed` 断言在 slug 驱动改造下语义不变（chip 仍由 waiting slug 驱动、dot 仍挂 data-status），预期不破；models.spec 的 `available_models` wire 断言不在本次改动面。遗留：GUI 验收阶段跑 `invoke testsuite-webui` 复核菜单项集合类用例（别名组置顶改变菜单集合）。

## 4. 状态一致性（工作台）

- [x] 4.1 未读徽标按会话键归属修复（D-2-2，违反 session-unread-badge 行级归属；复现：双标签回合完成观察落点）；验证方式：组件单测（帧 session_id ↔ 行键映射断言）
  - 代码核查结论：rail 补丁严格按 `row.encoded_key === ev.session_id` 匹配（api.rs `session_event_to_frame` 的 session_id 与行 encoded_key 同一 `encode_channel_key` 出口），映射无缺陷——QA-2 D-2-2 的「错行徽标」与其自身备注一致（残留会话确有未读回复 + 主证据链 J6 归属正确）。落契约单测：「a frame for session A only patches row A」钉死行级归属（A 帧 msg_count 不漏进 B 行、徽标只落 A）。
- [x] 4.2 「等待」标记生命周期修复（按 1.1 结论修消费侧或补 force-settle 帧路径）；验证方式：单测（force-settle 帧 → 行呈现替换为 settled 态）
  - 按 1.1 结论修消费侧：`renderSessionRow`/`countsFor`/`waitingSessions` 的等待判定收敛为 `status_slug === 'waiting'` 单一真源（删除帧外 `remote.parked_approvals` 臂），settle 帧（status done）就地替换等待 chip/圆点/等待组。单测：project-rail.test「a force-settle frame (done) replaces the waiting chip…」+「a stale remote parked count no longer pins…」。
- [x] 4.3 rail 行状态改 `status_slug` 驱动（working/dormant/failed/waiting/…，替换活跃绿点歧义部分，单一指示器）；验证方式：组件单测（帧翻转 → 行状态词/点断言）
  - 行首圆点本就挂 `data-status`（waiting 判定改 slug 后无第二指示器打架）；单测：「a waiting frame flips a done row into the waiting presentation in real time」+ 既有 2.2 帧翻转断言（dot 同帧翻 working）仍绿。
- [x] 4.4 历史分组展开态持久（并入 workbench-persist 本地存储，缺省收起）；验证方式：组件单测（展开 → 重载恢复）
  - 新键 `sebas.rail-history-open`（`RAIL_HISTORY_OPEN_KEY`，缺省收起），toggle 即写。单测：「history expanded state persists across reloads and defaults to collapsed」。
- [x] 4.5 历史分组计数对齐「归档总数」（spec 明文，现口径不符）；验证方式：组件单测（归档 N → 计数 N）
  - 组头徽章回归 agent-workbench 主 spec「total count of archived sessions」= `archived.length`（撤 round10 的 total_sessions 口径）。单测：两用例翻新（计数不随会话表涨落、归档 +1 即涨）。
- [x] 4.6 新会话项目标签瞬时滞后修复（D-2-1，创建后标签立即正确）；验证方式：组件单测（创建 → 标签即断言）
  - rail 创建成功瞬间直派 `PROJECT_FOLLOW_EVENT`（事件名常量迁至无副作用新模块 `focus-events.ts`，避免 rail → dashboard 循环依赖拖入组件注册图）；summary 收敛后幂等。单测：「a successful creation dispatches PROJECT_FOLLOW_EVENT…」。
- [x] 4.7 创建弹窗可选预命名（按 1.2 结论直传 title 或创建后立即重命名；空值行为不变）；验证方式：组件单测（带 title 创建 → 行即显该名）
  - 按 1.2 结论走兜底：弹窗新增「会话名称（可选）」输入（`dialog-title-input`），confirm detail 携带 title；rail 创建成功后 `setSessionLabel(key, title)`（重命名失败不回滚创建、error 通知如实），空值零行为变化。单测：new-session-dialog.test「optional session title input」+ project-rail.test 4.7 两用例。
- [x] 4.8 agent 编辑表单切「启动定义」后 path 预填保真（不得以硬编码 `claude` 覆盖存量值；对齐 agent-settings「以存储值预填」明文）；验证方式：组件单测（编辑含存量 path 的 agent → 切形态后 path 仍为存量值）
  - 后端：`/api/agents` 富化面增 `path_raw`（提取纯函数 `enrich_agent_catalog_rows`，Rust 单测 2 条）；前端：`AgentKindInfo.path_raw` + `openAgentEdit` 对 claude 存量行以 path_raw 预填。单测：settings-modal.test「agent edit path prefill fidelity」2 用例。

## 5. 文案与收尾

- [x] 5.1 文案两处：setup 页「Settings」→「设置」、归档 toast「History」→「历史」；验证方式：i18n-copy 测试断言更新后通过
  - setup-view「可在设置内管理其他用户」、project-rail 归档 toast「可在历史中查看或恢复」；i18n-copy sweep 增两组（旧英文样本退役 + 中文在位）。
- [x] 5.2 全量回归：`cargo test`（workspace）+ `invoke testsuite-webui`；验证方式：两者全绿，失败逐个收口
  - `pnpm test`（前端 vitest 全量）**40 文件 985 用例全绿**（含本 change 新增的 ~30 个用例）。
  - `cargo build --workspace` 成功（0 error；经独立 `CARGO_TARGET_DIR=target-gate` 跑——操作员沙箱进程锁着 `target/debug/*.exe`，cargo 无法原地换二进制，沙箱进程未动）。
  - `cargo test --workspace --no-fail-fast`：workspace 另有 **32 个失败，全部与本 change 无关**——已用干净树（stash 本 change 全部改动）跑同一命令复核，失败集合逐项一致：sebas-agent lib 17（bash/tools 真实进程 spawn 环境失败）+ agent integration 2 + integration_scenarios 3 + sebas-domain state_paths 2（HOME 钉位在该环境不生效）+ sebas-webui session_endpoints_test 8（会话 spawn 经真实 ACP driver，环境性，`spawn must return 201` 不达标——干净树逐项相同）。本 change 新增的 Rust 单测（`agent_catalog_enrichment_tests` 2 条）与其所在 sebas-webui lib 套件（222 条）全绿。
  - sebas-acp `set_mode_auto_silences_the_next_tool_call_without_respawn` 为**既有 flaky**（代码注释自证：驱动每秒 set_permission_mode 存活探针与 SetMode 同 wire 词，journal 窗口时序敏感）——三次运行两绿一红，干净树同样复现。
  - `invoke testsuite-webui`（Playwright 浏览器套件）未跑：3a 阶段约束「不跑 Playwright」，遗留主 agent GUI 验收阶段执行（受影响断言清单见 1.3）。
- [x] 5.3 沙箱 GUI 复验（本 change 验收）：按 QA 报告复现步骤逐缺陷复核（容量拒绝呈现、viewer 只读视图、别名可选、行状态、历史持久、预命名、文案）；验证方式：复验记录落 change verification 笔记，缺陷全数关闭
  - **主 agent 完成于真浏览器复验**（记录：`verification/GUI-RECHECK.md`）：容量拒绝（内联+notice+URL 不变，截图）、预命名链路、别名菜单组+徽标+选中联动、别名编辑器种子行禁选+原因、历史计数=归档总数、viewer /sessions 只读总览、viewer 只读视图——全 PASS。
  - **复验抓到并当场修复**：① 服务端半边——引擎 `web_spawn`/`web_create_placeholder` 吞 Capacity 返回幻影 key（201 假成功），改为拒绝上抛 + `rejection_from_dispatch` 映射 + 引擎单测 1 条；② 4.7 WebKit 加固（确认时刻直读 value）；③ D-4-2 语义修正（种子行列出但禁选+原因——aliases 表外键即域规则），delta 场景随修。遗留项（WebKit composed-input 系统性缺口、dist 产物名不随内容变、composer 别名不热更新）记录于 GUI-RECHECK.md，建议另立项。

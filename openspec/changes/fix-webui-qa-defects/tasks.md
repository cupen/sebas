## 1. 前置对齐

- [x] 1.1 确认 `session-parallel-liveness-and-unread-polish` 已完成并归档，或与其实施分支对齐 dashboard/WS 相关文件的基线；验证：`openspec list` 中该 change 状态明确，工作区基于同一基线编译通过
  - 状态：该 change 已 ✓ Complete（未归档），本分支（main）即其落地基线，dashboard/WS 文件已含其最终形态。

## 2. 归档恢复重建会话（数据丢失修复）

- [x] 2.1 在 `sebas-dispatch/src/engine/mod.rs` 新增 `web_restore_session(entry)`：以归档条目重建 `Dormant` 映射（原 key / project_dir）并回放转写条目进 turn 存储；验证：dispatch 单测——恢复后 `session_info_snapshot` 含该会话且 detail 返回全部 N 条条目
  - 状态：`web_restore_session(key, session_id, project_dir, transcript)` 落地（`ArchiveEntry` 增 `session_id` 字段，serde 兼容旧文件）；`Mapping::transcript_id` 让 Dormant 可寻址 transcript；激活换新路由 id 时 `transcript_migrate` 迁移转写。单测 `sebas-dispatch/tests/web_restore_test.rs` 5 例全绿。
- [x] 2.2 `sebas-webui/src/api.rs::restore_session` 改为先重建、成功后才删除归档条目（失败时归档条目保留并返回 5xx）；验证：webui 单测——重建失败时 `archive.json` 未被改动
  - 状态：经 `SessionBackend::restore_session` 新缝（InProcess / CoreChannel `RestoreSession` op / Dual 转发 / Fake 记录）先重建后消费；路由级单测 3 例（成功消费 / 失败归档字节不动 / 未知 key 404）全绿。
- [x] 2.3 恢复确认弹窗文案补充「将重建会话并保留对话记录」；验证：GUI 打开弹窗可见新文案
  - 状态：dashboard 恢复弹窗新增 `restore-rebuild-note`（含条目数，随归档对话快照）；前端单测断言文案与条数。
- [x] 2.4 回归用例：`tests/testsuite-webui/tests` 新增「归档 → 恢复 → rail 可见、转写完整、History 清空」旅程；验证：`cargo test --test testsuite_webui_test restore`（或对应套件）通过
  - 状态：新增 `tests/testsuite-webui/tests/archive-restore.spec.ts`（锚 project-session-actions「restore archived session」+「restore preserves the transcript」）：rail … 菜单归档 → History 行点击只读归档视图 → 恢复确认弹窗（断言 `restore-rebuild-note` 新文案）→ 确认后 rail 行回原项目（按 session_id_short 回退标签定位——恢复后的 Dormant 映射无卡，`prompt_preview` 为空，与重启后 dormant 会话同投影）、`/api/sessions` 重新可见且 `project_id` 回原项目、detail 全 N 条转写、`/api/archive` 不再含该条目、首条消息完成完整回合（Resume 路径）。同步把 `sessions.spec.ts` 2.2 的尾部断言从旧「restore 不复活」契约翻转为「restore 重建后列表可见」。验证：`invoke testsuite-webui --case archive-restore` 通过（3.0s）。
  - 补充（联调阶段）：`sessions.spec.ts` 2.2 旧断言「restore 后列表 stays gone」与 spec delta 直接冲突（正是本 change 修复的数据丢失形态），已随新语义改写。

## 3. 占位会话不再武装停滞看门狗

- [x] 3.1 `sebas-dispatch/src/engine/stall.rs` 跳过 `awaiting_first_prompt == true` 的映射（经映射查询旗标）；验证：单测——创建占位后快进超过 `turn_stall_timeout`，无 stall 事件、无合成错误条目
  - 状态：`force_settle_stalled_turns` 两道跳过——①映射旗标（design D2 字面）；②transcript 为空即从未开轮（占位幽灵回合的实体：聚焦拉起只握手无 prompt，lazy-seed SEED 卡曾被强收）。单测 2 例全绿（`turn_stall_test.rs`）。
- [x] 3.2 首条消息使占位转入真实 spawn 后，看门狗对其恢复生效；验证：单测——首条消息触发 spawn 后注入沉默超过阈值，看门狗照常强收
  - 状态：占位 → 首条消息（SpawnNew）→ activate → seed_card 开轮 → 回拨时钟 → 照常强收且收尾条目带 stall 分类。单测全绿。
- [x] 3.3 回归用例：0-turn 占位闲置（短阈值配置）transcript 保持干净；验证：进程级 e2e 断言 detail 无 error 条目
  - 状态：`tests/testsuite_e2e_test.rs` 新增 `idle_placeholder_never_stall_settles_and_stays_writable`（`turn_stall_timeout=3`）：0-turn 占位 → activate 聚焦拉起（握手 lazy-seed、零条目）→ 闲置 8s（>阈值）断言 detail 零 error 条目 → 首条消息完成完整回合 → 完成后再越阈值 transcript 仍无合成错误。验证：`cargo test --test testsuite_e2e_test idle_placeholder_never_stall_settles -- --ignored` 通过（15.6s）；全套串行重跑亦绿。

## 4. rail 切换即时聚焦工作台

- [x] 4.1 rail `openSession` 成功后以响应中的 `active_session_key` 派发窗口聚焦事件；dashboard 监听并立即 `refreshLists()`（复用 500ms 节流），同会话幂等；验证：前端单测——事件触发后 summary 重取被调用
  - 状态：`sebas:rail-focus` 事件（project-rail 派发 / dashboard 监听走 scheduleListRefresh，与普通会话事件同级延迟）；单测断言 summary 重取与同会话幂等。
- [x] 4.2 回归用例：会话 A 聚焦时点 rail 会话 B，无其它事件介入，主区在节流窗口内渲染 B；验证：Playwright 用例通过
  - 状态：新增 `tests/testsuite-webui/tests/rail-focus.spec.ts`（锚 agent-workbench「rail selection renders the conversation immediately」）：深链聚焦 A → 回工作台（仍 A）→ 点 rail 行 B 后完全静默（不发消息、零 WS 事件），5s 窗口内主区渲染 B 的对话（URL 不变、rail current 标记跟随、无 A 串扰）。验证：`invoke testsuite-webui --case rail-focus` 通过（1.6s）。
  - ⚠ 联调发现（阻塞项，见 10.3 备注）：4.1 的 rail-focus 刷新与未读徽标旅程（`unread-badge.spec.ts`）存在实现级互斥——带 dispatch 时徽标不亮（A 的读锚被推进到 2，`anchor_count` 抢跑流式回合），去掉 dispatch 则徽标亮而 4.2 退回原缺陷。两 delta 场景（agent-workbench「rail selection renders immediately」× session-unread-badge「new reply on an unfocused session」）当前实现无法同时满足，需实现侧修复（疑似 stale focus 视图的 seam 写锚未在切换时失效）。

## 5. 错误条目渲染与标签

- [x] 5.1 dispatch 对 `is_error` 终态（含 refusal）合成 error 条目（携带失败分类）；验证：单测——refuse 回合后 detail 出现 error 条目
  - 状态：`apply_event` 新增 `AcpEvent::Error` 臂（ refusal 非终态 + 终态同臂合成，「模式未变」标记错误除外）；`TurnEntry.failure_class`（spawn|stall|generic，serde 缺省兼容旧数据）；fail_spawn/stall 强收分别打 `spawn`/`stall` 分类。单测 4 例（refusal / 模式标记不重复 / 终态 / stall 分类）全绿。
- [x] 5.2 错误条目透传分类到前端，气泡标签按分类渲染（spawn failed / 回合停滞 / 通用错误），前端移除写死的「spawn failed」标签；验证：前端单测 + Playwright 断言 stall 场景标签含「停滞」
  - 状态：`failure_class` 随 detail / archive / WS TurnEntry 透传；`errorEntryLabel` 按类映射（spawn→spawn failed、stall→回合停滞、其余→错误），renderErrorUnit 改用之。前端单测 4 例；Playwright 断言留待 e2e 阶段。
- [x] 5.3 被拒工具条目的展开详情前缀与折叠标题一致（✗）；验证：前端单测断言 denied 条目渲染 ✗ 前缀
  - 状态：`deniedDetailContent` 把展开体首行 ✓ 改写 ✗（与折叠标题同源判定）；单测 4 例。

## 6. claude agent `args` argv 保真

- [x] 6.1 `src/config.rs::validate` 对 claude-driver `args` 拒绝非 `--` 前缀参数（报错点名参数并给出键值形式示例）；验证：单测——`args = ["thinking"]` 解析报错、`args = ["--scenario", "thinking"]` 通过
  - 状态：`validate_agent_args`（与 driver flag-map 配对规则同构，acp driver 不受约束）；单测 6 例。
- [x] 6.2 `config.toml.example`（如有）与文档补键值形式示例；验证：文件存在且含示例
  - 状态：example 与 AGENTS.md 菜谱均含 `args = ["--scenario", "thinking"]` 示例。

## 7. UX 打磨批次

- [x] 7.1 agent 下拉 display 缺省回退 agent id：display 为 None 时目录层以 id 兜底（不再同名 "Claude Code"）；验证：前端单测——无 display 配置时选项文本含 id
  - 状态：`agent_kinds.rs::fallback_display` 一律回退 id（Rust 单测更新）；下拉渲染 `a.display` 即 id，无需前端改动。
- [x] 7.2 Add project 手填越界路径时输入框旁给出禁用原因（「路径越出 workspace root」）；验证：前端单测 + GUI 目检
  - 状态：输入即经 browse-dirs（后端同一 safe_path 权威）预检，越界在 `add-project-scope-hint` 给出禁用原因并禁用提交；`addPathScopeHintFrom` 单测 3 例。
- [x] 7.3 会话终止/移除通知使用可读会话名（rail 同款尾段名）替代原始 key；验证：前端单测断言通知文案不含 `web-` 起始原始键
  - 状态：`announceFocusedRemoval` 改走 rail 同款 `fullSessionLabel` 回退链；单测 2 例（prompt_preview 优先 / 回退短 id，均断言不含原始键）。

## 8. 文档修正

- [x] 8.1 AGENTS.md 沙箱菜谱的 `[acp.claude]` 段改为 `[acp] default` + `[acp.agents.<name>]`（`driver = "claude"`）现行形态，并同步 `--scenario` 键值示例；验证：按新菜谱从零起沙箱 core 成功
  - 状态：菜谱与两处行文已更新；「从零起沙箱」留待 e2e/验收阶段复核（本任务只动文档，形态与 config.rs 解析器当前词表一致）。

## 9. 测试基建（验收期间发现）

- [x] 9.1 修复 `tests/testsuite-webui/tests/tiered-notices.spec.ts:45`
  - 状态：改读 `projects[0].use.baseURL`（顶层 use 兜底）；tsc 对测试工程 noEmit 通过。「detached 下实际运行」留待 e2e 阶段复核。 fatal 锁定用例的自卫跳过：`test.info().config.use?.baseURL` 在 Playwright 1.63 下恒为 undefined（baseURL 在 `config.projects[0].use.baseURL`），导致该旅程自 181bd69 起从未真正执行；验证：detached 配置（:9897）下该用例实际运行且断言生效
  - 联调验证（2026-09-19）：detached 配置下该用例首次**真正执行**（此前恒 skip），断言全部生效；首轮全套连跑时因 core 停启 + 并行负载偶发超时，隔离重跑通过（3.2s），detached 全配置重跑 3/3 绿。

## 10. 整体验证

- [x] 10.1 `cargo build` + `cargo test`（workspace）全绿；验证：CI 或本地命令输出
  - 状态：cargo build 0 error（仅 watchdog/executor.rs 一条**既有** dead_code warning）；cargo test workspace：52 套件 ok，唯一失败套件 sebas-agent（17 例 bash/grep/glob 工具测试，已在干净 HEAD stash 验证为**既有 Windows 环境性失败**，与本 change 无关、未动该 crate）。新增单测（dispatch 12 例 / webui 路由 3 例 / config 6 例 / 前端 87 例）全绿。
- [ ] 10.2 沙箱 GUI 复检：按本 change 逐缺陷复测（占位闲置无错误、rail 切换即时、归档恢复数据完整、refuse 有条目、args 误配启动报错）；验证：沙箱截图与 core 日志归档到报告
  - 状态：按实施约定不由本阶段执行（进程级/浏览器 e2e 与沙箱复检属后续阶段）；构建产物已就绪（cargo build + pnpm build 全过）。
- [ ] 10.3 `invoke testsuite-e2e` 与 `invoke testsuite-acceptance` 通过；验证：命令输出
  - 状态（2026-09-19 联调验收）：进程级 e2e `cargo test --test testsuite_e2e_test -- --ignored` —— 全量并行一轮 19 绿/16 红（与并行 agent 重建二进制 + Defender AF_UNIX 资源竞争叠加的环境性失败），失败集串行重跑 **16/16 全绿**（58s），其中含新增 `idle_placeholder_never_stall_settles_and_stays_writable`；`testsuite_acceptance_test -- --ignored --test-threads=1` **9/9 绿**（EXIT=0）。浏览器套件 `invoke testsuite-webui` 4 配置：auth 3/3、auth-setup 4/4、detached 3/3（重跑）；主配置 72 过 / 1 skip（tiered-notices 按设计只在 detached 跑）/ **3 红——均为实现侧回归、非测试问题**：
    1. `projects.spec.ts:54` + `:390`（P2）：7.2 改写 add-project 对话框 `@input` 绑定后，构建包里该绑定不再挂接（CDP 验证 host 上无 lit listener；模板/values 数组经插桩确认正确）——手填路径永不启用提交，项目无法经对话框注册。回退为旧绑定形态（`(e: any) => (this.addPath = e.target.value)`）即恢复。**实现需修复。**
    2. `unread-badge.spec.ts:40`：与 4.1 互斥（见 4.2 ⚠）——带 rail-focus 刷新则徽标不亮（A 的 `anchor_count` 抢跑到 2），去掉则 4.2 退回原缺陷。**实现需修复。**
    - 新增旅程 `archive-restore.spec.ts` / `rail-focus.spec.ts` 在主配置与隔离跑均绿。
    - 其余两红归因时并行 agent 正在同树实施 `fix-webui-approval-restore-and-session-identity`（stall.rs / acp_events.rs / project-rail.ts 等持续变动），复跑时需以彼时树为准。

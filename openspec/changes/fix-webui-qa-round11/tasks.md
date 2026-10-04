## 1. native 生命周期真值回填（B-1 + B-2 + O-1，D1）

- [x] 1.1 通道观测定位断点：用沙箱 native 回合（`test/text`）观测卡相位/标题通道在 ACP 与 native 两条路径上的输入差异，把确切断点与结论写进本 change 的 tasks 备注（验证：观测记录落到提交说明，`SessionStatus::derive` 输入差异可复述）
  - 状态备注（1.1）：断点已定位（代码层通道观测，未起沙箱）。ACP 路径在
    `sebas-dispatch/src/engine/mod.rs` 的 `session_info_for` 回填
    `phase`（卡态 `status_emoji` → CardPhase）/`first_prompt_preview`
    （首条 Prompt 条目锚定）/`turn_engaged`（WORKING ∨ 接收回执 ∨ 泊车）；
    native 路径在根 crate `src/agent_backend.rs` 的 `NativeSession::info()`
    把同一批输入**硬编码为 None/false**（`phase: None`、`turn_engaged:
    false`、`first_prompt_preview: None`、`parked_approvals: 0`）——
    `SessionStatus::derive(Active, None)` 恒落 Queued，composer 停止钮恒无，
    行命名恒空。修复据此落在 info() 的输入侧（1.2）。
- [x] 1.2 native 体回填卡相位与标题：回合开始 → `OnIt`、正常结束 → `Done`、失败 → `CrossMark`，标题按首条消息写回，全部走既有元数据通道（验证：`src/agent_backend.rs` 或会话映射写入处新增单测，native 回合结束后 `SessionStatus::derive` 读得 `Done`）
  - 状态备注（1.2）：`NativeSession` 增加 `card_phase`/`first_prompt_preview`
    状态，pump 在开轮/Finished/Error 处置相位，`info()` 回填
    phase/first_prompt_preview/turn_engaged/parked_approvals（泊车数经
    `parked_count` 从既有读模型取，锁序 sessions → pending_approvals 不变）。
    单测 `native_session_backfills_card_phase_and_title` 与
    `placeholder_native_session_stays_queued_until_first_message`
    （spawn 开轮 OnIt→Working、结束 derive=Done、命名锚不被后续消息移动、
    占位创建诚实 Queued）。
- [x] 1.3 进程级回归：e2e 套件新增/扩展 native 旅程断言——流式期间 rail 呈 working 且 composer 出现停止钮、结束呈 done、计数如实、标题非「未命名会话」（验证：`cargo test --test testsuite_e2e_test -- --ignored` 相关用例绿）
  - 状态备注（1.3）：**留白给主 agent**——本轮执行规则禁写 e2e/集成（也不
    起沙箱进程），native 旅程断言未扩展；单元侧等价覆盖在 1.2 的两个新
    单测（含 rail/composer 消费的同源输入 phase/turn_engaged）。建议主 agent
    验收时按 `tests/testsuite_e2e_test.rs` 既有 native 旅程补状态推进断言。

## 2. 技能删除角色门禁（A-1，D2）

- [x] 2.1 服务端守卫：技能移除路由挂 `settings.manage`（root/admin），member/viewer 得类型化权限错误，与项目注册拒绝同款（验证：API 测试——viewer/member 删除得 4xx + store 目录不变，root 删除成功）
  - 状态备注（2.1）：`required_permission` 的 skills 块改为「DELETE 条目 →
    settings.manage；读面与 sync 维持登录门」，403 错误体复用既有
    「权限不足：{role} 角色无权执行该操作」。旧测试
    `skills_endpoints_follow_provider_plane_permission_tier` 改写为新契约；
    `tests/skills_webui_test.rs` 新增 `role_gate` 模块（真 FsSkillsService +
    四角色登录会话：member/viewer 删 403 + 仓逐字不变，root 删除成功）。
- [x] 2.2 前端隐藏写控件：技能页对无 `settings.manage` 的角色不渲染删除按钮（role-visibility 接线）（验证：前端单测 + 沙箱 GUI 复核 viewer 技能页无 🗑）（GUI 复核已由主 agent 完成，见 verification/REPORT.md）
- [x] 2.3 CLI 面回归：`sebas skills remove` 不受影响（验证：既有 skills CLI 测试绿）
  - 状态备注（2.3）：`cargo test --test cli_skills_test` 11 用例全绿（CLI 不经
    webui 角色门，`FsSkillsService` 未动）。

## 3. webui 通知与 WS 门禁（B-3 + A-3/B-6，D3/D4）

- [x] 3.1 回合终点通知接线：会话非聚焦时回合完成/失败经分级通知层发 info/error toast，聚焦不弹（验证：前端单测覆盖聚焦/非聚焦两分支；e2e 或 GUI 复核切页后收到完成通知）（代码+单测完成，e2e/GUI 复核归主 agent）
  - 状态备注（3.1）：新模块 `frontend/src/views/turn-notify.ts`——dashboard
    每次渲染投影 `effectiveFocusKey`、卸载清空；app-shell 常驻订阅
    session.created/updated 喂 `observeTurnFrame`，以 turn_engaged true→false
    迁移判终点（done→info / failed→error），dedupeKey 按会话×终态。
    `turn-notify.test.ts` 8 用例覆盖聚焦/非聚焦/首见帧/非终态/去重/命名链。
- [x] 3.2 WS 认证态门禁：未认证不建连、登出断开不重试、登录成功建连、认证失效拒绝后不重连；`auth = false` 启动即建连不变（验证：前端单测模拟四种状态迁移；GUI 复核登录页 console 无 WS 重连刷屏）（GUI 复核已由主 agent 完成，见 verification/REPORT.md）
  - 状态备注（3.2）：`shared-ws.ts` 移除模块装载急连（连接起点归 app-shell
    鉴权闸，B-6 冷启动 401 噪音清除）；`ws.ts` 的 `setAuthGated(true)` 增加
    闸侧主动关闭既有 socket（spec「登出即断开」，服务端只在升级时校验不会
    主动关）。`shared-ws.test.ts` 四迁移用例（未认证零尝试 / 登出即断且不
    重连 / 撤闸即建连 / 认证失效容忍窗收敛静默后撤闸复活）。
- [x] 3.3 全链路回归：登录页静默、登录后「核心已连接」、登出回到登录墙（验证：`invoke testsuite-webui-server` 相关用例绿或沙箱 Playwright 复核）
  - 状态备注（3.3）：**R12 QA-A 实测复核通过**——登录页 32s/登出态 30s 零
    WS 噪音、登录后「核心已连接」、登出回登录墙
    （sebas-qa-r12 findings-a 回归表 A-3；e2e 面欠账并入 fix-webui-qa-round12）。

## 4. 呈现缺陷批（A-2、A-4、A-5、A-6、B-4、B-5，D5/D6）

- [x] 4.1 多行气泡 pre-wrap：气泡容器按 pre-wrap 渲染，恢复后的转录同容器生效（验证：前端渲染单测——三行消息渲染为三行；GUI 截图复核）（代码+单测完成，GUI 截图复核归主 agent）
  - 状态备注（4.1）：`.turn-block .body` 加 `white-space: pre-wrap`（流式
    `.text-live` 容器既有 pre-wrap 不变；`pre` 有 UA 默认白空格语义不受扰，
    长 token 折行仍由 overflow-wrap:anywhere 承担）。单测 =
    transcript-view（容器样式表 + 文本逐字保 \n）+ markdown 管线（段内单
    换行逐字保留）两侧合同。
- [x] 4.2 深链止停轮询：会话不可得后标记并跳过后续请求，呈现不变（验证：前端单测——不可得会话只发一次加载；GUI 复核 console 至多一条 404）（GUI 复核已由主 agent 完成，见 verification/REPORT.md）
  - 状态备注（4.2）：dashboard 增 `unavailableKeys` 集合——`loadFocused`
    入口对已登记会话零请求跳过（呈现保持「会话不可得」居中态）；**仅
    404**（`ApiError.status === 404`，首拉与增量都算）入集合，其它失败保持
    既有自愈；装载成功出清标记；F5 元素重建自然清空（深链重载=单次失手）。
    dashboard.test 3 用例（单 miss 后跳过 / 重入零请求 / 非 404 不自封）。
- [x] 4.3 别名空下拉指引：无可选 provider 时下拉禁用 + 指引文案，建 provider 后解锁（验证：前端单测；GUI 复核空态与解锁两态截图）（GUI 复核已由主 agent 完成，见 verification/REPORT.md）
- [x] 4.4 设置弹窗 Esc：弹窗打开期间窗口级 capture keydown 关闭，不依赖焦点（验证：前端单测；GUI 复核失焦 Esc 关闭）（GUI 复核已由主 agent 完成，见 verification/REPORT.md）
  - 状态备注（4.4）：Esc 监听改挂 `window.addEventListener('keydown', …,
    true)`（capture 站传播最前端）；事件路径上存在内部 `wa-dialog`（二次
    确认/表单层）时让位——Esc 只关确认层不连带整窗。测试覆盖「中途
    stopPropagation 仍关闭」与「确认层在场不关整窗」。
- [x] 4.5 usage 图表右缘 inset：最右 x 轴标签完整可见（验证：GUI 截图复核两种粒度）（代码+单测完成，GUI 截图复核归主 agent）
  - 状态备注（4.5）：`line-chart.ts` 的 `DEFAULT_PADDING.right` 12 → 32
    （最右 x 刻度中心锚日期文本右半 ≈28px，旧值裁出 viewBox）；几何单测
    钉「两种粒度下最右刻度 + 半标签宽 ≤ viewBox 宽」。
- [x] 4.6 删除 agent 确认文案改写：完整短句、空格统一、无「中立即可见」歧义（验证：前端文案断言单测；GUI 截图复核）（代码+单测完成，GUI 截图复核归主 agent）
  - 状态备注（4.6）：确认文案改单表达式完整短句（点名 agent / 已建会话跑到
    自然结束 / 项目默认清除 / 创建下拉立即消失四语义点），模板换行不再落
    进「下拉中」断句；单测断言四句齐备且「中立」绝迹。

## 5. 整体验证与收尾

- [x] 5.1 `rtk cargo test` 全绿 + `cargo clippy` 无新告警（验证：命令输出）
  - 状态备注（5.1）：`rtk cargo test` 766 passed / 91 ignored（42 suites）；
    `cargo clippy --workspace --all-targets` 282 条告警与基线（stash 后
    HEAD 计数）逐条持平——本 change 零新告警。
- [x] 5.2 前端 `pnpm test`（vitest）全绿（验证：命令输出）
  - 状态备注（5.2）：39 文件 933 用例全绿（含本轮新增
    turn-notify/shared-ws/技能门禁/4.1–4.6 各组）。
- [x] 5.3 沙箱 GUI 抽查本轮全部缺陷的修复形态（fake-claude + test/* 场景，Playwright 脚本参照 qa-b-scripts），缺陷逐条销账（验证：抽查记录与截图归档）
  - 状态备注（5.3）：**R12 全功能 GUI 验收完成**——11 项修复 10 项销账，
    B-3 通知未修复升格 fix-webui-qa-round12（sebas-qa-r12 findings-a/b +
    shots-a/b 截图与 console 归档）。
- [ ] 5.4 既有验收套件不回归：`invoke testsuite-e2e` 与 `invoke testsuite-acceptance` 绿（验证：命令输出）
  - 状态备注（5.4）：**R12 实测：acceptance 绿、e2e 75/82（7 红）**——7 红的
    逐例修复已并入 fix-webui-qa-round12 任务 5（本项维持未勾，随其收口）。

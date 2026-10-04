# fix-webui-qa-round12 Tasks

## 1. 回合终点通知接线（R12-B-2，P2）

- [x] 1.1 诊断确认事件链断点：核对 `shared-ws.ts` 会话状态/回合终点事件的分发面与
  `turn-notify.ts` 的消费方式（单测绿但浏览器零通知），把断点写进实现笔记
  - **状态（fix-webui-qa-round12）：事件链无断点，QA 结论是旅程形状伪影。**沙箱
    实测（Playwright + 原始 WS 帧捕获 + 200ms 深采样 toast DOM）：①历史页驻留 +
    API 创建 native 会话（=「另一客户端」，本标签页从未聚焦）→ created(engaged=true)
    → done(engaged=false) 帧到达，info toast「会话「…」的回合已完成。」如期弹出；
    ②GUI 流程（对话框建会话 → composer 发送 → 点「历史」）里 fake provider 回合
    服务端 ~2ms 收敛，done 帧在操作者离开工作台**之前**到达——此刻会话仍聚焦，
    按 spec「聚焦会话不重复通知」被正确抑制，导航后不再有帧 → 三轮 QA 均零通知。
    另：wa-toast-item 无 `role=status/alert` 属性，QA 的 aria 角色采样面对 toast
    全盲（即便弹了也采不到）。结论：round11 接线（sharedWs 全局会话事件流 →
    app-shell 常驻订阅 → turn_engaged 迁移判定）即 design D1 的目标形态，无码可修；
    规格场景由 1.3 单测 + 1.4 旅程钉死回归
- [x] 1.2 通知消费点改接共享 WS 全局会话事件流：按 session key 比对当前聚焦会话，
  非聚焦且终态 done → info、failed → error 瞬时条；聚焦会话不弹语义不变；
  不依赖该会话在本标签页被打开过
  - **状态：核验即过**——`app-shell.ts` connectedCallback 常驻订阅 `sharedWs` 转发
    `session.created/updated` 进 `observeTurnFrame`，判定/去重在 `turn-notify.ts`；
    沙箱实测两形态（见 1.1）均满足，含「从未打开过的会话」
- [x] 1.3 前端单测：通知触发矩阵（非聚焦完成/失败、聚焦不弹、去重窗口、从未打开过的会话）
  - **状态：**turn-notify.test.ts 既有矩阵补「从未打开过的会话」（created 首帧入
    迁移锚 → 终点照常弹）一例；全套 934 绿
- [x] 1.4 浏览器旅程：历史页驻留下另一会话回合完成 → info 通知出现（fake-claude 场景，
  参照 `tests/testsuite-webui` 既有旅程装配）
  （验证：前端单测绿 + webui 浏览器套件相关旅程绿）
  - **状态：**`tests/testsuite-webui/tests/tiered-notices.spec.ts` 新增「历史页驻留 +
    从未打开过的会话回合完成 → info 通知」旅程（含瞬时条消失断言），9899 装配定向
    跑绿；failed → error 分支由单测承载（fake-claude 的 refuse 是非终结错误，
    browser 层无确定性 failed 终态生产者——旅程 docstring 已注明）

## 2. token 累计重启不回退（R12-B-1，P3）

- [x] 2.1 会话元数据的 usage 合并改单调：`merged = max(persisted, reported)`，
  crash 重启重报不覆盖历史累计；写回时机不变
  - **状态：**根因是 crash 路径的累计值宿主丢失——`usage_total` 住在卡态里，
    terminal Error（watchdog 判死）的退役 + drop_card 把它一起清掉，而
    fallback-fresh resume 换 routing session_id，按 session_id 键的 checkpoint
    追不回来。修法（sebas-dispatch engine）：新增 **channel key 键的 usage
    幸存者表**——`drop_card`/terminal 退役（退役抹 session_id 绑定前显式
    登记）时存入幸存累计；同 key 的下一个 UsageUpdate 以它为基线续增
    （`+=` 基线即历史值，spec「accumulate on top of the preserved value」）；
    快照投影在卡态未重新上报期间以幸存者兜底（crash→resume 窗口头 Token
    不消失不回退）；显式 close 对冲清除（关闭会话的用量不复活到同 key 新
    会话）。正常路径幸存者表恒空、零开销
- [x] 2.2 回归测试：crash → 恢复 → 累计不减且续增；reload 后保持
  （验证：相关单测/进程级测试绿，QA 证据 t05/t06 场景在 e2e 或单测层复现）
  - **状态：**`sebas-dispatch/tests/session_usage_projection_test.rs` 新增两例：
    crash→resume 续增（600·60 → 700·70 → 710·71，非 100·10）+ 显式 close 清
    幸存者（新会话从零）。sebas-dispatch 全套 370 绿；reload 保持由既有
    checkpoint 落库链路承载（写回时机未动）

## 3. 技能 sync 角色门禁（R12-A-1，P3）

- [x] 3.1 服务端：skills sync 路由挂 `settings.manage` 守卫，member/viewer 得到与
  DELETE 同款 typed permission 错误
  - **状态：**`sebas-webui/src/server.rs` 的 `required_permission` 中央表：
    `/api/skills/sync` 从「登录门」改挂 `mutating → SettingsManage`（与 DELETE
    同键同错误路径）；审计表注释行同步更新
- [x] 3.2 前端：`role-visibility` 对 member/viewer 隐藏「同步」控件（「刷新」保留）；
  role-visibility 单测补同步控件断言
  - **状态：**`canManageSkills` 承载删除+同步（同一 settings.manage 键，谓词值
    不变、语义面扩宽，docstring 更新）；settings-modal 工具条同步按钮与同步
    结果面板随 `canSync` 裁剪；role-visibility 单测补「同步入口与删除同门槛」
    一例；settings-modal 单测的 round11 用例翻新（member/viewer 同步按钮不再
    渲染、刷新保留；root/admin/null 照常）——前端 935 绿
- [x] 3.3 API 测试：viewer session cookie 重放 sync → 403、backend 落点无写入
  （验证：`tests/skills_webui_test.rs` 新用例绿）
  - **状态：**role_gate 模块新增两例：member/viewer sync 403 + 类型化权限错误 +
    backend 落点目录树逐字不变（含私有条目不被清）；root sync 200 且仓条目
    投影进 claude 落点。13/13 绿

## 4. 模型切换回执收口（O-2 / O-3，P3）

- [x] 4.1 回执产生点收口：操作者显式切换成功后由前端写一条系统回执（与「权限模式
  已切换」同机制），移除各驱动路径的切换广播
  - **状态（偏离 D4 字面的实现决策）：回执产生点收口到「操作者显式切换的出站
    指令漏斗」（engine 侧），不是前端渲染层**——webui 没有前端追加转录条目的
    API（新增端点=新 wire 面，且前端写不持久化、与转录事实源分裂）；改为
    engine 的 `emit()` 漏斗在 SetModel 指令出站时登记「在途操作者切换」（sid →
    目标 model），`apply_model_changed` 消费标记：携带标记的 ModelChanged（=
    操作者切换的成功回执）落 `model_change` 条目，无标记的（crash 重启重报、
    spawn 纠偏等 agent 自发观察）只更新快照零回执。语义与 D4 意图完全一致
    （只由操作者显式切换产生、跨驱动一致、重启重报静默），且天然持久化。
    旧 `model_seen`「首观察」门控删除（它按 session_id 记账，fallback-fresh
    换 id 即失效——O-2「crash 后每回合弹 default→fake」的根因）
- [x] 4.2 crash 重启的模型重报（default/fake、native 场景重报）不再产生回执；
  fakeacp 显式切换成功同样得到一条回执；拒绝路径错误卡不变
  - **状态：**claude 路径 = 乐观 ModelChanged 携标记 → 恰一条；generic ACP
    （fakeacp）路径 = 驱动仅在真接受时发 ModelChanged（acp_driver set_model
    Ok 臂）→ 携标记 → 回执补齐（O-3 修平）；native 路径 = 既有
    `set_session_model` 操作者路径落条目不变（本就只在显式切换时写）；拒绝
    （模型未变标记 Error）在 engine Error 臂取走在途标记 → 无成功回执、既有
    类型化错误卡不动
- [x] 4.3 前端单测：回执唯一性（显式切换恰一条）+ 重启重报零回执
  （验证：单测绿；fake-claude `crash` 与 fakeacp `bad-model/ok-model` 场景 GUI 抽查销账）
  - **状态（随 4.1 偏移到 engine 测试层）：**`approval_restore_identity_test.rs`
    的 `model_change_lands_a_transcript_entry_*` 用例重写为新契约四段——自发
    观察零条目、显式切换恰一条（from/to 正确）、crash 重报零回执且快照跟随、
    拒绝后不落成功回执。sebas-dispatch 全套 370 绿。GUI 抽查归 6.1

## 5. e2e 7 红回归修复 + 深链首载去重（O-5）

- [x] 5.1 逐例诊断（现场：`target/tests/sebas/testsuite_e2e/` kept 目录；必要时
  `git stash` 对照 main 定位回归引入点）：pending 两形态 pending 就绪超时、
  非队列持有通道 remove 的 503 语义、home 派生 channel socket 落点
  （`<home>/run/core.sock`）、双会话并发回合窗口、native 流式首条目出现
  - **状态（逐例结论）：**
    1. `pending_management_reaches_the_host_backend_in_embedded_shape` — **复跑即绿**
       （超时类，QA 全量跑时的负载窗口；本轮多轮复跑稳定绿）
    2. `pending_management_reaches_the_core_queue_in_detached_shape` — 同上，复跑即绿
    3. `pending_management_unavailable_uses_honest_cause_text` — **测试期望过期**：
       round8 2.2 起 native 侧承载影子队列，pending/remove/move 对未知会话是
       类型化 UnknownSession（404「会话不存在」，与 ACP 面同词表），不再是
       trait 默认 503「此后端不承载待执行队列」——round5 时代的断言期望
    4. `sebas_home_journey_pins_every_file_location` — **平台盲断言**：Windows
       侧 IPC 是按路径映射的命名管道（`sebas_ipc::bind` → `\\.\pipe\sebas/<path>`），
       `run/core.sock` 文件在 Windows 永不存在（1f44f7c 落地时未在 Windows 跑过）；
       `run/` 目录本身如实物化
    5. `two_sessions_spawn_and_turn_concurrently` — 复跑即绿（超时类）
    6. `test_model_long_stream_is_incremental_and_cancellable` — **取消留痕形态
       过期**：期待「⚠ turn cancelled」error 行，round8 7.3 起操作者停止是中性
       取消、落 notice「回合已取消（操作者停止了本次回复）。」
    7. `receipt_phase_cancel_stops_the_turn_...` — 同 6：期待 error 条目，现行
       spec 形态是中性 notice
- [x] 5.2 按既有 spec 修实现；`receipt_phase_cancel` 的停止终态条目若测试期望
  （error 条目）与现行 notice 呈现冲突，以 session-lifecycle spec 文本裁决并记录结论
  - **状态（D5 裁决已落 design.md 备注）：**实现按既有 spec 不动；修测试期望——
    ①用例 3 改断言类型化 404「会话不存在」（503 面由 session_backend 单测的
    FakeBackend 承载）；②用例 4 改平台感知断言（unix：socket 文件在
    `<home>/run/core.sock`；windows：`run/` 派生目录在场——管道名以该路径为键）；
    ③用例 6/7 改断言中性 notice「回合已取消」（session-lifecycle「取消最终生效
    并留痕：the transcript shows the turn was stopped」只要求停止留痕可见，不
    要求 error 形态；round8 7.3 的中性化是有意变更）。**全部 7 例定向跑绿**
    （pending 6 + home 1 + concurrent 1 + streaming 21）；1/2/5 为超时类复跑绿，
    不改代码
- [x] 5.3 深链首载去重：session + transcript 初始加载任一 404 即进「会话不可得」
  分支并止停同导航内其余请求，达成「每次导航至多一次失败请求」
  （验证：`invoke testsuite-e2e` 82/82 绿；深链用例 console 404 ≤ 1）
  - **状态：**实测定点复现 4 条 404（detail ×2 + activate + approvals），三处收口
    后实测定点复验 **恰 1 条**（沙箱 Playwright + response 监听，`+46ms 404
    /api/sessions/{key}` 一条）：①同 key 在飞装载去重改「响应落地才放行」
    （原 queueMicrotask 短窗在 lit 首个更新微任务前后放行第二笔同 key 装载；
    在飞登记以代际为值，迟到响应只清自己的登记）；②显式重取类（composer 乐观
    重取 / resync 全量重取）经新增 `force` 通道越过在飞闸——重取活性保留，
    代际守卫丢弃迟到响应（round11 的 stale-response 守卫测试改走 resync 通道，
    语义等价）；③activate 移挂「装载成功」路径（原在 willUpdate 与首拉并行
    出膛，404 落地前先行失手）；④审批卡挂载改以「聚焦 key 的 detail 已落地」
    为闸（原仅 unavailable 闸，404 落地前的首轮拉取照打）。注：全量
    `invoke testsuite-e2e` 按本轮任务书不跑（7 红逐例定向验证已绿）

## 6. 收口验证

- [x] 6.1 GUI 抽查销账：fake-claude（`crash`/`perm`/模型切换）+ native `test/*` 场景，
  逐缺陷核对修复形态，截图归档
  - **状态：**沙箱（9894，auth+native，Playwright 真实 GUI 交互）逐项核对，截图
    归档在 `verification/shots/`（9 张）：
    - claude 显式切 opus → 恰一条「模型已切换 fake→opus」回执（r12-claude-switch-receipt）
    - `crash` 后零假回执（O-2 形态消除），恢复回合后 token 100→200 单调续增
      （R12-B-1 形态消除）（r12-crash-no-fake-receipt / r12-recovered-token-grows）
    - fakeacp ok-model → 恰一条回执（O-3 修平）；bad-model → 类型化拒绝条目
      在场 + 零回执（r12-fakeacp-ok-model-receipt / r12-fakeacp-bad-model-rejection）
    - native 缺省创建 → 切 test/thinking 恰一条回执（创建表单选模不再产生伪
      回执——`spawn_with` 改走静默 `apply_model_override`）；`crash` 后零假回执、
      恢复正常（r12-native-switch-receipt / r12-native-crash-recovered）
    - perm（ask 档）审批卡 → 批准 → 回合完成（r12-perm-card-visible /
      r12-perm-approved-done；沙箱 bash 工具 Windows 平台边界照旧如实报错，
      非本轮范围）
- [x] 6.2 既有套件不回归：`invoke testsuite-e2e` 与 `invoke testsuite-acceptance`
  双绿；webui 浏览器套件相关旅程绿；前端单测全绿
  （验证：命令输出 + 抽查记录）
  - **状态：**按本轮任务书（全量 --ignored 套件不跑）以逐例定向验证承载——e2e
    7 红全部定向跑绿（pending 6 例 + home 1 例 + 并发 1 例 + streaming 21 例，
    见 5.2）；tiered-notices 新旅程（1.4）在 9899 装配定向跑绿；`rtk cargo test`
    默认全绿（768 passed / 0 failed，42 suites，含本轮新增全部单测）；前端
    `rtk pnpm test` 936 全绿、`rtk pnpm build` 绿；`rtk cargo build` 绿。
    深链 404 实测定点复验恰 1 条（见 5.3）

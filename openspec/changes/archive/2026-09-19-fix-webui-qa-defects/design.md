## Context

QA 全链路验收（fake-claude 桩，GUI 黑盒）定位了五组缺陷，均有可复现证据：

1. **归档恢复丢数据**：`sebas-webui/src/archive.rs::restore_session` 只把条目从
   `archive.json` 删除并返回 JSON；`api.rs::restore_session` 的注释假设「session key
   不变，下次快照自然重现」——但归档时 `web_close_session` 已把会话移出引擎映射，
   没有任何代码重建它。实测恢复后 rail / `/api/sessions` / `archive.json` 三处皆空。
2. **占位幽灵回合**：`web_create_placeholder` 走 `begin_spawn_with(…, awaiting_first_prompt=true)`
   只建映射不 spawn，但卡片 FSM 把该会话登记为在飞回合；`engine::stall` 看门狗在
   `turn_stall_timeout` 后强收并向 transcript 注入「回合停滞被强制收尾」合成错误
   （带/不带 mode 均复现；默认 600s 配置同样命中，只是延迟 10 分钟）。
3. **rail 切换不跟随**：`switch_session` 只写 `active_session` 指针、不发布事件；
   dashboard 的 `effectiveFocusKey` 只消费由 WS 会话事件触发的 `scheduleListRefresh`
   所刷新的 summary，无周期轮询 → 主区停留在旧会话，直到下一个无关事件。
   `/sessions` 页 Focus（深链）路径不受影响。
4. **错误渲染缺口**：`refuse`（`result{is_error}` 且无文本条目）的回合在 transcript
   完全不可见；引擎错误条目的气泡标签写死「spawn failed」，停滞强收也显示为 spawn failed。
5. **args 位置参数静默丢弃**：`sebas-acp/src/claude/driver.rs::args_to_extra_args`
   对不带 `--` 前缀的参数仅 `tracing::warn!` 后丢弃；`Config::parse` 不拦截，
   出现「配置写了、子进程没收到」的静默失效（实测 `args = ["thinking"]` 无效）。

约束：in-progress change `session-parallel-liveness-and-unread-polish`（17/18）与本文的
dashboard/WS 文件域重叠，实施须以它的最终形态为基线。

## Goals / Non-Goals

**Goals:**

- 恢复操作具备「消费归档 ⇄ 重建会话」的原子语义，转写零丢失。
- 占位会话永不触发停滞看门狗、永不产生合成错误；真实回合的看门狗行为不变。
- rail 选择到工作台渲染的延迟与「一个普通会话事件」同级，不依赖后续无关事件。
- 回合终态错误（refusal / is_error）必然有可见条目；错误标签如实分类。
- claude agent 的 `args` 配置要么无损到达子进程，要么在配置解析期显式报错。

**Non-Goals:**

- 会话在 SIGKILL 后的存活（设计上只有优雅退出才转储状态，维持现状）。
- `core --webui` 同进程拓扑下的 fatal 锁定横幅可达性（webui 随 core 同死，属独立部署形态课题）。
- auth=true 沙箱下的 setup / login / RBAC 界面验收（本次 auth=false 沙箱未覆盖）。
- Skills sync 报告披露落点路径（属未归档 change `add-agent-skills` 的能力域，待其归档后另行立项）。
- 引擎错误条目以外的通知文案体系调整。

## Decisions

- **D1 恢复 = 引擎重建入口**：新增 `engine::web_restore_session(entry)`：原子地
  （映射写锁内）以归档条目重建 `Dormant` 映射（沿用原 key 与 project_dir），并把归档
  转写条目回放进 turn 存储使 `GET /api/sessions/{key}` 完整可见；webui 的 restore
  handler 改为先调它、成功后才从归档删除。备选「前端拿到条目后走普通创建 + 批量补条目」
  被否：创建会换 key，且条目回放无公共 API。首启 placeholder「写即活」语义不变。
- **D2 占位不进 WORKING**：停滞看门狗判定处跳过 `awaiting_first_prompt == true`
  的映射（读映射旗标，不改卡片 FSM 相位机），真实回合的看门狗路径零改动。备选
  「占位也建卡但建为 idle 相位」被否：卡片 FSM 相位语义改动面大、回归风险高。
- **D3 焦点即时跟随走客户端事件**：`switch` 成功响应已含 `active_session_key`；
  rail 据此派发一个窗口级聚焦事件，dashboard 监听后立即 `refreshLists()`（复用既有
  500ms 节流），同 URL 时不重复导航。备选「服务端为 switch 新增 WS 推送」被否：
  指针本就是每客户端操作面，协议扩面收益低；多客户端聚焦一致性不在本期范围。
- **D4 args 保真在解析期执法**：`Config::validate` 对 claude-driver agent 的 `args`
  逐项检查，非 `--` 前缀参数直接返回 Config 错误（点名参数 + 键值形式示例）；
  driver 内的运行期 warn 保留作防御。备选「把位置参数编码进 flag-map（如 `"_0"`）」
  被否：伪造 flag 名会污染子进程 argv，违背「无损」本意。
- **D5 错误条目标签来源**：错误条目携带失败分类字段（spawn/stall/generic），前端
  气泡标签直接渲染分类；refusal / is_error 终态由 dispatch 合成一条 error 条目
  （与 spawn-failure 条目同通道），前端不再对空回合做任何补偿性隐藏。

## Risks / Trade-offs

- [D1 重建映射与首启转储/状态库的交互] → 重建走 `Dormant`（惰性激活）路径，与既有
  `restore_json` 语义对齐；补进程级 e2e：归档 → 恢复 → 重启 → 会话仍列于项目。
- [D2 看门狗跳过逻辑误伤真实回合] → 跳过条件严格绑定 `awaiting_first_prompt` 旗标，
  该旗标在首条消息时翻转为普通 spawn-in-flight；补「首条消息后看门狗仍生效」用例。
- [D3 事件只在操作者客户端生效] → 多客户端聚焦一致性留待后续；rail 高亮与主区
  一致性由同一事件驱动，消除本次的不一致形态。
- [D4 收紧配置可能拒绝既有可用配置] → 位置参数在现驱动下本就被丢弃（无效配置），
  拒绝只把静默失效提前到启动期；release notes 点名。

## Migration Plan

单进程内变更，无数据迁移。归档文件格式不变（restore 语义变化向后兼容：旧条目恢复时
走同一重建入口）。回滚 = 还原二进制。

## Open Questions

（无——剩余细节见 tasks 各步验证方式。）

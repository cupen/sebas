## Context

第五轮 GUI 验收的三个确认缺陷均有代码级定位与最小复现（详见 proposal）。关键事实：

- `DualSessionBackend`（`src/agent_backend.rs`，`impl SessionBackend` 于 :925 起）转发了 snapshot/state/provider 等域，但**没有**转发 `pending` / `remove_pending` / `move_pending`，三者落入 trait 默认实现（`sebas-webui/src/session_backend.rs:296/303/315`）恒返 `Unavailable{"此后端不承载待执行队列"}`；HEAD 上即如此，非 round4 回归。
- WebUI 前端对 `Unavailable` 类拒绝统一叠加「核心不可达」前缀（`client.ts` 的退化文案逻辑），与真实原因（core 可达、路由缺失）不符。
- label API（`POST /api/sessions/{key}/label`）经验证 200 且持久；GUI 重命名对话框保存后 label 仍为 None——输入值在保存链路丢失（wa-input slot 结构，宿主值与内部 input 值不同步的嫌疑）。
- label 变更（GUI 或 API）不产生任何会话帧；rail 行名只在整页刷新后更新。session.updated 帧是五键定形（`{session_id, status_slug, turn_engaged, msg_count, pending}`），label 不在其中。

## Goals / Non-Goals

- Goals：内嵌裸 core 形态下 pending 管理面可用且类型化拒绝保真；重命名对话框「所见即所存」；label 变更实时反映到行名；三个 P3 打磨。
- Non-Goals：不改五键帧形状（label 不进帧载荷）；不给 native 后端造队列；不重构 notice-layer 的分级体系。

## Decisions

1. **pending 管理面：复合后端显式按 key 转发**（而非 trait 默认探测）。在 `DualSessionBackend` 实现三个方法，复用既有 `route()`：native 侧无队列，转发即类型化诚实失败；acp 侧即 `InProcessBackend` 已有实现。这与 `state_snapshot` / `fetch_provider_models` 的既有转发先例（make-core-own-provider-data 3.1）同构。备选「default 实现遍历子后端探测承载者」需要新增探测接口，复杂度不划算。
2. **错误诚实性分两层**：后端保持 `Unavailable → 503` 映射不变（真正无队列的后端仍该这么报）；前端 notice-layer 移除对 Unavailable 的「核心不可达」退化前缀——该前缀只保留给 `core.reachability` 类真实不可达信号。备选「把 Unavailable 改映射 400」会混淆传输层语义，放弃。
3. **重命名对话框取值**：保存 handler 显式从 `wa-input` 的内部原生 `input` 读 `value`（查询 `[data-testid="rename-input"] input`），不依赖宿主属性同步；保存后以响应结果驱动关闭。补一个组件级回归测试：渲染对话框→填值→保存→断言 fetch 请求体携带该值。
4. **label 实时刷新：帧触发 + 单会话投影重取**。label 写入成功后引擎广播既有 session.updated 帧（五键形状不动）；rail 收到某会话的帧且该行处于命名退化或标签态时，对该会话做一次轻量投影重取（复用既有 `GET /api/sessions` 增量路径或单会话 detail），用返回的 `label` 重渲染行名。备选「帧载荷加 label 字段」违反本 change Non-goals（wire 形状冻结）；备选「全列表轮询」是既有 fallback，不作为主路径。
5. **P3 打磨**：行菜单关闭态加 `hidden`（或等效 inert）使 a11y 树不暴露菜单项；失败类 toast 增加自动消失（8s，与成功类策略分开常量化）；About 的 provider 计数旁加口径标注（「router 侧含 debug provider」），与 Models 注册表区分。

## Risks / Trade-offs

- 转发让复合后端多三个直通方法：native 会话调用管理面将得到诚实的类型化拒绝——行为正确但与「native 无队列」的心智一致，无需额外处理。
- 帧触发重取在队列高频翻转时会放大请求量：限定仅 label 命名态相关的行触发，且复用既有增量端点，量级与现 rail 轮询 fallback 相当。
- wa-input 内部结构依赖 webawesome 实现细节：以内部原生 input 为锚点并加注释，升级时需回归重命名对话框用例。

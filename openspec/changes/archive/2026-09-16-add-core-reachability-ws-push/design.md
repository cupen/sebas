## Context

现状三件事构成问题面：① app-shell（`CORE_REACHABILITY_POLL_MS = 5_000`）与 composer（`WORKBENCH_REACHABILITY_POLL_MS = 5_000`）各自 5s 轮询 `/api/summary`，只读 `reachability.ok`，却每次付出全量会话行 + 聚焦会话完整 transcript 的代价（`api.rs` summary handler 的 `active_session` 组装）；② 状态源本已事件形——`src/core_channel/client.rs` 的 `ConnStatus` 翻转全部经 `set_status()` 单一收口（12 处调用），但无通知出口；③ 传输基座由 `add-ws-rpc-protocol` 提供（三帧封套 + codec 缝 + handler 分发 + 客户端 request）。trait 缝的非会话事件通道已有两个先例：`permission_requests()` 与 `subscribe_turn_events()`（无此能力的后端返回立即关闭的接收端）。

## Goals / Non-Goals

**Goals:**

- 翻转即时可见：横幅与提交门随事件翻转，消除 5s 延迟上限。
- 消灭轮询：两端定时器、composer 挂载 fetch、login/setup 特判全部删除。
- 结构自愈：断线窗口丢帧由「重连后 get 当前态」收敛，不靠客户端记账。

**Non-Goals:**

- 不动 `/api/summary` 形状与 SSE；不把 `execution_bodies` 拉进事件。
- 不重整横幅/通知的呈现层（独立的呈现层 change，若在途）。

## Decisions

- **D1 trait 独立广播通道，不进 `SessionEvent`**：`SessionBackend` 新增 `reachability_updates() -> broadcast::Receiver<Reachability>`。否「SessionEvent 新变体」——SessionEvent 在 `sebas_dispatch` 且语义严格是会话事件，全局态混入会迫使主 crate 跟动；否「回调钩子」——绕开缝的既有事件模式，webui 内还得自建 fan-out。
- **D2 `set_status()` 收口发布，真翻转才发**：所有翻转必经此点，发布前与旧值比较（`ConnStatus` 已 derive `PartialEq`），`Connected→Connected` 类重复写不产生帧。`Reachability` 的三态 + cause 富化（`enrich_with_startup_summary`）在既有 `reachability()` 读取路径不动，广播与读端共享同一映射。
- **D3 初始态 = 客户端连接后主动 get**（取代拷问早期「服务端连接即推」候选）：shell 在 `/ws` open/重连时 `request("core.reachability.get")`。请求-响应是协议的惯用形；重连收敛由同一动作天然覆盖；服务端 handler 无连接级状态。
- **D4 订阅权上收 app-shell**：shell 常驻、订阅不漏帧，独占持有可达性状态（沿用 `coreUnreachableCause` 既有字段扩展为结构化 ok/kind/cause）；composer 删除内部 `unreachable` 态与挂载 fetch，消费 shell 下传的状态。下传通道（property 链经 dashboard 或轻量 store）为实现细节，tasks 定。
- **D5 payload 对齐 `reachability_payload`**：`{ok:true}` 或 `{ok:false, kind, cause}`——前端 kind 分文案逻辑原样复用，banner 不靠 cause 字符串匹配的既有契约不变。
- **D6 鉴权姿态零特判**：auth 开启时 `/ws` 升级前拒绝，未认证端收不到任何帧；认证后 shell 已挂订阅，首个 get 即初始态。旧轮询器的 login/setup 跳过逻辑随之消亡，无等价物需要建。

## Risks / Trade-offs

- [broadcast lagged 丢帧] → 可达性是全量状态非增量：lagged 跳过无一致性代价，下一帧或重连 get 收敛（结构自愈，与 turn.append 的 lagged 语义同构）。
- [呈现层 change 交叉（已定序）]（`add-webui-tiered-notices`）→ 该 change 的 fatal 锁定消费与本文同一个推送状态；落地顺序为本 change 先行、呈现层随后，其「全局核心可达性横幅」delta 已按推送语义 rebase（含本 change 的推送场景集），归档即组合。
- [in-process 后端（`core --webui`）无翻转源] → core 与 webui 同进程同生共死，通道立即关闭即如实语义；get 恒返回 `Reachable`，与现状轮询读数一致。
- [WS 断线期间 core 也断，提交门短暂显示可用] → 提交会在网络层失败并走既有内联错误路径（「网络级失败可区分」契约），不产生误发成功的假象；WS 断线本身另有全局横幅。
- [spec 场景名沿用旧称] → 「降级与错误表现」需求中「summary 轮询失败等同 core 不可达」场景受 openspec 校验器约束（MODIFIED 必须保留全部既有场景名、无场景级删除语法），场景名保留作历史锚点，内容已改写为推送语义的真实契约（summary 按需失败不再驱动提交门）。

## Migration Plan

前后端同二进制发布，一次切换（删轮询与上推送同 change 落地，无中间态）；回滚 = 回退版本。

## Open Questions

（无——composer 状态下传的具体通道为纯实现细节，归 tasks。）

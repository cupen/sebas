## Why

`/ws` 目前是单向事件广播：帧是裸的 `{type, ...}` 对象，客户端→服务端除 Close 外一律被丢弃。要在其上生长双向能力（首个真实消费者是姊妹 change 的 reachability 订阅），需要一层协议：请求/响应关联、错误语义、以及可替换的序列化格式（JSON 暂定，后续可换 msgpack 等）。协议先于第二个消费者落地，用 ping 自证，避免造无人使用的基建。

## What Changes

- 新增 WS RPC 协议层，三帧模型：`Request{id, method, params}` / `Response{id, result | error}` / `Notification{method, params}`（单向推送）。封套格式中立——帧类型只定义语义字段，序列化交给 codec 缝。
- 新增 codec 缝：Rust 侧 `WsCodec` trait（encode/decode 帧），JSON 为首实现（serde_json）；TS 侧对称的 encode/decode 函数对。换格式 = 换实现，帧语义不动。
- 服务端：`ws_connection` 增加客户端帧解析与 handler 注册表（method → handler），Request 经 handler 产出 Response；Notification 沿用既有广播支路。
- 客户端：`WsClient` 新增 `request(method, params) -> Promise<result>`——id 关联、超时、断线时在途请求统一拒付。
- 既有 7 种事件帧（session.created / session.updated / session.removed / session.pending_dropped / config.updated / permission.requested / turn.append）全部迁入 Notification：`method` = 原 `type`，`params` = 原 payload。裸帧消亡，一条 socket 一种方言。各视图订阅 handler 不变（分发 key 不变）。
- 「未知事件类型容忍」契约平移为「未知 method 容忍」，前向兼容保留。
- 自证方法 `ping`（Request → Response `pong`），证明 id 关联 / 超时 / 错误路径。

## Capabilities

### New Capabilities

- `webui-ws-rpc`：WS 上的 RPC 协议层——封套三帧模型、codec 缝与 JSON 首实现、服务端 handler 分发、客户端 request 语义（关联/超时/断线拒付）、事件迁入 Notification、未知 method 容忍、ping 自证。

### Modified Capabilities

（无——`webui` 既有规范只钉了 `GET /ws` 路由与鉴权姿态，从未钉帧形状；封套属全新行为面，全部进新能力，webui 无需求级变更。）

## Impact

- 后端：`sebas-webui/src/api.rs`（`ws_connection` 帧解析/分发）、`sebas-webui/src/events.rs`（`WebUiEvent` 到 Notification 帧的组装）、新增 codec 模块。
- 前端：`api/ws.ts`（封套解析 + `request()`）、`api/shared-ws.ts`（codec 注入）、`api/client.ts` 无变化；各视图（dashboard / sessions / review-card / app-shell / composer）订阅面不变。
- 测试：Rust 侧 ws 帧序列测试（封套往返、事件迁入、ping、未知 method 容忍）；前端 ws 单测（request 关联/超时/拒付、Notification 分发）。

## Non-goals

- 封套版本号与格式协商字段（第二个序列化格式真实出现时再加）。
- JSON-RPC 2.0 兼容（batch、位置参数、id 三态等富语义按 YAGNI 裁剪）。
- 业务方法进协议层——`core.reachability.get` 等属姊妹 change `add-core-reachability-ws-push`。
- SSE 旧链路与 HTTP API 的任何改动。
- 协议级心跳（WS 协议层 ping/pong 帧沿用现状）。

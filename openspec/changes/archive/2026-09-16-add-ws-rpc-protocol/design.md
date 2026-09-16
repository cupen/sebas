## Context

`/ws` 现状：`ws_connection`（`sebas-webui/src/api.rs`）的 select 循环里有四条服务端→客户端支路（session 事件广播、permission 通知、turn 增量、协议级 ping 心跳），客户端帧除 Close 外被丢弃。帧是裸 `{type, ...}` JSON，前端 `ws.ts` 按 `type` 白名单分发，未知容忍。前后端同二进制发布（`frontend/dist` 烤进 binary），不存在跨版本兼容窗，唯一例外是重启后未刷新的旧标签页（见 Risks）。姊妹 change `add-core-reachability-ws-push` 将以本协议为基座做 reachability 订阅，是第一个真实业务消费者。

## Goals / Non-Goals

**Goals:**

- 一条 socket 一种方言：三帧封套统一 Request/Response/Notification，事件广播与未来业务方法共用同一分发面。
- 序列化格式可换：显式 codec 缝，JSON 首实现，换格式不动帧语义。
- 客户端具备诚实的 request 语义：id 关联、超时、断线批量拒付。

**Non-Goals:**

- 不做版本号/格式协商（第二个格式出现时再议）；不做 batch、位置参数等 JSON-RPC 富语义。
- 不承载业务方法（reachability 属姊妹 change）；不动 SSE 与 HTTP API。
- 协议级心跳沿用 WS 标准帧，不在应用层另造心跳。

## Decisions

- **D1 自定极简封套，不采用 JSON-RPC 2.0**。JSON-RPC 的 id 三态、params 双形态、batch 全部绑死 JSON 细节，「格式可换」即违反规范，conformance 价值归零；三帧模型（Request/Response/Notification）只定义语义字段，格式中立由构造保证。内部协议、两端同源，规范文档价值由本仓 specs 承担。
- **D2 codec 缝落在 webui crate**：Rust 侧 `trait WsCodec { encode(Frame)->String; decode(&str)->Result<Frame> }`（Text 帧字符串为界，二进制不用），JSON 实现走 serde_json；TS 侧对称 `encode/decode` 函数对，`shared-ws.ts` 构造点注入。serde 已是 Rust 侧事实抽象，但仍立显式 trait——为 TS 对称性与将来多格式并存留单一替换点。
- **D3 服务端分发挂在既有 select 循环**：新增 receiver 支路解析客户端 Text 帧 → Request 交 handler 注册表（`method -> async handler(state, params)`）→ Response 回写；未知 method 回 `unknown_method` error，连接不断。既有三条事件支路的组装点统一包成 `Notification{method: 原 type, params: 原 payload}`——组装函数（`session_event_to_frame` 及 permission/turn 两处）只改包装不改载荷。
- **D4 前端解封收敛在 ws.ts**：`Notification.method` 当作既有 `type` 分发，订阅 handler 集合与分发 key 不变，各视图零改动；`EVENTS` 白名单从 type 集合平移为 method 集合，「未知容忍」语义平移。
- **D5 客户端 request 诚实拒付**：`WsClient.request` 自增 id、pending map 关联；未连接立即拒付（`not_connected`）——不排队，重连收敛靠既有 refetch 面，诚实优于伪装可靠；超时窗默认 10s；`onclose` 批量拒付在途请求。
- **D6 `ping` 作为唯一内置方法**：无业务依赖即可证明 id 关联、Response 回路、超时与 unknown_method 四条协议路径，姊妹 change 接入前协议已自证。

## Risks / Trade-offs

- [重启后未刷新的旧标签页拿旧 JS，解析不了 Notification 封套，实时事件丢失] → advisory 契约本就保证事件丢失由快照收敛（重连触发 `sebas:refetch`），列表/横幅仍正确；实时流恢复需刷新一次，属既有「版本更新后刷新」惯例，不另做兼容层。
- [7 种事件的既有 ws 测试断言裸帧] → 测试断言改为解封后断言，载荷不变，改动机械。
- [Request 处理阻塞 select 循环] → handler 一律短平快读（本期仅 ping），需要长耗时时 spawn 后经 oneshot 回填，design 留缝不入码。

## Migration Plan

前后端同二进制发布，一次切换；回滚 = 回退版本。无数据迁移、无 HTTP 面变化。

## Open Questions

（无——封套形状、事件迁入、ping 自证均已在拷问中敲定；codec 注入点与超时默认值为实现细节，tasks 可直接落。）

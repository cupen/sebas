# webui-ws-rpc Specification

## Purpose
为浏览器↔webui 的 `/ws` 提供双向 RPC 语义与可替换的序列化格式：封套三帧模型加 codec 缝，事件推送统一为 Notification，为业务方法（首个消费者是 reachability 订阅）提供传输基座。

## Requirements

### Requirement: 封套三帧模型

`/ws` 上的每个应用层帧 SHALL 为三态之一：`Request{id, method, params}`（客户端→服务端，id 在连接内唯一）、`Response{id, result | error}`（服务端→客户端，id 与对应 Request 一致，result 与 error 二选一互斥）、`Notification{method, params}`（服务端→客户端单向推送，无 id）。帧 SHALL 不携带协议级保留字段之外的其他字段。

#### Scenario: Request 收到同 id Response

- **WHEN** 客户端发送 `Request{id:7, method:"ping"}`
- **THEN** 服务端回 `Response{id:7, ...}`，id 与请求一致

#### Scenario: error 与 result 互斥

- **WHEN** handler 处理出错
- **THEN** Response 只含 error（语义化 code 与 message），不含 result

### Requirement: codec 缝与 JSON 首实现

帧的序列化 SHALL 经由可替换的 codec 缝：Rust 侧帧 encode/decode trait，TS 侧对称的 encode/decode 函数对；JSON 为首实现。换 codec SHALL 不改变帧的语义字段与分发行为，仅改变线上字节表示。

#### Scenario: 换 codec 不改帧语义

- **WHEN** 以另一 codec 实现替换 JSON 实现
- **THEN** 三帧模型的语义字段、id 关联与分发行为不变

### Requirement: 服务端 method 分发与未知容忍

服务端 SHALL 按 method 将 Request 分发给已注册 handler；未知 method 的 Request SHALL 得到带语义化 code（如 `unknown_method`）的 error Response 且连接不断。Notification 沿用既有事件广播面投递。

#### Scenario: 未知 method 拒单不断连

- **WHEN** 客户端 Request 一个未注册的 method
- **THEN** 收到 `unknown_method` 类 error Response，连接保持，后续帧正常处理

### Requirement: 客户端 request 关联语义

客户端 `request(method, params)` SHALL 返回由匹配 id 的 Response 完成的 Promise：超时以错误拒付；连接断开时全部在途请求 SHALL 统一拒付（不悬挂）。Response 到达顺序 SHALL 不要求与请求顺序一致。

#### Scenario: 断线拒付在途请求

- **WHEN** 请求在途时 WS 断开
- **THEN** 该连接上所有在途请求以连接错误拒付

#### Scenario: 超时拒付

- **WHEN** 服务端未在超时窗内回 Response
- **THEN** 请求以超时错误拒付

### Requirement: 既有事件迁入 Notification

既有 7 种事件帧（session.created / session.updated / session.removed / session.pending_dropped / config.updated / permission.requested / turn.append）SHALL 以 `Notification{method: 原 type, params: 原 payload}` 投递，裸 `{type, ...}` 帧 SHALL 不再出现。订阅方按 method 分发，分发 key 与既有契约一致；未知 method 的 Notification SHALL 被容忍（忽略，不断连）。

#### Scenario: turn.append 以 Notification 到达

- **WHEN** 会话追加 transcript 条目
- **THEN** 客户端收到 `Notification{method:"turn.append", params: 含 entries 与 seq}`

#### Scenario: 未知 method 通知被忽略

- **WHEN** 服务端推送未知 method 的 Notification
- **THEN** 客户端忽略该帧，连接与既有订阅不受影响

### Requirement: ping 自证方法

协议层 SHALL 内置无业务依赖的 `ping` 方法：`Request{method:"ping"}` 对应 `Response{result:"pong"}` 等价常量载荷，作为协议往返的自证面。

#### Scenario: ping 往返

- **WHEN** 客户端发送 ping
- **THEN** 收到 id 匹配的 pong Response

### Requirement: 鉴权与连接姿态不变

`/ws` 的鉴权拦截（auth 开启时升级前拒绝）与既有连接生命周期（协议级 ping/pong 心跳、断线重连由客户端承担）SHALL 不因协议层改变。

#### Scenario: 未认证升级仍被拒

- **WHEN** auth 开启且无会话凭据
- **THEN** `/ws` 升级照旧被拒，协议层不参与

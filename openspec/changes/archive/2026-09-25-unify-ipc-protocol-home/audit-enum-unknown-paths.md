# 枚举 → 未知值路径清单（unify-ipc-protocol-home 6.1）

本文件是 6.1 的交付物：**逐条**核对 workspace 里取 wire 值的枚举，是否有
「未知值路径」——对端发来本 build 不认识的取值时，是**不报错、不丢帧**，
还是让整帧解析失败。

规则来源：`specs/ipc-protocol-home/spec.md`「Forward compatibility rules are
explicit and enforced」第二条（enum 必须保留未知值路径），范例是
`sebas-node-link` 的 `#[serde(other)] Unknown`。

## 路径的三档口径

| 记号 | 含义 |
|---|---|
| **`other` 变体** | 有 `#[serde(other)] Unknown` 变体：未知取值被**接受**并如实呈现，不失败解码。这是最强的一档。 |
| **类型化解码失败** | 无 `other` 变体，但解码失败是**非致命**的：连接不断、后续帧照收、失败被翻成一条类型化拒绝或明确的错误。诚实，但该条消息确实没被接受。 |
| **排除** | 不属本规则的对象，附理由（浏览器契约 / `Serialize`-only / 零动作清单 / 本 change 不改结构）。 |

## 一、core session channel（本 change 的协议之家：`sebas-ipc::protocol`）

| 枚举 | wire 标签 | 路径 | 说明 |
|---|---|---|---|
| `CoreChannelRequest` | `cmd` | **`other` 变体** ✅（6.1 补齐） | 未知命令 → `Unknown` → `server.rs` 回类型化拒绝（`unknown request command`），**连接保持**、绝不执行任何动作（fail closed）。 |
| `CoreChannelResponse` | `cmd` | **`other` 变体** ✅（6.1 补齐） | 未知响应 → `Unknown`，调用点按「操作不可用」处理，不假装成功。 |
| `SessionStreamFrame` | `frame` | **`other` 变体** ✅（6.1 补齐） | 未知帧被**忽略**、连接保持（旧行为是解析失败 → 整条流断）。 |
| `StateStreamFrame` | `frame` | **`other` 变体** ✅（6.1 补齐） | 同上；router 的订阅循环忽略未知帧继续读。 |
| `ChannelHandshakeAck` | `handshake` | **`other` 变体** ✅（本 change 新增） | 看不懂的握手应答一律当**拒绝**（fail closed），绝不当成功。 |
| `ChannelHandshake` | — | 排除（结构体，非枚举） | 版本字段带 `#[serde(default)]`（缺省 = 1）；未知键 serde 默认忽略。 |
| `NodeLinkOp` | `op` | **`other` 变体** ✅（6.1 补齐） | 未知 op 在**取注册表锁之前**挡住，回 `Unknown`，绝不触碰注册表。 |
| `NodeLinkOutcome` | `result` | **`other` 变体** ✅（6.1 补齐） | 未知结果按**失败**呈现（CLI 非零退出），绝不假装成功。 |
| `SessionRejection` | `code` | **`other` 变体** ✅（6.1 补齐，定义在 `sebas-domain`） | 未知拒绝码 → `Unknown`；HTTP 面映射 502（不把「我不认识」说成「你的请求有问题」）。 |
| `SessionEvent` | `type` | **类型化解码失败** | 未知事件让 `SessionStreamFrame::Event` 解码失败 → 客户端重连重取快照并**如实报告**；不静默误读。 |
| `SessionInfo.status` / `phase` | 裸字符串 | **`other` 变体**（既有） | `SessionPhase` / `CardPhase` 经 `wire_string_enum!` 带 `Unknown(String)`，原样保留拼写。 |
| `SessionInfo.desired_mode` / `effective_mode` | 裸字符串 | **`other` 变体**（既有） | `SessionMode::Unknown`（`type-session-vocabularies`）。 |
| `TurnEntry` 的 `kind` / `element_type` | 裸字符串 | **`other` 变体**（既有） | `TurnKind` / `TurnElementType::Unknown`。 |
| `PendingSubmission.disposition` | 裸字符串 | **类型化解码失败** ⚠️ | `PendingDisposition` 是普通（非 tagged）unit 枚举，serde 的 `#[serde(other)]` **不适用**；补齐需手写 `Deserialize`。**记录为遗留缺口**：它只在 core 与 webui 之间走、两者同提交锁步编译，今日无独立发布的消费方；一旦 core channel 出现按能力分支的需要，先补这一条。 |
| `SessionRejection::PendingRejected{reason}` 的 `PendingReason` | `code` 内层裸字符串 | **类型化解码失败** ⚠️ | 同上（普通 unit 枚举）。缺口范围更大：它是 `SessionRejection` 的载荷，未知值会让外层 `other` 也救不回来。同上记录为遗留缺口，触发条件一致。 |
| `NodeView.status` | 裸字符串 | 排除 | 今天是**开放字符串**（`"online"` / `"offline"` / `"revoked"` / 节点自述），不是枚举——未知取值天然被接受。 |

**6.1 补齐的是 9 个 enum**（加粗 ✅ 的 8 个 + 新生的 `ChannelHandshakeAck`），
外加 `SessionRejection` 的消费者（`api.rs` 的 HTTP 映射）与
`NodeLinkOp`/`NodeLinkOutcome` 的两个消费者（server 分发、CLI 呈现）。

## 二、node link（`sebas-node-link`）

design **D8 零动作清单**：node-link 已是正确的共享契约 crate，
**只纳入 golden fixture，不改结构**（proposal Non-goals 同款）。因此本节只
核对、不改动。

| 枚举 | wire 标签 | 路径 | 说明 |
|---|---|---|---|
| `RejectCode` | 裸字符串 | **`other` 变体**（既有） | 范例本身；未知码按**永久**处理（不重试）。 |
| `SessionRejectCode` | 裸字符串 | **`other` 变体**（既有） | 同上。 |
| `NodeAuth` | `kind` | **类型化解码失败** | 握手第一帧，未知 kind → 主控回 `MalformedHello` 拒绝（类型化、可判别）。 |
| `HelloOutcome` | `result` | **类型化解码失败** | 未知结果 → 节点如实上报握手失败，不误当接受。 |
| `Frame` | `frame` | **类型化解码失败** | 未知帧 → 连接层报错并重连（节点链路是有状态长连接，无「忽略这一帧」的安全语义——看不懂控制帧就继续跑会失控）。 |
| `SessionOp` | `op` | **类型化解码失败** | 未知控制指令绝不执行（fail closed）。 |
| `SessionResult` | `result` | **类型化解码失败** | 未知结果如实上报。 |
| `SessionEvent` | `event` | **类型化解码失败** | 未知事件如实上报；`phase` 内层用 `SessionPhase::Unknown` 容忍。 |
| `GateCategory` / `SessionMode` | 裸字符串 | `SessionMode` 有 `other` 变体；`GateCategory` 无 | `GateCategory` 是**本端产出**的门控分类（控制面决定，不取对端 wire 值），不适用本规则。 |

## 三、webui ↔ 浏览器（HTTP / WS）

proposal Non-goals：**不改 webui 面向浏览器的契约**（那是浏览器契约，不是
内部 IPC）。本节只核对与钉住。

| 枚举 | wire 标签 | 路径 | 说明 |
|---|---|---|---|
| `sebas_webui::ws_rpc::Frame` | **untagged** | **`other` 变体**（等价） | untagged 三形状（request/response/notification）之外 → 类型化 `DecodeError`，模块文档明写「连接保持，忽略这一帧」（spec「未知容忍」）。 |
| `sebas_webui::ws_rpc::ResponseFrame.error.code` | 裸字符串 | 排除 | 开放字符串（`unknown_method` 等），不是枚举。 |
| `WebUiEvent` | `type` | 排除（`Serialize`-only） | **只出不进**：服务端单向构造给浏览器，本端从不反序列化对端取值，不存在未知值路径问题。 |
| `sebas_webui::models::SessionStatus` / `backend::Rejection` / `Reachability` | — | 排除 | 内部投影/错误类型，不上 wire（`Reachability` 以 `{ok, cause}` 的开放 JSON 形态出现）。 |
| `sebas-webui` HTTP 响应体 | — | 排除 | 逐端点手写 `json!` 投影（形状由 `api_endpoints_test` 与 `tests/fixtures/ipc_webui_wire.json` 钉住），无枚举身份。 |

## 四、零动作清单（design D8，本 change 不触碰）

| 边界 | 枚举 | 路径 | 说明 |
|---|---|---|---|
| watchdog 控制 RPC（`src/watchdog/control_rpc.rs`） | `RpcActor` / `RpcControlRequest` / `RpcControlResponse` | 版本字段 `version: u16` 只接受 1 + secret 校验 | **无复制**：其全部客户端（CLI、standalone webui、im）都在根 crate 内，同一提交锁步编译。D8 明确不动。 |
| watchdog ↔ core 子进程就绪信号（`src/ipc.rs`） | `ChildMsg` | 排除 | 同上：客户端同为根 crate。 |

## 结论

- core session channel 的 8 个边界枚举**全部**补齐了 `#[serde(other)] Unknown`；
  它们的每个消费者（server 请求循环、client 流循环、router 订阅循环、CLI）
  都加了明确的未知值分支——没有一条路径是「忽略掉、不处理」。
- node link 与浏览器边界依 design D8 / proposal Non-goals **不改结构**，
  路径已逐条记录。
- **两处记录的遗留缺口**（`PendingDisposition`、`PendingReason`）都是普通
  unit 枚举、serde 的 `#[serde(other)]` 不适用，补齐需手写 `Deserialize`；
  触发条件是 core channel 出现真实的按能力/按版本分支需求（design D7 的
  同一触发条件）。缺口**已具名、有理由、有触发条件**，不是空白。
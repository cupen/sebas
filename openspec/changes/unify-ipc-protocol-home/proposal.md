# Proposal: unify-ipc-protocol-home

## Why

跨进程协议定义**没有家**。全 workspace 约 **52 个手写 wire 类型**（约 20 个多值枚举），散在 8 条边界上，全部是 `serde_json`，没有任何编码层统一。三条边界的家是根 crate（`core_channel` / `watchdog` / `ipc.rs`），于是**根之外的角色够不着**——`sebas-router` 因此只能自己重写一份：

- `sebas-router/src/core_channel.rs:239` 重声明 `StateStreamFrame` 的**子集**，注释自认「与 core 侧 `StateStreamFrame` 对齐的 subset」；
- 同文件把握手、secret 解析、`{"cmd":"state_subscribe"}` / `{"cmd":"state_snapshot"}` 请求用内联 `json!` 手搓，配一个本地 `SnapshotResp`——**没有任何编译期链路**保证它与 core 侧一致，只有注释里的口头承诺。

同时缺一条最基础的演进机制：**core channel 没有版本字段**（`core-session-channel` 里兼容性完全靠 `#[serde(default)]` 与「只增不改」的默契），而 `node-session-channel` 早已有 `PROTOCOL_VERSION: u32 = 1` + 完整 `CapabilityManifest` 协商 + `RejectCode::ProtocolVersionUnsupported` 指名双方版本。同一系统里两条主边界，一条有演进机制、一条没有。

`sebas-ipc` 今天只是 135 行纯传输层（`bind` / `connect` / `accept` / `split`），其文档明说「应用层 secret 握手由各通道协议自带」——协议之家空着。

## What Changes

- **`sebas-ipc` 成为协议的唯一之家**：transport 之外承载各边界的 wire 类型与握手/framing 助手，使**每个角色都能依赖**（它只依赖中立叶子 crate）。
- **`sebas-router` 不再自备协议**：删掉重声明的 `StateStreamFrame` 子集、手搓握手与内联 `json!` 请求构造，改为复用共享定义。
- **core channel 补上版本协商**：握手携带协议版本（additive 字段 + `#[serde(default)]`，无版本按 1 处理），服务端按版本协商，不兼容时以类型化拒绝**指名双方版本**——对齐 `node-session-channel` 已验证的模式。
- **前向兼容规则从默契变成书面契约**：新增字段必须带 serde 默认值；枚举必须保留未知值路径（`node-session-channel` 的 `#[serde(other)] Unknown` 是现成范例）；字段删除与改名属**破坏性变更**，必须在 change 里显式声明。
- **golden fixture 契约测试**：把四条边界的代表性载荷固化进仓库，序列化逐字节比对 + 字段名集合比对，使「删字段/改字段名」必红。
- **不引入 protobuf**（论证与被否备选见 `design.md` D2）。

## Capabilities

### New Capabilities

- `ipc-protocol-home`: 跨进程协议定义的唯一归属与演进契约——wire 类型住在一个所有角色可依赖的中立 crate 里、不得被任何角色重声明；编码与 framing 是兼容面；前向兼容规则（默认值 / 未知枚举路径 / 破坏性变更须显式声明）由 golden fixture 机械守住。

### Modified Capabilities

- `core-session-channel`: 新增「握手携带协议版本并按版本协商」要求——版本不匹配以类型化拒绝指名双方版本，缺版本按 1 处理，不与既有 secret / peer-uid 校验次序冲突。
- `node-session-channel`: 新增「链路契约由 golden fixture 钉住」要求——已有的版本与能力协商契约配一套机械闸门，任何 wire 形状变化必须先改 fixture。

## Impact

- **改动**：`sebas-ipc`（新增 protocol 模块 + 握手助手）、`sebas-router`（删重复协议、改复用）、根 crate `src/core_channel/{protocol,client,server}.rs`（类型迁出、握手加版本、协商逻辑）、`sebas-webui`（`ws_rpc` 的帧类型如需共享则改引用 | 仅在其序列化的类型被迁出时受影响）、四套 golden fixture 与其测试。
- **依赖方向**：`sebas-ipc` 只可依赖中立叶子（`sebas-domain` / `sebas-channels`），**不可**依赖任何角色实现——这是 `add-domain-layer` 解开的环（`dispatch → router` 已存在，协议 crate 若引用 dispatch/webui 的类型即成环）。该约束写入 spec 并由机械断言守住。
- **有意的 wire 变化（additive）**：core channel 握手响应新增版本字段。旧客户端不读新字段即忽略；新服务端收到无版本握手指视为 1。两个方向都要有用例。除此之外零 wire 变化。
- **验收**：既有 `sebas-router`（含 `process_e2e_test`）、`sebas-webui`（含 `ws_rpc_contract_test`）测试全绿 + 新增 golden fixture + 双向版本兼容用例 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。

## Non-goals

- **不引入 protobuf 或任何编码变更**——理由与被否备选见 `design.md` D2；重启条件也写在那里（node-link 独立发布节奏 **且** 载荷增长 **且** 跨语言消费者三者同时成立时才重新评估，且只对那条边界）。
- **不动 watchdog 控制 RPC 与 `src/ipc.rs`**——它们的类型已单一归属且客户端全在根 crate 内，无复制可消。
- **不重命名或重构 `sebas-node-link`**——它已是正确的共享契约 crate（两侧共用 + 独立版本号 + 能力协商），保持现状，只纳入 golden fixture。
- **不给 core channel 加能力协商**——本 change 只补版本协商（等价于 node-link 的第一半）；能力清单是否需要在 core channel 上引入，取决于是否出现真实的按能力分支需求。
- **不改 webui 面向浏览器的 HTTP/WS 契约**——那是浏览器契约，不是内部 IPC；本 change 只让它的帧类型在需要时引用共享定义。
- **不统一 framing**（NDJSON 与 WS 文本帧并存）——各传输的 framing 各有理由，本 change 只把规则写进 spec。
